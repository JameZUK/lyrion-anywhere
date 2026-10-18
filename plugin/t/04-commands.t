# LMS commands: registration, validation before sending, async results.
use strict; use warnings;
use Test::More;
use File::Temp qw(tempdir);
use Plugins::LyrionAnywhere::Commands;
use Slim::Control::Request;
use Slim::Networking::SimpleAsyncHTTP;
use Slim::Utils::Prefs;
use Slim::Utils::Timers;
use Proc::Background;
use lib 't/lib';
use AnywhereTest qw(start_stub_gateway);

Plugins::LyrionAnywhere::Commands::register();
ok($Slim::Control::Request::DISPATCH{'anywhere status'}, 'status registered');
ok($Slim::Control::Request::DISPATCH{"anywhere $_"}, "$_ registered")
	for qw(users user_create user_delete grant ungrant revoke device_rename audit totp_reset pairing diagnostics settings session_days plugins plugins_admin_only user_downloads
		tunnel_status tunnel_log tunnel_install tunnel_update tunnel_register tunnel_create tunnel_start tunnel_stop tunnel_quick tunnel_remove);
is($Slim::Control::Request::DISPATCH{'anywhere users'}[0], 0, 'no player needed');

my $dir = tempdir(CLEANUP => 1);
$Slim::Utils::Prefs::DIR = $dir;
mkdir "$dir/lyrion-anywhere";
open my $fh, '>', "$dir/lyrion-anywhere/admin.secret" or die; print $fh 'cd' x 32; close $fh;
start_stub_gateway($dir);

# Invalid input: an error result, and nothing reaches the gateway.
my $before = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
my $r = Slim::Control::Request->new(['anywhere', 'user_delete'], { username => '../x', csrf => Plugins::LyrionAnywhere::Commands::csrf_token() });
Plugins::LyrionAnywhere::Commands::admin($r);
is($r->{status}, 'done');
like($r->{results}{error}, qr/usernames/);
is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $before, 'nothing sent');

# Valid input: processing until the gateway answers, then done with data.
my $token = Plugins::LyrionAnywhere::Commands::csrf_token();
$r = Slim::Control::Request->new(['anywhere', 'pairing'], { username => 'alice', csrf => $token });
Plugins::LyrionAnywhere::Commands::admin($r);
is($r->{status}, 'processing');
my $sent = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
is($sent->{url}, 'http://127.0.0.1:9445/admin/v1/users/alice/pairing');
Slim::Networking::SimpleAsyncHTTP::respond($sent, '{"code":"ABCDE23456","expires_in":600}');
is($r->{status}, 'done');
is($r->{results}{ok}, 1);
is($r->{results}{data}{code}, 'ABCDE23456');

# Gateway errors become an error result, not a crash.
$r = Slim::Control::Request->new(['anywhere', 'users'], { csrf => $token });
Plugins::LyrionAnywhere::Commands::admin($r);
Slim::Networking::SimpleAsyncHTTP::fail($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], '403 Forbidden');
is($r->{results}{error}, 'the gateway refused the admin secret');

# Unexpected parameters are ignored, not forwarded.
$r = Slim::Control::Request->new(['anywhere', 'users'], { evil => 'x', username => 'alice', csrf => $token });
Plugins::LyrionAnywhere::Commands::admin($r);
is($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1]{url}, 'http://127.0.0.1:9445/admin/v1/users');

# --- CSRF: LMS's JSON-RPC is open to any web page on the LAN (cross-origin text/plain
# POSTs), so every state-changing command needs the settings page's token. -------------

# Made-up Cloudflare secrets of the right shape (base64 JSON with a, t, s; an API token).
use MIME::Base64 qw(encode_base64);
my $TUNNEL_TOKEN = encode_base64('{"a":"0123456789abcdef0123456789abcdef","s":"' . encode_base64('y' x 32, '')
	. '","t":"01234567-89ab-cdef-0123-456789abcdef"}', '');
my $API_TOKEN = 'Zyx_wvu-9876543210abcdefghijklmnopqrstuv';

like($token, qr/^[0-9a-f]{32}\z/, 'token: 128 random bits as hex');
is(Plugins::LyrionAnywhere::Commands::csrf_token(), $token, 'one token per LMS process');

my %mutating_args = (
	user_create   => { username => 'mallory', password => 'attacker password', role => 'admin' },
	user_delete   => { username => 'alice' },
	user_password => { username => 'alice', password => 'attacker password' },
	user_disable  => { username => 'alice', disabled => 0 },
	grant         => { username => 'alice', player => 'aa:bb:cc:dd:ee:01' },
	ungrant       => { username => 'alice', player => 'aa:bb:cc:dd:ee:01' },
	revoke        => { id => 1 },
	device_rename => { id => 1, name => 'Kitchen' },
	totp_reset    => { username => 'alice' },
	pairing       => { username => 'alice' },
	session_days  => { days => 365 },
	plugins_admin_only => { commands => 'audiomuseai' },
	user_downloads => { username => 'alice', enabled => 1, formats => 'original,mp3,opus' },
	tunnel_install  => {},
	tunnel_update   => {},
	tunnel_register => { token => $TUNNEL_TOKEN, hostname => 'music.example.com' },
	tunnel_create   => { api_token => $API_TOKEN, hostname => 'music.example.com', replace_dns => 0 },
	tunnel_start    => {},
	tunnel_stop     => {},
	tunnel_quick    => {},
	tunnel_remove   => {},
);
my %read_only_args = (
	users => {}, grants => { username => 'alice' }, devices => {}, audit => {},
	diagnostics => { id => 1 }, settings => {}, plugins => {},
	tunnel_status => {}, tunnel_log => {},
);
# Every operation is classified, so a new one can't slip through unprotected.
is_deeply([sort(keys %mutating_args, keys %read_only_args)], [Plugins::LyrionAnywhere::Admin::operations()],
	'every admin operation is covered here');
ok(Plugins::LyrionAnywhere::Commands::mutating($_), "$_ is mutating") for sort keys %mutating_args;
ok(!Plugins::LyrionAnywhere::Commands::mutating($_), "$_ is read-only") for sort keys %read_only_args;
ok(Plugins::LyrionAnywhere::Commands::mutating('restart'), 'restart is mutating');
ok(Plugins::LyrionAnywhere::Commands::mutating('no_such_op'), 'unknown operations are treated as mutating');

my $wrong = $token =~ tr/0-9a-f/1-9a-f0/r;
for my $op (sort keys %mutating_args) {
	for my $case (
		['no token',          {}],
		['empty token',       { csrf => '' }],
		['wrong token',       { csrf => $wrong }],
		['token prefix',      { csrf => substr($token, 0, 16) }],
		['token plus junk',   { csrf => "${token}0" }],
		['upper-case token',  { csrf => uc $token }],
		['token as a list',   { csrf => [$token] }],
	) {
		my ($name, $extra) = @$case;
		my $n = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
		my $req = Slim::Control::Request->new(['anywhere', $op], { %{ $mutating_args{$op} }, %$extra });
		Plugins::LyrionAnywhere::Commands::admin($req);
		is($req->{status}, 'done', "$op, $name: answered at once");
		like($req->{results}{error} // '', qr/^refused/, "$op, $name: refused");
		is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $n, "$op, $name: nothing sent to the gateway");
	}
	my $req = Slim::Control::Request->new(['anywhere', $op], { %{ $mutating_args{$op} }, csrf => $token });
	Plugins::LyrionAnywhere::Commands::admin($req);
	is($req->{status}, 'processing', "$op with the token: sent");
	my $sent = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
	unlike(($sent->{url} // '') . ($sent->{body} // ''), qr/csrf|\Q$token\E/, "$op: the token is not passed to the gateway");
}

# Pentest 2026-10 (A7): read-only commands answered anyone on the LAN, and `devices` now
# holds remote users' addresses and browsers. They need the token too.
for my $op (sort keys %read_only_args) {
	for my $case (['no token', {}], ['wrong token', { csrf => $wrong }]) {
		my ($name, $extra) = @$case;
		my $n = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
		my $req = Slim::Control::Request->new(['anywhere', $op], { %{ $read_only_args{$op} }, %$extra });
		Plugins::LyrionAnywhere::Commands::admin($req);
		like($req->{results}{error} // '', qr/^refused/, "$op, $name: refused");
		is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $n, "$op, $name: nothing sent to the gateway");
	}
	my $req = Slim::Control::Request->new(['anywhere', $op], { %{ $read_only_args{$op} }, csrf => $token });
	Plugins::LyrionAnywhere::Commands::admin($req);
	is($req->{status}, 'processing', "$op works with the token");
	ok(!defined $req->{results}{error}, "$op: no error");
}

# restart and status.
{
	require Plugins::LyrionAnywhere::Gateway;
	my $restarts = 0;
	no warnings 'redefine';
	local *Plugins::LyrionAnywhere::Gateway::restart = sub { $restarts++ };
	local *Plugins::LyrionAnywhere::Gateway::state = sub { { running => 0 } };
	is($Slim::Control::Request::DISPATCH{'anywhere restart'}[2], 1, 'restart parses key:value arguments (for csrf)');
	my $req = Slim::Control::Request->new(['anywhere', 'restart'], {});
	Plugins::LyrionAnywhere::Commands::restart($req);
	like($req->{results}{error}, qr/^refused/, 'restart without the token: refused');
	$req = Slim::Control::Request->new(['anywhere', 'restart'], { csrf => $wrong });
	Plugins::LyrionAnywhere::Commands::restart($req);
	like($req->{results}{error}, qr/^refused/, 'restart with a wrong token: refused');
	is($restarts, 0, 'gateway not restarted');
	$req = Slim::Control::Request->new(['anywhere', 'restart'], { csrf => $token });
	Plugins::LyrionAnywhere::Commands::restart($req);
	is($restarts, 1, 'restart with the token works');
	$req = Slim::Control::Request->new(['anywhere', 'status'], {});
	Plugins::LyrionAnywhere::Commands::status($req);
	is($req->{status}, 'done', 'status needs no token');
	ok(!grep({ ($_ // '') eq $token } values %{ $req->{results} }), 'status does not reveal the token');
}

# `anywhere status` without the token (review 2026-10-15, L): any LAN CLI or JSON-RPC
# caller may ask, so it says only whether the gateway runs and the gist of the last error.
# Paths, the pid, the listen address and the gateway's stderr need the settings page's token.
{
	require Plugins::LyrionAnywhere::Gateway;
	no warnings 'redefine';
	my $full = {
		enabled => 1, running => 0, pid => 4242, uptime => 0,
		binary => '/srv/lms/Plugins/LyrionAnywhere/Bin/x86_64-linux/anywhere-gateway',
		binary_ok => 1, data_dir => '/srv/lms/prefs/lyrion-anywhere', listen => '10.0.0.10:9443',
		error => "gateway exited: Error: cannot open /srv/lms/prefs/lyrion-anywhere/anywhere.db\nsecond line",
		settings_notice => 'trusted_proxies was changed outside the settings page and put back',
	};
	local *Plugins::LyrionAnywhere::Gateway::state = sub { +{ %$full } };
	is($Slim::Control::Request::DISPATCH{'anywhere status'}[2], 1, 'status parses key:value arguments (for csrf)');
	for my $case (['no token', {}], ['a wrong token', { csrf => $wrong }], ['a token as a list', { csrf => [$token] }]) {
		my ($name, $params) = @$case;
		my $req = Slim::Control::Request->new(['anywhere', 'status'], $params);
		Plugins::LyrionAnywhere::Commands::status($req);
		my $r = $req->{results};
		is($req->{status}, 'done', "status, $name: answered");
		is_deeply([sort keys %$r], [sort qw(enabled running binary_ok uptime error)], "status, $name: only the public fields");
		is($r->{running}, 0, "status, $name: running");
		is($r->{binary_ok}, 1, "status, $name: binary_ok");
		is($r->{error}, 'gateway exited', "status, $name: the error without its details");
		my $all = join "\n", map { $_ // '' } values %$r;
		unlike($all, qr{/srv|4242|10\.0\.0\.10|anywhere\.db|second line}, "status, $name: no paths, pid, address or stderr");
	}
	my $req = Slim::Control::Request->new(['anywhere', 'status'], { csrf => $token });
	Plugins::LyrionAnywhere::Commands::status($req);
	is_deeply($req->{results}, $full, 'status with the token: everything the settings page shows');

	# The gist of each kind of error, never a path or a value someone set.
	my %gist = (
		'' => '',
		'gateway binary not found: /srv/x/anywhere-gateway' => 'gateway binary not found',
		'gateway binary is not executable: /srv/x' => 'gateway binary is not executable',
		'could not start gateway: Permission denied at /srv/x line 3.' => 'could not start gateway',
		'listen address must be IP:port' => 'listen address must be IP:port',
		'bad trusted proxy address: 10.0.0.10' => 'bad trusted proxy address',
		"gateway exited" => 'gateway exited',
		"two\nlines" => 'two',
		("x" x 500) => ("x" x 120),
	);
	for my $in (sort keys %gist) {
		is(Plugins::LyrionAnywhere::Commands::public_error($in), $gist{$in}, "public_error: " . substr($in, 0, 40));
	}
	is(Plugins::LyrionAnywhere::Commands::public_error(undef), '', 'public_error: undef');
	is(Plugins::LyrionAnywhere::Commands::public_error("ok\x00\x1b[31m: x"), 'ok[31m', 'public_error: control characters removed');
}

# Restart never starts a gateway that is turned off (it is stopped and stays stopped).
{
	my $prefs = preferences('plugin.lyrionanywhere');
	$prefs->set(enabled => 0);
	my $n = @Proc::Background::STARTED;
	my $req = Slim::Control::Request->new(['anywhere', 'restart'], { csrf => $token });
	Plugins::LyrionAnywhere::Commands::restart($req);
	is($req->{status}, 'done');
	is($req->{results}{started}, 0, 'turned off: not started');
	like($req->{results}{message}, qr/turned off/, 'and the reply says why');
	is(scalar @Proc::Background::STARTED, $n, 'nothing was launched');
	is(Plugins::LyrionAnywhere::Gateway->state->{running}, 0, 'the running gateway was stopped');
	ok(!grep({ $_->[1] == \&Plugins::LyrionAnywhere::Gateway::_launch || $_->[1] == \&Plugins::LyrionAnywhere::Gateway::_check } @Slim::Utils::Timers::TIMERS),
		'and no start or restart is pending');
	$prefs->set(enabled => 1);
	$req = Slim::Control::Request->new(['anywhere', 'restart'], { csrf => $token });
	Plugins::LyrionAnywhere::Commands::restart($req);
	is($req->{results}{started}, 1, 'turned on: restarted');
	is(scalar @Proc::Background::STARTED, $n + 1, 'one gateway launched');
	is(Plugins::LyrionAnywhere::Gateway->state->{running}, 1);
	Plugins::LyrionAnywhere::Gateway->stop;
	start_stub_gateway($dir);
}

# Regression: `anywhere session_days days:N` always failed (days was never read).
{
	my $req = Slim::Control::Request->new(['anywhere', 'session_days'], { days => 90, csrf => $token });
	Plugins::LyrionAnywhere::Commands::admin($req);
	is($req->{status}, 'processing', 'session_days: accepted');
	ok(!defined $req->{results}{error}, 'session_days: no error') or diag $req->{results}{error};
	my $sent = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
	is($sent->{url}, 'http://127.0.0.1:9445/admin/v1/settings');
	is($sent->{body}, '{"session_days":90}', 'the lifetime reaches the gateway');
	Slim::Networking::SimpleAsyncHTTP::respond($sent, '{"session_days":90}');
	is($req->{results}{ok}, 1);
	$req = Slim::Control::Request->new(['anywhere', 'session_days'], { days => 0, csrf => $token });
	Plugins::LyrionAnywhere::Commands::admin($req);
	like($req->{results}{error}, qr/1-365/, 'session_days: out of range refused');
}

# Admin-only plugins: the list goes to the gateway as a JSON array; clearing sends [].
{
	my $n = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
	my $req = Slim::Control::Request->new(['anywhere', 'plugins_admin_only'], { commands => 'audiomuseai,selectVirtualLibrary', csrf => $token });
	Plugins::LyrionAnywhere::Commands::admin($req);
	my $sent = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[$n];
	ok($sent, 'plugins_admin_only sent');
	like($sent->{url}, qr{/admin/v1/plugins\z}, 'to the plugins endpoint');
	like($sent->{body}, qr/"admin_only":\["audiomuseai","selectVirtualLibrary"\]/, 'as a list');
	$req = Slim::Control::Request->new(['anywhere', 'plugins_admin_only'], { csrf => $token });
	Plugins::LyrionAnywhere::Commands::admin($req);
	like($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1]{body}, qr/"admin_only":\[\]/, 'no commands: none admin-only');
}

# Downloads: enabled and formats reach the gateway (and only with the token, above).
{
	my $req = Slim::Control::Request->new(['anywhere', 'user_downloads'],
		{ username => 'alice', enabled => 0, formats => 'opus', csrf => $token });
	Plugins::LyrionAnywhere::Commands::admin($req);
	is($req->{status}, 'processing', 'user_downloads: accepted');
	my $sent = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
	is($sent->{url}, 'http://127.0.0.1:9445/admin/v1/users/alice/downloads');
	like($sent->{body}, qr/"enabled":false/, 'enabled is a JSON boolean');
	like($sent->{body}, qr/"formats":\["opus"\]/, 'formats as a list');
	my $n = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
	$req = Slim::Control::Request->new(['anywhere', 'user_downloads'],
		{ username => 'alice', enabled => 1, formats => "opus\n", csrf => $token });
	Plugins::LyrionAnywhere::Commands::admin($req);
	like($req->{results}{error} // '', qr/formats/, 'a bad format list is refused');
	is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $n, 'and not sent');
}

# --- Cloudflare Tunnel commands -------------------------------------------------------
{
	is($Slim::Control::Request::DISPATCH{"anywhere $_"}[2], 1, "$_ parses key:value arguments")
		for qw(tunnel_status tunnel_log tunnel_register tunnel_create);

	# The status and the log need the token too (the hostname and log lines are not public).
	for my $op (qw(tunnel_status tunnel_log)) {
		my $n = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
		my $req = Slim::Control::Request->new(['anywhere', $op], {});
		Plugins::LyrionAnywhere::Commands::admin($req);
		like($req->{results}{error} // '', qr/^refused/, "$op without the token: refused");
		is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $n, "$op without the token: nothing asked");
	}

	# A valid registration reaches the gateway with the right URL and body, and the token
	# appears in no result and no log line, whatever the answer.
	@Slim::Utils::Log::MESSAGES = ();
	my @reqs;
	my $req = Slim::Control::Request->new(['anywhere', 'tunnel_register'],
		{ token => $TUNNEL_TOKEN, hostname => 'Music.Example.com', csrf => $token });
	push @reqs, $req;
	Plugins::LyrionAnywhere::Commands::admin($req);
	is($req->{status}, 'processing', 'tunnel_register: sent');
	my $sent = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
	is($sent->{url}, 'http://127.0.0.1:9445/admin/v1/tunnel/register', 'tunnel_register: the URL');
	is_deeply(JSON::PP::decode_json($sent->{body}), { token => $TUNNEL_TOKEN, hostname => 'music.example.com' }, 'tunnel_register: the body');
	Slim::Networking::SimpleAsyncHTTP::respond($sent, '');
	is($req->{results}{ok}, 1, 'tunnel_register: 204 is ok');

	# 202 with {} (or nothing) is a success, as 204 is.
	for my $body ('{}', '') {
		$req = Slim::Control::Request->new(['anywhere', 'tunnel_create'],
			{ api_token => $API_TOKEN, hostname => 'music.example.com', replace_dns => 1, csrf => $token });
		push @reqs, $req;
		Plugins::LyrionAnywhere::Commands::admin($req);
		$sent = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
		is($sent->{url}, 'http://127.0.0.1:9445/admin/v1/tunnel/create', 'tunnel_create: the URL');
		like($sent->{body}, qr/"replace_dns":true/, 'tunnel_create: replace_dns a JSON boolean');
		Slim::Networking::SimpleAsyncHTTP::respond($sent, $body);
		is($req->{status}, 'done', "tunnel_create, 202 '$body': done");
		is($req->{results}{ok}, 1, "tunnel_create, 202 '$body': ok");
		ok(!defined $req->{results}{error}, "tunnel_create, 202 '$body': no error");
	}
	for my $op (qw(tunnel_install tunnel_update tunnel_quick)) {
		$req = Slim::Control::Request->new(['anywhere', $op], { csrf => $token });
		Plugins::LyrionAnywhere::Commands::admin($req);
		Slim::Networking::SimpleAsyncHTTP::respond($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], '{}');
		is($req->{results}{ok}, 1, "$op: 202 {} is ok");
	}

	# A refusal (409) becomes the gateway's sentence.
	$req = Slim::Control::Request->new(['anywhere', 'tunnel_start'], { csrf => $token });
	Plugins::LyrionAnywhere::Commands::admin($req);
	Slim::Networking::SimpleAsyncHTTP::fail($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], '409 Conflict', '{"error":"no tunnel is registered"}');
	is($req->{status}, 'done', 'tunnel_start, 409: done');
	is($req->{results}{error}, 'refused by the gateway: no tunnel is registered', "tunnel_start, 409: the gateway's sentence");
	ok(!defined $req->{results}{ok}, 'tunnel_start, 409: not ok');

	# A failure echoing the secrets back is masked too.
	# (Each command knows, and masks, its own secret.)
	for my $case (['tunnel_register', { token => $TUNNEL_TOKEN, hostname => 'music.example.com' }, $TUNNEL_TOKEN],
		['tunnel_create', { api_token => $API_TOKEN, hostname => 'music.example.com' }, $API_TOKEN]) {
		my ($op, $args, $secret) = @$case;
		$req = Slim::Control::Request->new(['anywhere', $op], { %$args, csrf => $token });
		push @reqs, $req;
		Plugins::LyrionAnywhere::Commands::admin($req);
		Slim::Networking::SimpleAsyncHTTP::fail($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], "409 Conflict $secret",
			qq({"error":"refused $secret"}));
		like($req->{results}{error}, qr/^refused by the gateway: refused \[hidden\]/, "$op: an error, with its secret masked");
	}

	# Bad input: refused at once, the value not repeated.
	for my $case (['tunnel_register', { token => "$TUNNEL_TOKEN\r\nX-Evil: 1", hostname => 'music.example.com' }, "$TUNNEL_TOKEN"],
		['tunnel_create', { api_token => "$API_TOKEN/../x", hostname => 'music.example.com' }, $API_TOKEN],
		['tunnel_create', { api_token => $API_TOKEN, hostname => 'music.example.com', replace_dns => 'yes' }, $API_TOKEN],
		['tunnel_register', { token => $TUNNEL_TOKEN, hostname => 'music.example.com:443' }, $TUNNEL_TOKEN]) {
		my ($op, $args, $secret) = @$case;
		my $n = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
		$req = Slim::Control::Request->new(['anywhere', $op], { %$args, csrf => $token });
		push @reqs, $req;
		Plugins::LyrionAnywhere::Commands::admin($req);
		is($req->{status}, 'done', "$op, bad input: answered at once");
		ok(length($req->{results}{error} // ''), "$op, bad input: an error");
		is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $n, "$op, bad input: nothing sent");
	}

	# Without the token: refused, and the secret not logged either.
	$req = Slim::Control::Request->new(['anywhere', 'tunnel_create'], { api_token => $API_TOKEN, hostname => 'music.example.com' });
	push @reqs, $req;
	Plugins::LyrionAnywhere::Commands::admin($req);
	like($req->{results}{error}, qr/^refused/, 'tunnel_create without the page token: refused');

	my $results = JSON::PP->new->canonical->allow_nonref->encode([map { $_->{results} } @reqs]);
	ok(index($results, $TUNNEL_TOKEN) < 0, 'the tunnel token is in no result');
	ok(index($results, $API_TOKEN) < 0, 'the API token is in no result');
	ok(@Slim::Utils::Log::MESSAGES, 'something was logged');
	my $logged = join "\n", map { $_->[1] // '' } @Slim::Utils::Log::MESSAGES;
	ok(index($logged, $TUNNEL_TOKEN) < 0, 'the tunnel token is not logged');
	ok(index($logged, $API_TOKEN) < 0, 'the API token is not logged');
}

done_testing;
