# Gateway supervisor: platform detection, binary selection, command line, backoff.
use strict; use warnings;
use Test::More;
use File::Temp ();
use Plugins::LyrionAnywhere::Gateway;
my $G = 'Plugins::LyrionAnywhere::Gateway';

is(Plugins::LyrionAnywhere::Gateway::platform('linux', 'x86_64-linux-thread-multi'), 'x86_64-linux');
is(Plugins::LyrionAnywhere::Gateway::platform('linux', 'aarch64-linux-gnu'), 'aarch64-linux');
is(Plugins::LyrionAnywhere::Gateway::platform('linux', 'arm-linux-gnueabihf'), 'armhf-linux');
is(Plugins::LyrionAnywhere::Gateway::platform('darwin', 'darwin-thread-multi-2level'), undef, 'unknown CPU is not guessed');
is(Plugins::LyrionAnywhere::Gateway::platform('MSWin32', 'MSWin32-x64-multi-thread'), undef);
is(Plugins::LyrionAnywhere::Gateway::platform('MSWin32', 'x86_64-windows'), 'x86_64-windows');
is(Plugins::LyrionAnywhere::Gateway::platform('freebsd', 'amd64-freebsd'), undef);

is(Plugins::LyrionAnywhere::Gateway::binary_path(basedir => '/p', os => 'linux', arch => 'x86_64-linux'),
   '/p/Bin/x86_64-linux/anywhere-gateway');
is(Plugins::LyrionAnywhere::Gateway::binary_path(basedir => '/p', os => 'MSWin32', arch => 'x86_64-windows'),
   '/p/Bin/x86_64-windows/anywhere-gateway.exe');
is(Plugins::LyrionAnywhere::Gateway::binary_path(basedir => '/p', override => '/opt/gw'), '/opt/gw');
is(Plugins::LyrionAnywhere::Gateway::binary_path(basedir => '/p', override => 'relative/gw'), undef, 'relative override refused');

my %ok = (
	binary => '/p/gw', listen => '127.0.0.1:9443', admin_listen => '127.0.0.1:9445',
	data_dir => '/var/lib/lms/prefs/lyrion-anywhere', http_port => 9000, cli_port => 9090,
	log_file => '/var/log/lms/lyrion-anywhere.log',
	trusted_proxies => ['10.0.0.10'], allowed_origins => ['https://music.example.com'],
);
my ($argv, $err) = Plugins::LyrionAnywhere::Gateway::command_line(%ok);
ok($argv, 'valid options give a command line') or diag $err;
is_deeply($argv, [
	'/p/gw', 'serve', '--listen', '127.0.0.1:9443', '--admin-listen', '127.0.0.1:9445',
	'--data-dir', '/var/lib/lms/prefs/lyrion-anywhere', '--lms', '127.0.0.1',
	'--lms-http-port', 9000, '--lms-cli-port', 9090,
	'--log-file', '/var/log/lms/lyrion-anywhere.log',
	'--trusted-proxy', '10.0.0.10', '--allowed-origin', 'https://music.example.com',
], 'argument list (each value its own argument: no shell)');

{
	my ($a) = Plugins::LyrionAnywhere::Gateway::command_line(%ok, static_dir => '/p/web');
	my ($i) = grep { $a->[$_] eq '--static-dir' } 0 .. $#$a;
	ok(defined $i && $a->[$i + 1] eq '/p/web', 'web player directory passed when bundled')
		or diag explain $a;
	ok(!grep({ $_ eq '--static-dir' } @$argv), 'no web directory flag when none is bundled');
}

{
	my $base = File::Temp::tempdir(CLEANUP => 1);
	is(Plugins::LyrionAnywhere::Gateway::web_dir($base), undef, 'no web directory: nothing served');
	mkdir "$base/web";
	is(Plugins::LyrionAnywhere::Gateway::web_dir($base), undef, 'empty web directory: nothing served');
	open my $fh, '>', "$base/web/index.html" or die; close $fh;
	is(Plugins::LyrionAnywhere::Gateway::web_dir($base), "$base/web", 'bundled player found');
	is(Plugins::LyrionAnywhere::Gateway::web_dir(undef), undef, 'no plugin directory yet');
}

my %bad = (
	'relative web dir'            => { static_dir => 'web' },
	'admin API on all interfaces' => { admin_listen => '0.0.0.0:9445' },
	'admin API on a LAN address'  => { admin_listen => '10.0.0.5:9445' },
	'listen not IP:port'          => { listen => 'localhost:9443' },
	'listen with shell syntax'    => { listen => '127.0.0.1:9443;rm -rf /' },
	'listen port out of range'    => { listen => '127.0.0.1:70000' },
	'relative data dir'           => { data_dir => 'data' },
	'no binary'                   => { binary => undef },
	'bad proxy'                   => { trusted_proxies => ['10.0.0.10; touch /tmp/x'] },
	'proxy hostname'              => { trusted_proxies => ['proxy.example'] },
	'origin with path'            => { allowed_origins => ['https://music.example.com/x'] },
	'origin javascript:'          => { allowed_origins => ['javascript:alert(1)'] },
	'bad http port'               => { http_port => '90a0' },
);
for my $name (sort keys %bad) {
	my ($a, $e) = Plugins::LyrionAnywhere::Gateway::command_line(%ok, %{ $bad{$name} });
	ok(!$a && $e, "refused: $name");
}

is(Plugins::LyrionAnywhere::Gateway::backoff($_->[0]), $_->[1], "backoff($_->[0])")
	for ([1, 5], [2, 10], [3, 30], [4, 60], [9, 60], [0, 5]);

is_deeply([Plugins::LyrionAnywhere::Gateway::split_list(' 10.0.0.1, 10.0.0.2  10.0.0.3,,')],
	['10.0.0.1', '10.0.0.2', '10.0.0.3']);

ok(Plugins::LyrionAnywhere::Gateway::valid_hostport('[::1]:9443'));
ok(Plugins::LyrionAnywhere::Gateway::is_loopback('[::1]:9445'));
ok(!Plugins::LyrionAnywhere::Gateway::valid_ip('256.1.1.1'));

# Regression: Perl's $ also matches before a trailing newline, so these used to pass and
# would have reached the gateway's command line.
for my $bad ("10.0.0.1\n", "::1\n") {
	ok(!Plugins::LyrionAnywhere::Gateway::valid_ip($bad), 'refused: newline after an IP');
}
for my $bad ("127.0.0.1:9443\n", "[::1]:9443\n") {
	ok(!Plugins::LyrionAnywhere::Gateway::valid_hostport($bad), 'refused: newline after host:port');
	ok(!Plugins::LyrionAnywhere::Gateway::is_loopback($bad), 'refused: newline after loopback address');
}
ok(!Plugins::LyrionAnywhere::Gateway::valid_origin("https://music.example.com\n"), 'refused: newline after origin');
ok(Plugins::LyrionAnywhere::Gateway::valid_origin('https://music.example.com'), 'a plain origin is fine');

done_testing;
