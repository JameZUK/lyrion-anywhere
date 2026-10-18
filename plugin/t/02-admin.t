# Admin API client: request mapping, validation, and what goes on the wire.
use strict; use warnings;
use Test::More;
use File::Temp qw(tempdir);
use JSON::PP;
use Plugins::LyrionAnywhere::Admin;
use Slim::Networking::SimpleAsyncHTTP;
use Slim::Utils::Log;
use Slim::Utils::Prefs;
use lib 't/lib';
use AnywhereTest qw(start_stub_gateway);

sub req { [Plugins::LyrionAnywhere::Admin::request_for(@_)] }

is_deeply(req('users', {}), ['GET', '/admin/v1/users', undef]);
is_deeply(req('user_create', { username => 'alice', password => 'correct horse', role => 'listener' }),
	['POST', '/admin/v1/users', { username => 'alice', password => 'correct horse', role => 'listener' }]);
is_deeply(req('ungrant', { username => 'alice', player => 'AA:BB:CC:DD:EE:01' }),
	['POST', '/admin/v1/users/alice/players/aa:bb:cc:dd:ee:01/remove', undef]);
is_deeply(req('revoke', { id => 12 }), ['POST', '/admin/v1/devices/12/revoke', undef]);
is_deeply(req('diagnostics', { id => 12 }), ['GET', '/admin/v1/devices/12/diagnostics', undef]);
is_deeply(req('device_rename', { id => 12, name => 'Kitchen iPad' }), ['POST', '/admin/v1/devices/12/name', { name => 'Kitchen iPad' }]);
is_deeply(req('device_rename', { id => 3, name => "K\x{fc}che \x{2019}n" }), ['POST', '/admin/v1/devices/3/name', { name => "K\x{fc}che \x{2019}n" }], 'any script (the gateway cleans names)');
is_deeply(req('device_rename', { id => 3, name => 'x' x 64 }), ['POST', '/admin/v1/devices/3/name', { name => 'x' x 64 }], '64 characters');
ok(Plugins::LyrionAnywhere::Admin::is_mutating('device_rename'), 'renaming is mutating');
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
	# Device names: 1-64 characters, no line breaks or other controls, a plain string.
	'empty device name'          => ['device_rename', { id => 1, name => '' }],
	'blank device name'          => ['device_rename', { id => 1, name => "  \t " }],
	'device name too long'       => ['device_rename', { id => 1, name => 'x' x 65 }],
	'newline in device name'     => ['device_rename', { id => 1, name => "Kitchen\nX-Evil: 1" }],
	'CR in device name'          => ['device_rename', { id => 1, name => "Kitchen\r" }],
	'NUL in device name'         => ['device_rename', { id => 1, name => "Kit\0chen" }],
	'DEL in device name'         => ['device_rename', { id => 1, name => "Kit\x7fchen" }],
	'device name as a list'      => ['device_rename', { id => 1, name => ['Kitchen'] }],
	'missing device name'        => ['device_rename', { id => 1 }],
	'rename without an id'       => ['device_rename', { name => 'Kitchen' }],
	'rename id traversal'        => ['device_rename', { id => '1/../../users', name => 'Kitchen' }],
	'rename id with newline'     => ['device_rename', { id => "1\n", name => 'Kitchen' }],
);
for my $name (sort keys %attacks) {
	my ($op, $args) = @{ $attacks{$name} };
	ok(!eval { Plugins::LyrionAnywhere::Admin::request_for($op, $args); 1 }, "refused: $name");
}

# --- Cloudflare Tunnel: request mapping and inputs (docs/plans/cloudflare-tunnel.md) ------
use MIME::Base64 qw(encode_base64);

# A token of the shape Cloudflare's dashboard shows (made up: base64 JSON with a, t and s).
sub tunnel_token {
	my %o = (a => '0123456789abcdef0123456789abcdef', t => '01234567-89ab-cdef-0123-456789abcdef',
		s => encode_base64('x' x 32, ''), @_);
	my $json = JSON::PP->new->canonical->encode({ map { defined $o{$_} ? ($_ => $o{$_}) : () } keys %o });
	return encode_base64($json, '');
}
my $TOKEN = tunnel_token();
my $API_TOKEN = 'Abc_def-0123456789xyzABCDEFGHIJKLMNOPQRS';
{
	my $json = JSON::PP->new->canonical;
	my %plain = (
		tunnel_status  => 'GET /admin/v1/tunnel',
		tunnel_log     => 'GET /admin/v1/tunnel/log',
		tunnel_install => 'POST /admin/v1/tunnel/install',
		tunnel_update  => 'POST /admin/v1/tunnel/update',
		tunnel_start   => 'POST /admin/v1/tunnel/start',
		tunnel_stop    => 'POST /admin/v1/tunnel/stop',
		tunnel_quick   => 'POST /admin/v1/tunnel/quick',
		tunnel_remove  => 'POST /admin/v1/tunnel/remove',
	);
	for my $op (sort keys %plain) {
		my ($m, $p, $b) = Plugins::LyrionAnywhere::Admin::request_for($op, {});
		is("$m $p", $plain{$op}, "$op: method and path");
		is($b, undef, "$op: no body of its own (an empty JSON object is sent)");
		is(Plugins::LyrionAnywhere::Admin::is_mutating($op), $m eq 'POST' ? 1 : 0, "$op: mutating only if it is a POST");
	}
	# Unrelated arguments are not passed on.
	is_deeply(req('tunnel_start', { hostname => 'music.example.com', token => $TOKEN }), ['POST', '/admin/v1/tunnel/start', undef], 'start ignores other arguments');

	my ($m, $p, $b) = Plugins::LyrionAnywhere::Admin::request_for('tunnel_register', { token => $TOKEN, hostname => 'Music.Example.COM' });
	is("$m $p", 'POST /admin/v1/tunnel/register', 'register: method and path');
	is($json->encode($b), $json->encode({ token => $TOKEN, hostname => 'music.example.com' }), 'register: token as given, hostname lower-cased');

	($m, $p, $b) = Plugins::LyrionAnywhere::Admin::request_for('tunnel_create', { api_token => $API_TOKEN, hostname => 'MUSIC.example.com', replace_dns => 1 });
	is("$m $p", 'POST /admin/v1/tunnel/create', 'create: method and path');
	is($json->encode($b), qq({"api_token":"$API_TOKEN","hostname":"music.example.com","replace_dns":true}), 'create: replace_dns a JSON true, hostname lower-cased');
	(undef, undef, $b) = Plugins::LyrionAnywhere::Admin::request_for('tunnel_create', { api_token => $API_TOKEN, hostname => 'music.example.com', replace_dns => '0' });
	is($json->encode($b), qq({"api_token":"$API_TOKEN","hostname":"music.example.com","replace_dns":false}), 'create: replace_dns 0 is a JSON false');
	(undef, undef, $b) = Plugins::LyrionAnywhere::Admin::request_for('tunnel_create', { api_token => $API_TOKEN, hostname => 'music.example.com' });
	is($json->encode($b), qq({"api_token":"$API_TOKEN","hostname":"music.example.com","replace_dns":false}), 'create: no replace_dns means false');
}

# Hostnames.
{
	my $ok = \&Plugins::LyrionAnywhere::Admin::valid_hostname;
	for my $h ('music.example.com', 'a.bc', 'MUSIC.Example.Com', 'x-1.my-home.example.co.uk', 'a1.b2', ('a' x 63) . '.example.com',
		join('.', ('a' x 63) x 3, 'a' x 61), 'evil-trycloudflare.com') {
		ok($ok->($h), 'hostname accepted: ' . substr($h, 0, 40));
	}
	for my $case (
		['too short', 'a.b'], ['empty', ''], ['undefined', undef], ['one label', 'localhost'], ['one long label', 'musicexamplecom'],
		['too long', join('.', ('a' x 63) x 4)], ['label too long', ('a' x 64) . '.example.com'],
		['leading hyphen', '-music.example.com'], ['trailing hyphen', 'music-.example.com'], ['empty label', 'music..example.com'],
		['leading dot', '.music.example.com'], ['trailing dot', 'music.example.com.'], ['underscore', 'my_music.example.com'],
		['port', 'music.example.com:443'], ['path', 'music.example.com/lms'], ['scheme', 'https://music.example.com'],
		['IPv4 address', '10.0.0.10'], ['IPv6 address', '[::1]'], ['IPv6 bare', 'fe80::1'], ['numeric top-level', 'music.example.123'],
		['quick tunnel domain', 'random-words.trycloudflare.com'], ['quick tunnel apex', 'trycloudflare.com'],
		['tunnel domain', '0123.cfargotunnel.com'], ['tunnel apex, upper case', 'CFArgoTunnel.com'],
		['space', 'music example.com'], ['newline after', "music.example.com\n"], ['CRLF header', "music.example.com\r\nX-Evil: 1"],
		['NUL', "music.example.com\0"], ['non-ASCII', "m\x{fc}sic.example.com"], ['path traversal', '../music.example.com'],
		['a list', ['music.example.com']], ['user info', 'user@music.example.com'], ['query', 'music.example.com?x=1'],
	) {
		my ($name, $h) = @$case;
		ok(!$ok->($h), "hostname refused: $name");
	}
}

# Tunnel tokens: the shape only (base64 JSON with a, t and s); the gateway checks the rest.
{
	my $ok = \&Plugins::LyrionAnywhere::Admin::valid_tunnel_token;
	ok($ok->($TOKEN), 'a token of the dashboard shape is accepted');
	my $odd = tunnel_token(s => "?" x 40);
	like($odd, qr/[+\/]/, 'a token with + or / in it');
	(my $url = $odd) =~ tr{+/}{-_};
	ok($ok->($odd) && $ok->($url), 'standard and URL alphabets accepted');
	(my $bare = $TOKEN) =~ s/=+\z//;
	ok($ok->($bare), 'padding optional');
	for my $case (
		['too short', 'eyJhIjoiYSJ9'], ['empty', ''], ['undefined', undef],
		['too long', 'A' x 4097],
		['bad alphabet', substr($TOKEN, 0, 70) . '!!!!'], ['space inside', substr($TOKEN, 0, 40) . ' ' . substr($TOKEN, 40)],
		['newline after', "$TOKEN\n"], ['CRLF header', "$TOKEN\r\nX-Evil: 1"], ['path characters', '../' x 30],
		['too much padding', "$TOKEN==="],
		['base64 but not JSON', encode_base64('x' x 80, '')],
		['JSON but a list', encode_base64('["a","t","s","' . ('x' x 60) . '"]', '')],
		['no a', tunnel_token(a => undef)], ['no t', tunnel_token(t => undef)], ['no s', tunnel_token(s => undef)],
		['empty s', tunnel_token(s => '')], ['s an object', tunnel_token(s => { x => 1 })],
		['a list', [$TOKEN]],
	) {
		my ($name, $t) = @$case;
		ok(!$ok->($t), "tunnel token refused: $name");
	}
}

# API tokens: 20-128 characters of A-Z a-z 0-9 _ -.
{
	my $ok = \&Plugins::LyrionAnywhere::Admin::valid_api_token;
	ok($ok->($API_TOKEN), 'API token accepted');
	ok($ok->('a' x 20) && $ok->('a' x 128), '20 and 128 characters');
	for my $case (['too short', 'a' x 19], ['too long', 'a' x 129], ['bad alphabet', ('a' x 30) . '+/'], ['dot', ('a' x 30) . '.b'],
		['space', ('a' x 15) . ' ' . ('b' x 15)], ['newline after', ('a' x 40) . "\n"], ['CRLF header', ('a' x 40) . "\r\nX-Evil: 1"],
		['Bearer prefix', 'Bearer ' . ('a' x 40)], ['empty', ''], ['undefined', undef], ['a list', ['a' x 40]]) {
		my ($name, $t) = @$case;
		ok(!$ok->($t), "API token refused: $name");
	}
}

# The commands refuse bad input before anything is built, saying what is wrong without
# repeating the value.
{
	my %bad = (
		'register without a token'    => ['tunnel_register', { hostname => 'music.example.com' }, qr/tunnel token/],
		'register without a hostname' => ['tunnel_register', { token => $TOKEN }, qr/hostname/],
		'register, bad token'         => ['tunnel_register', { token => 'x' x 70, hostname => 'music.example.com' }, qr/tunnel token/],
		'register, hostname with path'=> ['tunnel_register', { token => $TOKEN, hostname => 'music.example.com/../x' }, qr/hostname/],
		'register, hostname newline'  => ['tunnel_register', { token => $TOKEN, hostname => "music.example.com\n" }, qr/hostname/],
		'register, quick domain'      => ['tunnel_register', { token => $TOKEN, hostname => 'a.trycloudflare.com' }, qr/trycloudflare/],
		'create without an API token' => ['tunnel_create', { hostname => 'music.example.com' }, qr/API token/],
		'create without a hostname'   => ['tunnel_create', { api_token => $API_TOKEN }, qr/hostname/],
		'create, short API token'     => ['tunnel_create', { api_token => 'short', hostname => 'music.example.com' }, qr/API token/],
		'create, API token CRLF'      => ['tunnel_create', { api_token => "$API_TOKEN\r\nX: y", hostname => 'music.example.com' }, qr/API token/],
		'create, replace_dns=yes'     => ['tunnel_create', { api_token => $API_TOKEN, hostname => 'music.example.com', replace_dns => 'yes' }, qr/replace_dns/],
		'create, replace_dns=2'       => ['tunnel_create', { api_token => $API_TOKEN, hostname => 'music.example.com', replace_dns => 2 }, qr/replace_dns/],
		'create, replace_dns=true'    => ['tunnel_create', { api_token => $API_TOKEN, hostname => 'music.example.com', replace_dns => 'true' }, qr/replace_dns/],
		'create, replace_dns newline' => ['tunnel_create', { api_token => $API_TOKEN, hostname => 'music.example.com', replace_dns => "1\n" }, qr/replace_dns/],
		'create, replace_dns empty'   => ['tunnel_create', { api_token => $API_TOKEN, hostname => 'music.example.com', replace_dns => '' }, qr/replace_dns/],
		'create, replace_dns a list'  => ['tunnel_create', { api_token => $API_TOKEN, hostname => 'music.example.com', replace_dns => [1] }, qr/replace_dns/],
		'create, IP address'          => ['tunnel_create', { api_token => $API_TOKEN, hostname => '10.0.0.10' }, qr/hostname/],
		'create, port'                => ['tunnel_create', { api_token => $API_TOKEN, hostname => 'music.example.com:8443' }, qr/hostname/],
	);
	for my $name (sort keys %bad) {
		my ($op, $args, $why) = @{ $bad{$name} };
		ok(!eval { Plugins::LyrionAnywhere::Admin::request_for($op, $args); 1 }, "refused: $name");
		my $e = $@ // '';
		like($e, $why, "$name: says why");
		for my $secret (grep { defined && !ref && length > 4 } map { $args->{$_} } qw(token api_token)) {
			ok(index($e, $secret) < 0, "$name: the message does not repeat the secret");
		}
	}
}

# redact() masks secret arguments, and only those.
{
	my $r = \&Plugins::LyrionAnywhere::Admin::redact;
	is($r->("x $TOKEN y", { token => $TOKEN }), 'x [hidden] y', 'tunnel token masked');
	is($r->("$API_TOKEN/$API_TOKEN", { api_token => $API_TOKEN }), '[hidden]/[hidden]', 'API token masked everywhere');
	is($r->('a.b+c', { password => 'a.b+c' }), '[hidden]', 'regex characters are taken literally');
	is($r->('music.example.com', { hostname => 'music.example.com' }), 'music.example.com', 'non-secrets left alone');
	is($r->(undef, { token => 'abcd' }), undef, 'undef stays undef');
}

# call(): no secret yet → friendly error, nothing sent.
my $dir = tempdir(CLEANUP => 1);
$Slim::Utils::Prefs::DIR = $dir;
my ($got, $error);
Plugins::LyrionAnywhere::Admin::call('users', {}, sub { $got = shift }, sub { $error = shift });
like($error, qr/not started/, 'no secret: friendly error');
is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, 0, 'nothing sent without a secret');

# With a secret, but no gateway started by this plugin: nothing is sent.
mkdir "$dir/lyrion-anywhere";
open my $fh, '>', "$dir/lyrion-anywhere/admin.secret" or die; print $fh ('ab' x 32) . "\n"; close $fh;
undef $error;
Plugins::LyrionAnywhere::Admin::call('users', {}, sub { $got = shift }, sub { $error = shift });
is($error, 'the gateway is not running', 'no gateway started: refused');
is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, 0, 'the secret is not sent anywhere');

# With a secret and the gateway this plugin started: bearer header, loopback URL, JSON body.
start_stub_gateway($dir);
Plugins::LyrionAnywhere::Admin::call('grant', { username => 'alice', player => 'aa:bb:cc:dd:ee:01' }, sub { $got = shift }, sub { $error = shift });
my $r = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
is($r->{method}, 'POST');
is($r->{url}, 'http://127.0.0.1:9445/admin/v1/users/alice/players');
is($r->{headers}{Authorization}, 'Bearer ' . ('ab' x 32));
is_deeply(decode_json($r->{body}), { player => 'aa:bb:cc:dd:ee:01' });
Slim::Networking::SimpleAsyncHTTP::respond($r, '');
is_deeply($got, {}, '204 with no body is success');

# A rename goes out as UTF-8 JSON, whatever the script.
Plugins::LyrionAnywhere::Admin::call('device_rename', { id => 7, name => "K\x{fc}che \x{263a}" }, sub { $got = shift }, sub { $error = shift });
$r = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
is("$r->{method} $r->{url}", 'POST http://127.0.0.1:9445/admin/v1/devices/7/name', 'rename: method and URL');
is_deeply(decode_json($r->{body}), { name => "K\x{fc}che \x{263a}" }, 'rename: the name survives the trip');
ok(!utf8::is_utf8($r->{body}), 'rename: the body is UTF-8 bytes');

Plugins::LyrionAnywhere::Admin::call('users', {}, sub { $got = shift }, sub { $error = shift });
Slim::Networking::SimpleAsyncHTTP::respond($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], '{"users":[{"username":"alice"}]}');
is($got->{users}[0]{username}, 'alice');

Plugins::LyrionAnywhere::Admin::call('users', {}, sub { $got = shift }, sub { $error = shift });
Slim::Networking::SimpleAsyncHTTP::fail($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], 'Connection refused');
is($error, 'the gateway is not running', 'connection errors are explained');

# The gateway's own reason is shown (one cleaned line), not just the status code.
for my $case (
	['400 Bad Request', '{"error":"username already exists"}', 'refused by the gateway: username already exists'],
	['500 Internal Server Error', '{"error":"database is locked\r\nX-Evil: 1"}', 'refused by the gateway: database is locked X-Evil: 1'],
	['400 Bad Request', '{"error":"' . ('x' x 500) . '"}', 'refused by the gateway: ' . ('x' x 200)],
	['400 Bad Request', 'not json', 'refused by the gateway (check the input)'],
	['400 Bad Request', '{"error":{"nested":1}}', 'refused by the gateway (check the input)'],
	['404 Not Found', '', 'not found'],
	['403 Forbidden', '{"error":"anything"}', 'the gateway refused the admin secret'],
) {
	my ($status, $body, $want) = @$case;
	Plugins::LyrionAnywhere::Admin::call('users', {}, sub { $got = shift }, sub { $error = shift });
	Slim::Networking::SimpleAsyncHTTP::fail($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], $status, $body);
	is($error, $want, "gateway error $status with " . substr($body, 0, 30));
}

# Passwords never reach the log, even when the gateway refuses the request.
{
	@Slim::Utils::Log::MESSAGES = ();
	my $pw = 'correct horse battery staple';
	Plugins::LyrionAnywhere::Admin::call('user_create', { username => 'dave', password => $pw, role => 'listener' }, sub {}, sub { $error = shift });
	Slim::Networking::SimpleAsyncHTTP::fail($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], '400 Bad Request', '{"error":"refused"}');
	Plugins::LyrionAnywhere::Admin::call('user_password', { username => 'dave', password => $pw }, sub {}, sub { $error = shift });
	Slim::Networking::SimpleAsyncHTTP::fail($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], 'Connection refused');
	ok(@Slim::Utils::Log::MESSAGES, 'the failures were logged');
	ok(!grep({ index($_->[1], $pw) >= 0 } @Slim::Utils::Log::MESSAGES), 'without the password');
}

# --- the admin secret goes only to the port the plugin started the gateway on -----------
# Regression: admin_port is an ordinary pref (LMS's `pref` command can change it), and the
# secret went to whatever port it named.
{
	my $prefs = preferences('plugin.lyrionanywhere');
	$prefs->set(admin_port => 9446);
	is($prefs->get('admin_port'), 9446, 'the pref changes (a valid port)');
	Plugins::LyrionAnywhere::Admin::call('users', {}, sub {}, sub { $error = shift });
	is($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1]{url}, 'http://127.0.0.1:9445/admin/v1/users',
		'still sent to the port the running gateway was started with');

	# Stopped: nothing is sent at all.
	Plugins::LyrionAnywhere::Gateway->stop;
	my $n = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
	undef $error;
	Plugins::LyrionAnywhere::Admin::call('users', {}, sub {}, sub { $error = shift });
	is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $n, 'gateway stopped: the secret is not sent');
	is($error, 'the gateway is not running');

	# The gateway crashed: nothing is sent to its old port.
	my $gw = start_stub_gateway($dir, admin_port => 9446);
	Plugins::LyrionAnywhere::Admin::call('users', {}, sub {}, sub { $error = shift });
	is($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1]{url}, 'http://127.0.0.1:9446/admin/v1/users', 'new port after a restart');
	$gw->die;
	$n = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
	Plugins::LyrionAnywhere::Admin::call('users', {}, sub {}, sub { $error = shift });
	is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $n, 'gateway exited: the secret is not sent');
	Plugins::LyrionAnywhere::Gateway->stop;
}

# Where the system can tell (Linux), the port must be held by the gateway's own process: a
# local program listening there instead never gets the secret.
SKIP: {
	skip 'needs /proc/net/tcp', 4 unless -r '/proc/net/tcp' && -d "/proc/$$/fd";
	require IO::Socket::INET;
	# The "gateway": a real child process, holding a listening socket on the admin port.
	my $spawn = sub {
		my $listen = shift;
		pipe my $r, my $w or die;
		my $pid = fork // die;
		if (!$pid) {
			close $r;
			my $s = $listen ? IO::Socket::INET->new(Listen => 1, LocalAddr => '127.0.0.1', LocalPort => 0, ReuseAddr => 1) : undef;
			print $w ($s ? $s->sockport : 0), "\n";
			close $w;
			sleep 30;
			POSIX::_exit(0);
		}
		close $w;
		chomp(my $port = <$r>);
		return ($pid, $port);
	};
	require POSIX;
	my ($pid, $port) = $spawn->(1);
	start_stub_gateway($dir, admin_port => $port, pid => $pid);
	my $n = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
	Plugins::LyrionAnywhere::Admin::call('users', {}, sub {}, sub { $error = shift });
	is($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1]{url}, "http://127.0.0.1:$port/admin/v1/users", 'the gateway holds its port: sent');
	Plugins::LyrionAnywhere::Gateway->stop;
	# Let the first one go: while it is still exiting, the next start waits for it (on a timer),
	# and the check below would see no gateway at all rather than the wrong port holder.
	for (1 .. 100) { Plugins::LyrionAnywhere::Gateway::_reap(); last unless kill 0, $pid; select undef, undef, undef, 0.05 }

	# Another local program holds the port the gateway was told to use.
	# (The gateway process starts first, so it doesn't inherit the other program's socket.)
	my ($pid2) = $spawn->(0);
	my $squatter = IO::Socket::INET->new(Listen => 1, LocalAddr => '127.0.0.1', LocalPort => 0, ReuseAddr => 1) or die;
	start_stub_gateway($dir, admin_port => $squatter->sockport, pid => $pid2);
	$n = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
	undef $error;
	@Slim::Utils::Log::MESSAGES = ();
	Plugins::LyrionAnywhere::Admin::call('users', {}, sub {}, sub { $error = shift });
	is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $n, 'port held by another program: the secret is not sent');
	is($error, 'the gateway is not running');
	ok(grep({ $_->[1] =~ /not held by the gateway/ } @Slim::Utils::Log::MESSAGES), 'and the log says why');
	Plugins::LyrionAnywhere::Gateway->stop;
	for (1 .. 50) { last if waitpid($pid, POSIX::WNOHANG()) && waitpid($pid2, POSIX::WNOHANG()); select undef, undef, undef, 0.1 }
	kill 'KILL', $pid, $pid2;
	start_stub_gateway($dir);
}

# LMS's `webproxy` pref sends every plain-HTTP request through the proxy unless the host
# matches localhost or 127.0.0.1 (Slim::Networking::Async::HTTP::use_proxy, LMS 8.5 to
# 9.1). So the secret goes only to 127.0.0.1: another loopback address ([::1], 127.0.0.2)
# would be proxied and the secret would leave the host (review 2026-10-15, L).
{
	no warnings 'redefine';
	for my $target ('[::1]:9445', '127.0.0.2:9445', '127.0.0.10:9445', '127.0.0.1.example.com:9445', 'localhost:9445', '10.0.0.10:9445', '127.0.0.1:9445/x') {
		local *Plugins::LyrionAnywhere::Gateway::admin_target = sub { $target };
		undef $error;
		my $before = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
		Plugins::LyrionAnywhere::Admin::call('users', {}, sub {}, sub { $error = shift });
		is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $before, "admin target $target: the secret is not sent");
		like($error // '', qr/127\.0\.0\.1/, "admin target $target: refused, saying why");
	}
	local *Plugins::LyrionAnywhere::Gateway::admin_target = sub { '127.0.0.1:9445' };
	Plugins::LyrionAnywhere::Admin::call('users', {}, sub {}, sub { $error = shift });
	like($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1]{url}, qr{^http://127\.0\.0\.1:9445/}, '127.0.0.1: sent, never proxied by LMS');
}

# --- Cloudflare Tunnel on the wire: secrets go only in the body, never in a log or an error.
{
	start_stub_gateway($dir);
	@Slim::Utils::Log::MESSAGES = ();
	my ($data, $e);
	Plugins::LyrionAnywhere::Admin::call('tunnel_register', { token => $TOKEN, hostname => 'Music.Example.com' }, sub { $data = shift }, sub { $e = shift });
	my $r = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
	is("$r->{method} $r->{url}", 'POST http://127.0.0.1:9445/admin/v1/tunnel/register', 'register: sent to the admin API');
	is_deeply(decode_json($r->{body}), { token => $TOKEN, hostname => 'music.example.com' }, 'register: the body');
	ok(index($r->{url}, $TOKEN) < 0, 'the token is not in the URL');
	Slim::Networking::SimpleAsyncHTTP::respond($r, '');
	is_deeply($data, {}, 'register: 204 is success');

	Plugins::LyrionAnywhere::Admin::call('tunnel_install', {}, sub { $data = shift }, sub { $e = shift });
	$r = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
	is("$r->{method} $r->{url}", 'POST http://127.0.0.1:9445/admin/v1/tunnel/install', 'install: sent');
	is($r->{body}, '{}', 'install: an empty JSON object');
	undef $data;
	Slim::Networking::SimpleAsyncHTTP::respond($r, '{}');
	is_deeply($data, {}, 'install: 202 {} is success');

	Plugins::LyrionAnywhere::Admin::call('tunnel_status', {}, sub { $data = shift }, sub { $e = shift });
	$r = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
	is("$r->{method} $r->{url}", 'GET http://127.0.0.1:9445/admin/v1/tunnel', 'status: a GET');
	Slim::Networking::SimpleAsyncHTTP::respond($r, '{"state":"connected","connections":4,"quick_ends_in":null}');
	is($data->{state}, 'connected', 'status: the gateway answer is passed on');

	# A refusal gives the gateway's sentence.
	undef $e;
	Plugins::LyrionAnywhere::Admin::call('tunnel_create', { api_token => $API_TOKEN, hostname => 'music.example.com', replace_dns => 0 }, sub {}, sub { $e = shift });
	$r = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
	is_deeply(decode_json($r->{body}), { api_token => $API_TOKEN, hostname => 'music.example.com', replace_dns => JSON::PP::false }, 'create: the body');
	Slim::Networking::SimpleAsyncHTTP::fail($r, '409 Conflict',
		'{"error":"a DNS record for music.example.com already exists; tick Replace to overwrite it"}');
	is($e, 'refused by the gateway: a DNS record for music.example.com already exists; tick Replace to overwrite it', "409: the gateway's sentence");

	# Even a secret echoed back by the gateway or the HTTP client is masked, in the error and the log.
	for my $op (['tunnel_register', { token => $TOKEN, hostname => 'music.example.com' }, $TOKEN],
		['tunnel_create', { api_token => $API_TOKEN, hostname => 'music.example.com', replace_dns => 1 }, $API_TOKEN]) {
		my ($name, $args, $secret) = @$op;
		undef $e;
		Plugins::LyrionAnywhere::Admin::call($name, $args, sub {}, sub { $e = shift });
		Slim::Networking::SimpleAsyncHTTP::fail($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], "400 Bad Request ($secret)", qq({"error":"bad value $secret"}));
		is($e, 'refused by the gateway: bad value [hidden]', "$name: an echoed secret is masked in the error");
		Plugins::LyrionAnywhere::Admin::call($name, $args, sub {}, sub { $e = shift });
		Slim::Networking::SimpleAsyncHTTP::fail($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], "Connect timed out ($secret)");
		ok(index($e, $secret) < 0, "$name: and in a transport error");
	}
	ok(@Slim::Utils::Log::MESSAGES, 'the failures were logged');
	ok(!grep({ index($_->[1], $TOKEN) >= 0 || index($_->[1], $API_TOKEN) >= 0 } @Slim::Utils::Log::MESSAGES), 'without the tunnel token or the API token');

	# Bad input never reaches the wire.
	my $n = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
	undef $e;
	Plugins::LyrionAnywhere::Admin::call('tunnel_create', { api_token => "$API_TOKEN\r\nX-Evil: 1", hostname => 'music.example.com' }, sub {}, sub { $e = shift });
	is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $n, 'a bad API token is not sent');
	like($e, qr/API token/, 'and the reason is given');
	ok(index($e, $API_TOKEN) < 0, 'without the value');
}

# A tampered secret file is not used.
open $fh, '>', "$dir/lyrion-anywhere/admin.secret" or die; print $fh "not-hex\r\nX-Evil: 1"; close $fh;
my $before = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
Plugins::LyrionAnywhere::Admin::call('users', {}, sub {}, sub { $error = shift });
is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $before, 'malformed secret: nothing sent');
Plugins::LyrionAnywhere::Gateway->stop;

done_testing;
