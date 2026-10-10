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
	for qw(users user_create user_delete grant ungrant revoke audit totp_reset pairing);
is($Slim::Control::Request::DISPATCH{'anywhere users'}[0], 0, 'no player needed');

my $dir = tempdir(CLEANUP => 1);
$Slim::Utils::Prefs::DIR = $dir;
mkdir "$dir/lyrion-anywhere";
open my $fh, '>', "$dir/lyrion-anywhere/admin.secret" or die; print $fh 'cd' x 32; close $fh;

# Invalid input: an error result, and nothing reaches the gateway.
my $before = @Slim::Networking::SimpleAsyncHTTP::REQUESTS;
my $r = Slim::Control::Request->new(['anywhere', 'user_delete'], { username => '../x' });
Plugins::LyrionAnywhere::Commands::admin($r);
is($r->{status}, 'done');
like($r->{results}{error}, qr/usernames/);
is(scalar @Slim::Networking::SimpleAsyncHTTP::REQUESTS, $before, 'nothing sent');

# Valid input: processing until the gateway answers, then done with data.
$r = Slim::Control::Request->new(['anywhere', 'pairing'], { username => 'alice' });
Plugins::LyrionAnywhere::Commands::admin($r);
is($r->{status}, 'processing');
my $sent = $Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1];
is($sent->{url}, 'http://127.0.0.1:9445/admin/v1/users/alice/pairing');
Slim::Networking::SimpleAsyncHTTP::respond($sent, '{"code":"ABCDE23456","expires_in":600}');
is($r->{status}, 'done');
is($r->{results}{ok}, 1);
is($r->{results}{data}{code}, 'ABCDE23456');

# Gateway errors become an error result, not a crash.
$r = Slim::Control::Request->new(['anywhere', 'users'], {});
Plugins::LyrionAnywhere::Commands::admin($r);
Slim::Networking::SimpleAsyncHTTP::fail($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1], '403 Forbidden');
is($r->{results}{error}, 'the gateway refused the admin secret');

# Unexpected parameters are ignored, not forwarded.
$r = Slim::Control::Request->new(['anywhere', 'users'], { evil => 'x', username => 'alice' });
Plugins::LyrionAnywhere::Commands::admin($r);
is($Slim::Networking::SimpleAsyncHTTP::REQUESTS[-1]{url}, 'http://127.0.0.1:9445/admin/v1/users');

done_testing;
