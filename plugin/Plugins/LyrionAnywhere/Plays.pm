package Plugins::LyrionAnywhere::Plays;

# `anywhere play_record`: counts a play that happened offline on a Lyrion Anywhere device
# (SPEC D6). Only the gateway sends it, after checking the play (POST /v1/plays in
# docs/PROTOCOL.md): LMS has no command of its own to raise a track's play count.
#
#   anywhere play_record track_id:<id> played_at:<unix> key:<8-64 chars> ts:<unix> sig:<hex>
#
# LMS's JSON-RPC and CLI are open to the LAN, so the command is signed: sig is the hex
# HMAC-SHA256 of "lyrion-anywhere play_record v1", track_id, played_at, key and ts (one per
# line), keyed with the admin secret the gateway keeps in its data directory (the same
# secret this plugin uses to call the gateway's admin API). Anything unsigned, wrongly
# signed or signed more than two minutes ago is refused before anything is looked up.
#
# The play is recorded as LMS records one when a track starts on a player
# (Slim::Player::Song::open): the track's persistent row gets playcount + 1, and lastplayed
# becomes the time the play started (if that is later than what is there). Only local
# library files count. Keys recently recorded are remembered, so a resend (the gateway gave
# up waiting for the answer) is answered `duplicate`; the gateway also remembers keys.
#
# Answers: ok:1, or error:<duplicate|not_found|not_library|busy|unsupported|failed|refused…>.

use strict;
use warnings;

use Digest::SHA ();

use Slim::Utils::Log;

use Plugins::LyrionAnywhere::Admin;

my $log = logger('plugin.lyrionanywhere');

use constant SIG_CONTEXT => 'lyrion-anywhere play_record v1';
use constant MAX_SKEW    => 120;                 # a signature is good for this long
use constant MAX_AGE     => 91 * 86400;          # oldest play (the gateway allows 90 days)
use constant MAX_FUTURE  => 300;                 # clock skew
use constant REMEMBER    => 10_000;              # recorded keys kept in memory

my %seen;     # key => 1
my @order;    # keys, oldest first

sub _now { time() }    # the clock (tests replace it)

# --- checks ---------------------------------------------------------------------------

sub _plain { my $v = shift; return defined $v && !ref $v }
sub valid_track_id { my $v = shift; return _plain($v) && $v =~ /^[1-9][0-9]{0,11}\z/ ? 1 : 0 }
sub valid_time     { my $v = shift; return _plain($v) && $v =~ /^[1-9][0-9]{0,10}\z/ ? 1 : 0 }
sub valid_key      { my $v = shift; return _plain($v) && $v =~ /^[A-Za-z0-9_-]{8,64}\z/ ? 1 : 0 }
sub valid_sig      { my $v = shift; return _plain($v) && $v =~ /^[0-9a-f]{64}\z/ ? 1 : 0 }

sub signature {
	my ($secret, $track_id, $played_at, $key, $ts) = @_;
	return Digest::SHA::hmac_sha256_hex(join("\n", SIG_CONTEXT, $track_id, $played_at, $key, $ts), $secret);
}

# Constant-time comparison of two strings of the same length.
sub _same {
	my ($x, $y) = @_;
	return 0 unless length($x) == length($y);
	my $diff = 0;
	$diff |= ord(substr $x, $_, 1) ^ ord(substr $y, $_, 1) for 0 .. length($x) - 1;
	return $diff == 0 ? 1 : 0;
}

# Is this a well-formed, correctly signed, current request? undef if so, else the error.
sub check {
	my ($p, $secret, $now) = @_;
	return 'refused: bad track_id'  unless valid_track_id($p->{track_id});
	return 'refused: bad played_at' unless valid_time($p->{played_at});
	return 'refused: bad key'       unless valid_key($p->{key});
	return 'refused: bad ts'        unless valid_time($p->{ts});
	return 'refused: bad sig'       unless valid_sig($p->{sig});
	return 'unavailable'            unless defined $secret && length $secret;
	return 'refused: stale'         if abs($now - $p->{ts}) > MAX_SKEW;
	my $want = signature($secret, @{$p}{qw(track_id played_at key ts)});
	return 'refused: bad signature' unless _same($p->{sig}, $want);
	return 'refused: bad time'      if $p->{played_at} > $now + MAX_FUTURE || $p->{played_at} < $now - MAX_AGE;
	return undef;
}

sub _remember {
	my $key = shift;
	return if $seen{$key}++;
	push @order, $key;
	delete $seen{ shift @order } while @order > REMEMBER;
}

sub seen { return $seen{ $_[0] // '' } ? 1 : 0 }
sub forget_all { %seen = (); @order = () }    # tests

# --- recording ------------------------------------------------------------------------

# Count one play of library track $id that started at $played_at. 'ok' or an error code.
sub record_play {
	my ($id, $played_at) = @_;
	my $stats = main->can('STATISTICS');
	return 'unsupported' if $stats && !$stats->();
	return 'unsupported' unless Slim::Schema->can('find');
	# Like LMS itself: the persistent database is locked while the scanner runs.
	return 'busy' if Slim::Music::Import->can('stillScanning') && Slim::Music::Import->stillScanning();
	return 'busy' if Slim::Schema->can('hasLibrary') && !Slim::Schema->hasLibrary();

	my $track = eval { Slim::Schema->find('Track', $id) };
	return 'failed' if $@;
	return 'not_found' unless $track;
	my $url = eval { $track->url } // '';
	return 'not_library' if eval { $track->remote } || $url !~ m{^file://};

	my $persistent = eval { $track->retrievePersistent };
	return 'not_library' unless $persistent;
	my $ok = eval {
		$persistent->set(playcount => ($persistent->playcount || 0) + 1);
		my $last = $persistent->lastplayed || 0;
		$persistent->set(lastplayed => $played_at) if $played_at > $last;
		$persistent->update;
		1;
	};
	if (!$ok) {
		$log->warn('could not record an offline play: ' . ($@ // 'unknown error'));
		return 'failed';
	}
	return 'ok';
}

# --- the command ----------------------------------------------------------------------

sub command {
	my $request = shift;
	my %p = map { $_ => scalar $request->getParam($_) } qw(track_id played_at key ts sig);
	my $answer = sub {
		my ($k, $v) = @_;
		$request->addResult($k, $v);
		$request->setStatusDone;
	};
	if (my $e = check(\%p, Plugins::LyrionAnywhere::Admin::secret(), _now())) {
		$log->warn("refused `anywhere play_record`: $e") if $e =~ /^refused/;
		return $answer->(error => $e);
	}
	return $answer->(error => 'duplicate') if $seen{ $p{key} };
	my $r = record_play($p{track_id}, $p{played_at});
	return $answer->(error => $r) unless $r eq 'ok';
	_remember($p{key});
	$log->info("recorded an offline play of track $p{track_id}");
	return $answer->(ok => 1);
}

1;
