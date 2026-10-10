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

use Slim::Networking::SimpleAsyncHTTP;
use Slim::Utils::Log;

use Plugins::LyrionAnywhere::Gateway;

my $log = logger('plugin.lyrionanywhere');

use constant TIMEOUT => 15;

sub valid_username { my $u = shift // ''; return $u =~ /^[a-z0-9._-]{1,64}\z/ }
sub valid_mac      { my $m = shift // ''; return $m =~ /^(?:[0-9a-f]{2}:){5}[0-9a-f]{2}\z/i }
sub valid_id       { my $i = shift // ''; return $i =~ /^\d{1,18}\z/ }

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
	diagnostics  => ['GET',  '/admin/v1/devices/{id}/diagnostics', [qw(id)]],
	audit        => ['GET',  '/admin/v1/audit?limit={limit}', [qw(limit)]],
	settings     => ['GET',  '/admin/v1/settings'],
	session_days => ['POST', '/admin/v1/settings', [qw(days)],
	                 sub { { session_days => 0 + $_[0]{days} } }],
	plugins      => ['GET',  '/admin/v1/plugins'],
	plugins_admin_only => ['POST', '/admin/v1/plugins', [qw(commands)],
	                 sub { { admin_only => [grep { length } split /,/, $_[0]{commands} // ''] } }],
);

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
	limit    => sub { ($_[0] // '') =~ /^\d{1,4}\z/ && $_[0] >= 1 && $_[0] <= 1000 },
	days     => sub { ($_[0] // '') =~ /^\d{1,3}\z/ && $_[0] >= 1 && $_[0] <= 365 },
	# Comma-separated plugin command words (none = no plugin is admin-only).
	commands => sub {
		my $v = $_[0] // '';
		return 0 if ref $v || $v !~ /^[A-Za-z0-9_,-]{0,13000}\z/;
		my @c = grep { length } split /,/, $v;
		return @c <= 200 && !grep { length > 64 } @c;
	},
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
	limit    => 'limit must be 1-1000',
	days     => 'the sign-in lifetime must be 1-365 days',
	commands => 'plugin commands are comma-separated words of A-Z a-z 0-9 _ - (at most 200)',
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
	# admin_listen() is always 127.0.0.1 and a validated port: the secret stays on this host.
	my $url = 'http://' . Plugins::LyrionAnywhere::Gateway::admin_listen() . $path;
	my $http = Slim::Networking::SimpleAsyncHTTP->new(
		sub {
			my $res = shift;
			my $content = $res->content;
			my $data = (defined $content && length $content) ? eval { decode_json($content) } : {};
			$ok->($data // {});
		},
		sub {
			my ($res, $error) = @_;
			$log->warn("admin API $op failed: " . ($error // 'unknown error'));
			$err->(_friendly($error));
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

sub _friendly {
	my $e = shift // '';
	return 'the gateway is not running'               if $e =~ /connect|refused|timed? ?out/i;
	return 'not found'                                if $e =~ /404/;
	return 'refused by the gateway (check the input)' if $e =~ /400/;
	return 'the gateway refused the admin secret'     if $e =~ /403/;
	return "gateway error: $e";
}

1;
