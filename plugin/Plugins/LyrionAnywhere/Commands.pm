package Plugins::LyrionAnywhere::Commands;

# LMS JSON-RPC/CLI commands used by the settings page: `anywhere status`, `anywhere restart`
# and one command per admin operation (`anywhere users`, `anywhere user_create
# username:… password:… role:…`, …).
#
# They run inside LMS, so they are as reachable as LMS's own JSON-RPC (port 9000) and CLI
# (9090), which are unauthenticated on the LAN unless LMS password protection is on. LMS's
# /jsonrpc.js also accepts cross-origin "text/plain" POSTs, so any web page a LAN user
# opens could send commands blind. Every admin command therefore needs `csrf:<token>`: a
# random value made when LMS starts, given only to the settings page (which other web sites
# cannot read). That includes the read-only ones, since they return remote users' addresses,
# browsers and the audit log (pentest 2026-10). `anywhere status` is the one exception, and
# only in part: without the token it says whether the gateway runs and the gist of the last
# error; the binary and data paths, pid, listen address and the gateway's stderr need the
# token (review 2026-10-15). The gateway never forwards `anywhere` commands from the
# internet (it is reserved in its allow-list).
#
# The `anywhere tunnel_*` commands (the built-in Cloudflare Tunnel) need the token too,
# `tunnel_status` and `tunnel_log` included: they return the public hostname and log lines.
#
# `anywhere play_record` (Plays.pm) is the one command the gateway itself sends: it is
# signed with the admin secret instead of carrying the settings page's token.

use strict;
use warnings;

use Digest::SHA ();
use Time::HiRes ();

use Slim::Control::Request;
use Slim::Utils::Log;

use Plugins::LyrionAnywhere::Admin;
use Plugins::LyrionAnywhere::Gateway;
use Plugins::LyrionAnywhere::Plays;

my $log = logger('plugin.lyrionanywhere');

# Arguments passed on to the admin API. `csrf` is deliberately absent: it is checked here
# and never sent to the gateway. `password`, `token` (a Cloudflare tunnel token) and
# `api_token` (a Cloudflare API token) are secrets: nothing here logs the arguments or puts
# them in a result, and Admin::redact() masks them out of any error message.
my @ARG_NAMES = qw(username password role disabled player id limit days commands enabled formats name
	token api_token hostname replace_dns);

# --- CSRF token -----------------------------------------------------------------------

my $token;

sub _random_bytes {
	my $n = shift;
	if (open my $fh, '<:raw', '/dev/urandom') {
		my $got = read($fh, my $buf, $n);
		close $fh;
		return $buf if $got && $got == $n;
	}
	if (eval { require Crypt::URandom; 1 }) {
		my $buf = eval { Crypt::URandom::urandom($n) };
		return $buf if defined $buf && length $buf == $n;
	}
	# Last resort (no /dev/urandom, e.g. Windows without Crypt::URandom): hash what
	# varies between processes and calls.
	my $seed = join ':', $$, time(), Time::HiRes::time(), rand(), {} + 0, $0, $^T, values %ENV;
	return substr(Digest::SHA::sha256($seed . rand()), 0, $n);
}

# The token for this LMS process: 128 random bits as hex. Kept in memory only (never in a
# pref, which LMS's `pref` command could read).
sub csrf_token {
	$token //= unpack 'H*', _random_bytes(16);
	return $token;
}

# Constant-time comparison, so the token cannot be guessed a character at a time.
sub token_ok {
	my $given = shift;
	return 0 unless defined $given && !ref $given;
	my $want = csrf_token();
	return 0 unless length($given) == length($want);
	my $diff = 0;
	$diff |= ord(substr $given, $_, 1) ^ ord(substr $want, $_, 1) for 0 .. length($want) - 1;
	return $diff == 0 ? 1 : 0;
}

# Commands that change state (everything the admin API does with POST, and restart).
sub mutating {
	my $op = shift // return 1;
	return 1 if $op eq 'restart';
	return Plugins::LyrionAnywhere::Admin::is_mutating($op);
}

sub _refuse {
	my ($request, $op) = @_;
	$log->warn("refused `anywhere $op` without the settings page's token");
	$request->addResult('error', 'refused: use the Lyrion Anywhere settings page (reload it if LMS restarted)');
	$request->setStatusDone;
}

sub register {
	# [needsClient, isQuery, hasTags, function]
	Slim::Control::Request::addDispatch(['anywhere', 'status'],  [0, 1, 1, \&status]);
	Slim::Control::Request::addDispatch(['anywhere', 'restart'], [0, 0, 1, \&restart]);
	Slim::Control::Request::addDispatch(['anywhere', 'play_record'], [0, 0, 1, \&Plugins::LyrionAnywhere::Plays::command]);
	for my $op (Plugins::LyrionAnywhere::Admin::operations()) {
		Slim::Control::Request::addDispatch(['anywhere', $op], [0, 1, 1, \&admin]);
	}
}

# What `anywhere status` tells a caller without the token.
my @PUBLIC_STATUS = qw(enabled running binary_ok uptime);

# The gist of an error for a caller without the token: its first line, up to the first
# ": " (after which come paths, addresses or the gateway's stderr), without control
# characters, at most 120 characters.
sub public_error {
	my $e = shift // return '';
	$e = (split /\r?\n/, $e, 2)[0] // '';
	$e =~ s/: .*//s;
	$e =~ s/[\x00-\x1f\x7f]//g;
	return substr $e, 0, 120;
}

sub status {
	my $request = shift;
	my $state = Plugins::LyrionAnywhere::Gateway->state;
	if (token_ok($request->getParam('csrf'))) {
		$request->addResult($_, $state->{$_}) for sort keys %$state;
	} else {
		$request->addResult($_, $state->{$_}) for @PUBLIC_STATUS;
		$request->addResult('error', public_error($state->{error}));
	}
	$request->setStatusDone;
}

sub restart {
	my $request = shift;
	return _refuse($request, 'restart') unless token_ok($request->getParam('csrf'));
	# A gateway that is turned off is stopped, never started: only Enabled (and Save) turns
	# remote access on.
	unless (Plugins::LyrionAnywhere::Gateway->restart) {
		$request->addResult('started', 0);
		$request->addResult('message', 'Remote access is turned off, so the gateway was not started. Tick Enabled and save the settings to turn it on.');
		$request->setStatusDone;
		return;
	}
	$request->addResult('started', 1);
	$request->setStatusDone;
}

sub admin {
	my $request = shift;
	my $op = $request->getRequest(1) // '';
	# Reads too: devices, diagnostics, audit and users hold personal data.
	return _refuse($request, $op) unless token_ok($request->getParam('csrf'));
	my %args;
	for my $k (@ARG_NAMES) {
		my $v = $request->getParam($k);
		$args{$k} = $v if defined $v;
	}
	$args{limit} //= 100 if $op eq 'audit';

	# Validate before anything is sent; a bad request never reaches the gateway.
	eval { Plugins::LyrionAnywhere::Admin::request_for($op, \%args) };
	if (my $e = $@) {
		chomp $e;
		$request->addResult('error', Plugins::LyrionAnywhere::Admin::redact($e, \%args));
		$request->setStatusDone;
		return;
	}

	$request->setStatusProcessing;
	Plugins::LyrionAnywhere::Admin::call(
		$op, \%args,
		sub {
			my $data = shift;
			$request->addResult('ok', 1);
			$request->addResult('data', $data);
			$request->setStatusDone;
		},
		sub {
			my $e = shift;
			$request->addResult('error', $e);
			$request->setStatusDone;
		},
	);
}

1;
