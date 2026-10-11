# LMS commands: registration, validation before sending, async results.
use strict; use warnings;
use Test::More;
use File::Temp qw(tempdir);
use Plugins::LyrionAnywhere::Commands;
use Slim::Control::Request;
use Slim::Networking::SimpleAsyncHTTP;
use Slim::Utils::Prefs;

Plugins::LyrionAnywhere::Commands::register();
ok($Slim::Control::Request::DISPATCH{'anywhere status'}, 'status registered');
ok($Slim::Control::Request::DISPATCH{"anywhere $_"}, "$_ registered")
	for qw(users user_create user_delete grant ungrant revoke device_rename audit totp_reset pairing diagnostics settings session_days plugins plugins_admin_only user_downloads);
is($Slim::Control::Request::DISPATCH{'anywhere users'}[0], 0, 'no player needed');

my $dir = tempdir(CLEANUP => 1);
$Slim::Utils::Prefs::DIR = $dir;
mkdir "$dir/lyrion-anywhere";
open my $fh, '>', "$dir/lyrion-anywhere/admin.secret" or die; print $fh 'cd' x 32; close $fh;

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
);
my %read_only_args = (
	users => {}, grants => { username => 'alice' }, devices => {}, audit => {},
	diagnostics => { id => 1 }, settings => {}, plugins => {},
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

done_testing;
