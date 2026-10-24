# Status right after LMS loads the plugin, before the delayed gateway start: the bundled
# binary path must already be under the plugin directory (it was "/Bin/..." for 5 s).
use strict; use warnings;
BEGIN { package main; use constant WEBUI => 0; }
use Test::More;
use File::Temp qw(tempdir);
use Slim::Utils::Prefs;
use Slim::Utils::Timers;
use Plugins::LyrionAnywhere::Plugin;
use Plugins::LyrionAnywhere::Gateway;

my $dir = tempdir(CLEANUP => 1);
$Slim::Utils::Prefs::DIR = $dir;

my $plat = Plugins::LyrionAnywhere::Gateway::platform();
plan skip_all => 'no bundled binary for this platform' unless $plat;

Plugins::LyrionAnywhere::Plugin->initPlugin;
ok(scalar @Slim::Utils::Timers::TIMERS, 'gateway start is still pending');
ok($Slim::Utils::Prefs::NS::VALIDATORS{'plugin.lyrionanywhere'}{$_}, "validator registered for $_")
	for qw(listen public_url trusted_proxies trusted_networks client_ip_header admin_port gateway_path);
is(preferences('plugin.lyrionanywhere')->get('trusted_networks'), '', 'no trusted networks by default: plain HTTP only from loopback');
{
	my ($argv, $err) = Plugins::LyrionAnywhere::Gateway::command_line(Plugins::LyrionAnywhere::Gateway::_options(), binary => '/srv/gw');
	ok($argv, 'the default settings give a command line') or diag $err;
	ok(!grep({ $_ eq '--trusted-network' } @{ $argv || [] }), 'with no --trusted-network flag');
}
my $s = Plugins::LyrionAnywhere::Gateway->state;
like($s->{binary}, qr{^/tmp/anywhere-test-plugin/Bin/\Q$plat\E/anywhere-gateway(?:\.exe)?$},
	'binary path is under the plugin directory before the start timer runs');
is($s->{running}, 0, 'not running');
done_testing;
