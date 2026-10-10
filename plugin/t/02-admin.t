# Admin API client: request mapping, validation, and what goes on the wire.
use strict; use warnings;
use Test::More;
use File::Temp qw(tempdir);
use JSON::PP;
use Plugins::LyrionAnywhere::Admin;
use Slim::Networking::SimpleAsyncHTTP;
use Slim::Utils::Prefs;

sub req { [Plugins::LyrionAnywhere::Admin::request_for(@_)] }

is_deeply(req('users', {}), ['GET', '/admin/v1/users', undef]);
is_deeply(req('user_create', { username => 'alice', password => 'correct horse', role => 'listener' }),
	['POST', '/admin/v1/users', { username => 'alice', password => 'correct horse', role => 'listener' }]);
is_deeply(req('ungrant', { username => 'alice', player => 'AA:BB:CC:DD:EE:01' }),
	['POST', '/admin/v1/users/alice/players/aa:bb:cc:dd:ee:01/remove', undef]);
is_deeply(req('revoke', { id => 12 }), ['POST', '/admin/v1/devices/12/revoke', undef]);
is_deeply(req('diagnostics', { id => 12 }), ['GET', '/admin/v1/devices/12/diagnostics', undef]);
is_deeply(req('audit', { limit => 50 }), ['GET', '/admin/v1/audit?limit=50', undef]);
is_deeply(req('settings', {}), ['GET', '/admin/v1/settings', undef]);
is_deeply(req('plugins', {}), ['GET', '/admin/v1/plugins', undef]);
is_deeply(req('plugins_admin_only', { commands => 'audiomuseai,custombrowse' }),
	['POST', '/admin/v1/plugins', { admin_only => ['audiomuseai', 'custombrowse'] }]);
is_deeply(req('plugins_admin_only', {}), ['POST', '/admin/v1/plugins', { admin_only => [] }], 'none: an empty list');
my (undef, undef, $days) = Plugins::LyrionAnywhere::Admin::request_for('session_days', { days => '90' });
is(JSON::PP->new->canonical->encode($days), '{"session_days":90}', 'the lifetime is a JSON number');
{
	my $json = JSON::PP->new->canonical;
	my ($m, $p, $b) = Plugins::LyrionAnywhere::Admin::request_for('user_downloads', { username => 'bob', enabled => 0 });
	is("$m $p", 'POST /admin/v1/users/bob/downloads', 'downloads: path');
	is($json->encode($b), '{"enabled":false}', 'downloads off: a JSON boolean, formats left alone');
	(undef, undef, $b) = Plugins::LyrionAnywhere::Admin::request_for('user_downloads',
		{ username => 'bob', enabled => 1, formats => 'original,opus' });
	is($json->encode($b), '{"enabled":true,"formats":["original","opus"]}', 'downloads with formats: a list');
	ok(Plugins::LyrionAnywhere::Admin::is_mutating('user_downloads'), 'changing downloads is mutating');
}
my ($m, $p, $b) = Plugins::LyrionAnywhere::Admin::request_for('user_disable', { username => 'bob', disabled => 1 });
is(JSON::PP->new->canonical->encode($b), '{"disabled":true}', 'disabled is a JSON boolean');

# Anything that could alter the path or break a header is refused before sending.
my %attacks = (
	'path traversal in username' => ['user_delete', { username => '../users' }],
	'slash in username'          => ['grants',      { username => 'a/b' }],
	'query in username'          => ['grants',      { username => 'a?x=1' }],
	'upper case username'        => ['grants',      { username => 'Alice' }],
	'CRLF in password'           => ['user_password', { username => 'a', password => "abcdefghij\r\nX-Evil: 1" }],
	'short password'             => ['user_create', { username => 'a', password => 'short', role => 'listener' }],
	'unknown role'               => ['user_create', { username => 'a', password => 'abcdefghijk', role => 'root' }],
	'mac with path'              => ['ungrant',     { username => 'a', player => '../../users' }],
	'non-numeric device'         => ['revoke',      { id => '1/../2' }],
	'non-numeric diagnostics id' => ['diagnostics', { id => '../audit' }],
	'huge limit'                 => ['audit',       { limit => 100000 }],
	'zero days'                  => ['session_days', { days => 0 }],
	'too many days'              => ['session_days', { days => 366 }],
	'negative days'              => ['session_days', { days => -1 }],
	'fractional days'            => ['session_days', { days => '1.5' }],
	'exponent days'              => ['session_days', { days => '1e2' }],
	'days with junk'             => ['session_days', { days => "30\n" }],
	'missing days'               => ['session_days', {}],
	'plugin with a space'        => ['plugins_admin_only', { commands => 'audio muse' }],
	'plugin with a slash'        => ['plugins_admin_only', { commands => 'a/../b' }],
	'plugin newline'             => ['plugins_admin_only', { commands => "audiomuseai\n" }],
	'plugin too long'            => ['plugins_admin_only', { commands => 'x' x 65 }],
	'too many plugins'           => ['plugins_admin_only', { commands => join(',', map { "p$_" } 1 .. 201) }],
	'plugins as a list'          => ['plugins_admin_only', { commands => ['a'] }],
	# Regression: Perl's $ also matches before a trailing newline, so these used to pass.
	'newline after username'     => ['grants',      { username => "alice\n" }],
	'newline after mac'          => ['ungrant',     { username => 'a', player => "aa:bb:cc:dd:ee:01\n" }],
	'newline after device id'    => ['revoke',      { id => "12\n" }],
	'newline after role'         => ['user_create', { username => 'a', password => 'abcdefghijk', role => "admin\n" }],
	'newline after disabled'     => ['user_disable', { username => 'a', disabled => "1\n" }],
	'newline after limit'        => ['audit',       { limit => "50\n" }],
	'unknown operation'          => ['restart_lms', {}],
	'missing username'           => ['pairing',     {}],
	# Downloads: strict booleans and a known, non-repeating format list.
	'downloads without enabled'  => ['user_downloads', { username => 'a' }],
	'downloads enabled=yes'      => ['user_downloads', { username => 'a', enabled => 'yes' }],
	'downloads enabled=2'        => ['user_downloads', { username => 'a', enabled => 2 }],
	'newline after enabled'      => ['user_downloads', { username => 'a', enabled => "1\n" }],
	'unknown format'             => ['user_downloads', { username => 'a', enabled => 1, formats => 'flac' }],
	'empty formats'              => ['user_downloads', { username => 'a', enabled => 1, formats => '' }],
	'repeated format'            => ['user_downloads', { username => 'a', enabled => 1, formats => 'mp3,mp3' }],
	'too many formats'           => ['user_downloads', { username => 'a', enabled => 1, formats => 'original,mp3,opus,mp3' }],
	'formats with a space'       => ['user_downloads', { username => 'a', enabled => 1, formats => 'mp3, opus' }],
	'upper-case format'          => ['user_downloads', { username => 'a', enabled => 1, formats => 'MP3' }],
	'newline after formats'      => ['user_downloads', { username => 'a', enabled => 1, formats => "mp3\n" }],
	'formats as a list'          => ['user_downloads', { username => 'a', enabled => 1, formats => ['mp3'] }],
	'trailing comma in formats'  => ['user_downloads', { username => 'a', enabled => 1, formats => 'mp3,' }],
	'downloads path traversal'   => ['user_downloads', { username => '../users', enabled => 1 }],
);
for my $name (sort keys %attacks) {
	my ($op, $args) = @{ $attacks{$name} };
	ok(!eval { Plugins::LyrionAnywhere::Admin::request_for($op, $args); 1 }, "refused: $name");
}

# call(): no secret yet → friendly error, nothing sent.
my $dir = tempdir(CLEANUP => 1);
$Slim::Utils::Prefs::DIR = $dir;
my ($got, $error);
Plugins::LyrionAnywhere::Admin::call('users', {}, sub { $got = shift }, sub { $error = shift });
like($error, qr/not started/, 'no secret: friendly error');
is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, 0, 'nothing sent without a secret');

# With a secret: bearer header, loopback URL, JSON body.
mkdir "$dir/lyrion-anywhere";
open my $fh, '>', "$dir/lyrion-anywhere/admin.secret" or die; print $fh ('ab' x 32) . "\n"; close $fh;
Plugins::LyrionAnywhere::Admin::call('grant', { username => 'alice', player => 'aa:bb:cc:dd:ee:01' }, sub { $got = shift }, sub { $error = shift });
my $r = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
is($r->{method}, 'POST');
is($r->{url}, 'http://127.0.0.1:9445/admin/v1/users/alice/players');
is($r->{headers}{Authorization}, 'Bearer ' . ('ab' x 32));
is_deeply(decode_json($r->{body}), { player => 'aa:bb:cc:dd:ee:01' });
Slim::Networking::SimpleAsyncHTTP::respond($r, '');
is_deeply($got, {}, '204 with no body is success');

Plugins::LyrionAnywhere::Admin::call('users', {}, sub { $got = shift }, sub { $error = shift });
Slim::Networking::SimpleAsyncHTTP::respond($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], '{"users":[{"username":"alice"}]}');
is($got->{users}[0]{username}, 'alice');

Plugins::LyrionAnywhere::Admin::call('users', {}, sub { $got = shift }, sub { $error = shift });
Slim::Networking::SimpleAsyncHTTP::fail($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], 'Connection refused');
is($error, 'the gateway is not running', 'connection errors are explained');

# A tampered secret file is not used.
open $fh, '>', "$dir/lyrion-anywhere/admin.secret" or die; print $fh "not-hex\r\nX-Evil: 1"; close $fh;
my $before = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
Plugins::LyrionAnywhere::Admin::call('users', {}, sub {}, sub { $error = shift });
is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $before, 'malformed secret: nothing sent');

done_testing;
