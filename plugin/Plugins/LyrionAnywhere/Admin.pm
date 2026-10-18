package Plugins::LyrionAnywhere::Admin;

# Client for the gateway's loopback admin API (docs/PROTOCOL.md, "Admin API").
#
# request_for() maps an operation and its arguments to (method, path, body) and validates
# every argument first, so nothing malformed (path segments, header-breaking characters)
# is ever sent. call() sends it asynchronously with the admin secret the gateway wrote to
# its data directory.

use strict;
use warnings;

use File::Spec::Functions qw(catfile);
use JSON::XS::VersionOneAndTwo;
use MIME::Base64 ();

use Slim::Networking::SimpleAsyncHTTP;
use Slim::Utils::Log;

use Plugins::LyrionAnywhere::Gateway;

my $log = logger('plugin.lyrionanywhere');

use constant TIMEOUT => 15;

sub valid_username { my $u = shift // ''; return $u =~ /^[a-z0-9._-]{1,64}\z/ }
sub valid_mac      { my $m = shift // ''; return $m =~ /^(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\z/i }
sub valid_id       { my $i = shift // ''; return $i =~ /^\d{1,18}\z/ }

# A device name for the gateway, which checks it fully (1-64 characters, cleaned): here, at
# most 64 characters once trimmed, and no control characters.
sub valid_device_name {
	my $n = shift;
	return 0 if !defined $n || ref $n;
	(my $t = $n) =~ s/\A\s+|\s+\z//g;
	return length($t) >= 1 && length($t) <= 64 && $n !~ /[\x00-\x1f\x7f]/;
}

# --- Cloudflare Tunnel inputs (docs/plans/cloudflare-tunnel.md, "Inputs") ----------------
# The gateway checks them all again; these keep anything malformed from being sent at all.

# A public hostname for the tunnel, already lower-cased: 4-253 characters, two or more DNS
# labels of a-z 0-9 - (1-63 each, no leading or trailing -), no trailing dot, port or path,
# not an IP address, and not under Cloudflare's own tunnel domains.
sub valid_hostname {
	my $h = shift;
	return 0 if !defined $h || ref $h;
	$h = lc $h;
	return 0 unless length($h) >= 4 && length($h) <= 253;
	return 0 unless $h =~ /\A[a-z0-9-]+(?:\.[a-z0-9-]+)+\z/;
	for my $label (split /\./, $h, -1) {
		return 0 unless $label =~ /\A[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\z/;
	}
	# An IPv4 address (or anything ending in a numeric "top-level domain", which none is).
	return 0 if $h =~ /\.[0-9]+\z/;
	return 0 if $h =~ /(?:\A|\.)(?:trycloudflare\.com|cfargotunnel\.com)\z/;
	return 1;
}

# A tunnel token as Cloudflare's dashboard shows it: 64-4096 characters of base64 (standard
# or URL alphabet, padding optional) holding a JSON object with "a", "t" and "s". Only the
# shape is checked here; the gateway checks the fields themselves.
sub valid_tunnel_token {
	my $t = shift;
	return 0 if !defined $t || ref $t;
	return 0 unless length($t) >= 64 && length($t) <= 4096;
	return 0 unless $t =~ /\A[A-Za-z0-9+\/_-]+={0,2}\z/;
	(my $std = $t) =~ tr{-_}{+/};
	$std =~ s/=+\z//;
	return 0 if length($std) % 4 == 1;
	$std .= '=' x ((4 - length($std) % 4) % 4);
	my $json = MIME::Base64::decode_base64($std);
	my $data = eval { decode_json($json) };
	return 0 unless ref $data eq 'HASH';
	for my $k (qw(a t s)) {
		return 0 unless defined $data->{$k} && !ref $data->{$k} && length $data->{$k};
	}
	return 1;
}

# A Cloudflare API token: 20-128 characters of A-Z a-z 0-9 _ -.
sub valid_api_token { my $t = shift; return defined $t && !ref $t && $t =~ /\A[A-Za-z0-9_-]{20,128}\z/ }

sub valid_password {
	my $p = shift;
	return defined $p && length($p) >= 10 && length($p) <= 1024 && $p !~ /[\x00-\x1f\x7f]/;
}

# Operation => [method, path template, required args, body builder, optional args]
my %OPS = (
	users        => ['GET',  '/admin/v1/users'],
	user_create  => ['POST', '/admin/v1/users', [qw(username password role)],
	                 sub { { username => $_[0]{username}, password => $_[0]{password}, role => $_[0]{role} } }],
	user_password=> ['POST', '/admin/v1/users/{username}/password', [qw(username password)],
	                 sub { { password => $_[0]{password} } }],
	user_disable => ['POST', '/admin/v1/users/{username}/disabled', [qw(username disabled)],
	                 sub { { disabled => ($_[0]{disabled} ? \1 : \0) } }],
	user_delete  => ['POST', '/admin/v1/users/{username}/delete', [qw(username)]],
	totp_reset   => ['POST', '/admin/v1/users/{username}/totp/reset', [qw(username)]],
	# Downloads on (1) or off (0); `formats` (optional) limits them, e.g. "original,opus".
	user_downloads => ['POST', '/admin/v1/users/{username}/downloads', [qw(username enabled)],
	                 sub {
	                     my %b = (enabled => ($_[0]{enabled} ? \1 : \0));
	                     $b{formats} = [split /,/, $_[0]{formats}] if defined $_[0]{formats};
	                     \%b;
	                 }, [qw(formats)]],
	pairing      => ['POST', '/admin/v1/users/{username}/pairing', [qw(username)]],
	grants       => ['GET',  '/admin/v1/users/{username}/players', [qw(username)]],
	grant        => ['POST', '/admin/v1/users/{username}/players', [qw(username player)],
	                 sub { { player => lc $_[0]{player} } }],
	ungrant      => ['POST', '/admin/v1/users/{username}/players/{player}/remove', [qw(username player)]],
	devices      => ['GET',  '/admin/v1/devices'],
	revoke       => ['POST', '/admin/v1/devices/{id}/revoke', [qw(id)]],
	# The gateway cleans the name and renames the device's LMS player too.
	device_rename => ['POST', '/admin/v1/devices/{id}/name', [qw(id name)],
	                 sub { { name => $_[0]{name} } }],
	diagnostics  => ['GET',  '/admin/v1/devices/{id}/diagnostics', [qw(id)]],
	audit        => ['GET',  '/admin/v1/audit?limit={limit}', [qw(limit)]],
	settings     => ['GET',  '/admin/v1/settings'],
	session_days => ['POST', '/admin/v1/settings', [qw(days)],
	                 sub { { session_days => 0 + $_[0]{days} } }],
	plugins      => ['GET',  '/admin/v1/plugins'],
	plugins_admin_only => ['POST', '/admin/v1/plugins', [qw(commands)],
	                 sub { { admin_only => [grep { length } split /,/, $_[0]{commands} // ''] } }],
	# Cloudflare Tunnel (docs/plans/cloudflare-tunnel.md, "Build contract"). The status and
	# log are reads but still need the page's token (they hold the public hostname and log).
	tunnel_status  => ['GET',  '/admin/v1/tunnel'],
	tunnel_log     => ['GET',  '/admin/v1/tunnel/log'],
	tunnel_install => ['POST', '/admin/v1/tunnel/install'],
	tunnel_update  => ['POST', '/admin/v1/tunnel/update'],
	tunnel_register => ['POST', '/admin/v1/tunnel/register', [qw(token hostname)],
	                 sub { { token => $_[0]{token}, hostname => lc $_[0]{hostname} } }],
	# replace_dns is optional: left out, an existing DNS record is reported, not replaced.
	tunnel_create  => ['POST', '/admin/v1/tunnel/create', [qw(api_token hostname)],
	                 sub { { api_token => $_[0]{api_token}, hostname => lc $_[0]{hostname},
	                         replace_dns => ($_[0]{replace_dns} ? \1 : \0) } }, [qw(replace_dns)]],
	tunnel_start   => ['POST', '/admin/v1/tunnel/start'],
	tunnel_stop    => ['POST', '/admin/v1/tunnel/stop'],
	tunnel_quick   => ['POST', '/admin/v1/tunnel/quick'],
	tunnel_remove  => ['POST', '/admin/v1/tunnel/remove'],
);

# Arguments that are secrets: never logged, never put in a result, and masked out of any
# message (even the gateway's own) before it is shown or logged.
my @SECRET_ARGS = qw(password token api_token);

# $text with every secret argument's value replaced, so nothing passes one on by accident.
sub redact {
	my ($text, $args) = @_;
	return $text unless defined $text && ref $args eq 'HASH';
	for my $k (@SECRET_ARGS) {
		my $v = $args->{$k};
		next unless defined $v && !ref $v && length $v >= 4;
		$text =~ s/\Q$v\E/[hidden]/g;
	}
	return $text;
}

sub operations { return sort keys %OPS }

# Does the operation change anything? (Every POST does; unknown operations count as yes.)
sub is_mutating {
	my $op = shift // '';
	my $spec = $OPS{$op} or return 1;
	return $spec->[0] eq 'GET' ? 0 : 1;
}

my %CHECK = (
	username => \&valid_username,
	password => \&valid_password,
	role     => sub { ($_[0] // '') =~ /^(?:listener|admin)\z/ },
	disabled => sub { ($_[0] // '') =~ /^[01]\z/ },
	enabled  => sub { ($_[0] // '') =~ /^[01]\z/ },
	# Download formats: one to three of original, mp3, opus, comma-separated, no repeats.
	formats  => sub {
		my $v = $_[0] // '';
		return 0 if ref $v || $v !~ /^(?:original|mp3|opus)(?:,(?:original|mp3|opus)){0,2}\z/;
		my %seen;
		return !grep { $seen{$_}++ } split /,/, $v;
	},
	player   => \&valid_mac,
	id       => \&valid_id,
	name     => \&valid_device_name,
	limit    => sub { ($_[0] // '') =~ /^\d{1,4}\z/ && $_[0] >= 1 && $_[0] <= 1000 },
	days     => sub { ($_[0] // '') =~ /^\d{1,3}\z/ && $_[0] >= 1 && $_[0] <= 365 },
	# Comma-separated plugin command words (none = no plugin is admin-only).
	commands => sub {
		my $v = $_[0] // '';
		return 0 if ref $v || $v !~ /^[A-Za-z0-9_,-]{0,13000}\z/;
		my @c = grep { length } split /,/, $v;
		return @c <= 200 && !grep { length > 64 } @c;
	},
	hostname    => \&valid_hostname,
	token       => \&valid_tunnel_token,
	api_token   => \&valid_api_token,
	replace_dns => sub { my $v = $_[0]; defined $v && !ref $v && $v =~ /\A[01]\z/ },
);

my %MESSAGE = (
	username => 'usernames are 1-64 characters: a-z 0-9 . _ -',
	password => 'passwords need at least 10 characters (no line breaks)',
	role     => 'role must be listener or admin',
	disabled => 'disabled must be 0 or 1',
	enabled  => 'enabled must be 0 or 1',
	formats  => 'formats must be one or more of original, mp3, opus (comma-separated)',
	player   => 'player must be a MAC address like aa:bb:cc:dd:ee:ff',
	id       => 'bad device id',
	name     => 'device names are 1-64 characters (no line breaks)',
	limit    => 'limit must be 1-1000',
	days     => 'the sign-in lifetime must be 1-365 days',
	commands => 'plugin commands are comma-separated words of A-Z a-z 0-9 _ - (at most 200)',
	hostname => 'the hostname must be a full domain name such as music.example.com: letters, digits and hyphens, with no port, path, trailing dot or IP address, and not under trycloudflare.com or cfargotunnel.com',
	token    => 'that is not a tunnel token: paste the long token Cloudflare shows for the tunnel (Networks > Tunnels > your tunnel), all of it and nothing else',
	api_token => 'that is not a Cloudflare API token: it should be 20-128 characters of letters, digits, _ and -',
	replace_dns => 'replace_dns must be 0 or 1',
);

# Returns ($method, $path, $body_or_undef) or dies with a user-readable message.
sub request_for {
	my ($op, $args) = @_;
	my $spec = $OPS{$op // ''} or die "unknown operation\n";
	my ($method, $path, $required, $body, $optional) = @$spec;
	for my $k (@{ $required || [] }) {
		die "$MESSAGE{$k}\n" unless $CHECK{$k}->($args->{$k});
	}
	for my $k (@{ $optional || [] }) {
		next unless defined $args->{$k};
		die "$MESSAGE{$k}\n" unless $CHECK{$k}->($args->{$k});
	}
	$path =~ s/\{(\w+)\}/lc $args->{$1}/ge;
	return ($method, $path, $body ? $body->($args) : undef);
}

sub secret {
	my $file = catfile(Plugins::LyrionAnywhere::Gateway::data_dir(), 'admin.secret');
	open my $fh, '<', $file or return undef;
	my $s = <$fh>;
	close $fh;
	return undef unless defined $s;
	$s =~ s/\s+//g;
	return ($s =~ /^[0-9a-f]{32,128}\z/) ? $s : undef;
}

# Send an operation. $ok->($data) on success; $err->($message) on failure.
sub call {
	my ($op, $args, $ok, $err) = @_;
	my ($method, $path, $body) = eval { request_for($op, $args) };
	if (!$method) {
		my $m = $@ || 'bad request';
		chomp $m;
		return $err->($m);
	}
	my $secret = secret() or return $err->('the gateway has not started yet (no admin secret)');
	# Only to the gateway this plugin started, on the loopback port it started it with (never
	# a port from a later pref change): the secret stays with the gateway.
	my $target = Plugins::LyrionAnywhere::Gateway::admin_target()
		or return $err->('the gateway is not running');
	# 127.0.0.1 only. With LMS's `webproxy` pref set, Slim::Networking::Async::HTTP sends
	# plain HTTP through the proxy unless the host matches localhost or 127.0.0.1, so any
	# other loopback address ([::1], 127.0.0.2) would carry the secret off the host.
	return $err->('the admin API must be on 127.0.0.1; not sending the admin secret')
		unless $target =~ /\A127\.0\.0\.1:\d{1,5}\z/;
	my $url = 'http://' . $target . $path;
	my $http = Slim::Networking::SimpleAsyncHTTP->new(
		sub {
			my $res = shift;
			my $content = $res->content;
			my $data = (defined $content && length $content) ? eval { decode_json($content) } : {};
			$ok->($data // {});
		},
		sub {
			my ($http, $error, $res) = @_;
			# Only the operation and the transport's error are logged, never the arguments;
			# redact() guards against a secret echoed back in either.
			$log->warn(redact("admin API $op failed: " . ($error // 'unknown error'), $args));
			$err->(redact(_friendly($error, gateway_message($res, $http)), $args));
		},
		{ timeout => TIMEOUT },
	);
	my @headers = ('Authorization' => "Bearer $secret", 'X-Anywhere-Client' => 'plugin');
	if ($method eq 'GET') {
		$http->get($url, @headers);
	} else {
		push @headers, 'Content-Type' => 'application/json';
		$http->post($url, @headers, encode_json($body // {}));
	}
}

# The gateway's own explanation from an error reply ({"error": "..."}), if there is one:
# a single line of at most 200 characters, control characters removed. SimpleAsyncHTTP
# passes the response to the error callback (or keeps it on the request object).
sub gateway_message {
	for my $r (@_) {
		next unless ref $r;
		my $res = eval { $r->can('content') ? $r : $r->can('response') ? $r->response : undef } or next;
		my $content = eval { $res->content };
		next unless defined $content && length $content && length $content <= 65536;
		my $data = eval { decode_json($content) };
		next unless ref $data eq 'HASH' && defined $data->{error} && !ref $data->{error};
		my $m = "$data->{error}";
		$m =~ s/[\x00-\x1f\x7f]+/ /g;
		$m =~ s/\A\s+|\s+\z//g;
		next unless length $m;
		$m = substr($m, 0, 200) if length $m > 200;
		return $m;
	}
	return undef;
}

sub _friendly {
	my ($e, $message) = @_;
	$e //= '';
	return 'the gateway is not running'               if $e =~ /connect|refused|timed? ?out/i;
	return "refused by the gateway: $message"         if defined $message && $e !~ /403/;
	return 'not found'                                if $e =~ /404/;
	return 'refused by the gateway (check the input)' if $e =~ /400/;
	return 'the gateway refused the admin secret'     if $e =~ /403/;
	return "gateway error: $e";
}

1;
