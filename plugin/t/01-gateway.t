# Gateway supervisor: platform detection, binary selection, command line, backoff.
use strict; use warnings;
use Test::More;
use File::Temp ();
use Plugins::LyrionAnywhere::Gateway;
my $G = 'Plugins::LyrionAnywhere::Gateway';
sub show { my $s = $_[0] // 'undef'; $s =~ s/([\x00-\x1f])/sprintf('\\x%02x', ord $1)/ge; $s }

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

# Downloads: transcoding options.
{
	my $cl = sub { my ($a, $e) = Plugins::LyrionAnywhere::Gateway::command_line(%ok, @_); $a };
	my $tail = sub { my $a = shift; [ @$a[ scalar(@$argv) .. $#$a ] ] };
	is_deeply($tail->($cl->()), [], 'defaults: no transcoding flags (the gateway looks for ffmpeg on PATH)');
	is_deeply($tail->($cl->(transcode => 0)), ['--no-transcode'], 'transcoding off');
	is_deeply($tail->($cl->(transcode => 0, ffmpeg => '/usr/bin/ffmpeg')), ['--no-transcode'],
		'transcoding off: no ffmpeg path passed (the flags conflict)');
	is_deeply($tail->($cl->(ffmpeg => '/usr/bin/ffmpeg', download_cache_mb => 4096)),
		['--ffmpeg', '/usr/bin/ffmpeg', '--download-cache-mb', 4096], 'ffmpeg path and cache size');
	is_deeply($tail->($cl->(download_cache_mb => '')), [], 'empty cache size: the default');
	is_deeply($tail->($cl->(client_ip_header => 'CF-Connecting-IP')),
		['--client-ip-header', 'CF-Connecting-IP'], 'CDN client address header');
	is_deeply($tail->($cl->(client_ip_header => '')), [], 'no CDN header by default');
	is_deeply($tail->($cl->(analysis => 0)), ['--no-analysis'], 'track analysis off');
	is_deeply($tail->($cl->(analysis => 1)), [], 'track analysis on by default');
	is_deeply($tail->($cl->(analysis => 0, transcode => 0)), ['--no-transcode', '--no-analysis'],
		'analysis and transcoding off together');
}

my %bad = (
	'relative ffmpeg'             => { ffmpeg => 'bin/ffmpeg' },
	'ffmpeg with a newline'       => { ffmpeg => "/usr/bin/ffmpeg\n--no-transcode" },
	'cache too small'             => { download_cache_mb => 15 },
	'cache too big'               => { download_cache_mb => 1048577 },
	'cache not a number'          => { download_cache_mb => '2048; rm -rf /' },
	'cache with a newline'        => { download_cache_mb => "2048\n" },
	'cache in exponent form'      => { download_cache_mb => '2e3' },
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
	'client header with a space'  => { client_ip_header => 'CF Connecting IP' },
	'client header injection'     => { client_ip_header => "CF-Connecting-IP\r\n--trusted-proxy" },
	'client header reserved'      => { client_ip_header => 'x-forwarded-for' },
	'client header as option'     => { client_ip_header => ['--x'] },
);
for my $name (sort keys %bad) {
	my ($a, $e) = Plugins::LyrionAnywhere::Gateway::command_line(%ok, %{ $bad{$name} });
	ok(!$a && $e, "refused: $name");
}

is(Plugins::LyrionAnywhere::Gateway::backoff($_->[0]), $_->[1], "backoff($_->[0])")
	for ([1, 5], [2, 10], [3, 20], [4, 40], [5, 80], [6, 160], [7, 300], [9, 300], [100, 300], [0, 5]);

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

# --- strict IP parsing (regression: anything of hex digits and colons used to pass) ---
for my $good ('10.0.0.10', '0.0.0.0', '255.255.255.255', '::1', '::', 'fe80::1', '2001:db8::10',
              '2001:0db8:0000:0000:0000:0000:0000:0001', '::ffff:10.0.0.1') {
	ok(Plugins::LyrionAnywhere::Gateway::valid_ip($good), "valid IP: $good");
}
for my $bad (':::', '1:2', ':', '1::2::3', '12345::1', 'g::1', '1:2:3:4:5:6:7:8:9', 'fe80::1%eth0',
             '::ffff:10.0.0.01', '010.0.0.1', '10.0.0', '10.0.0.1.2', '1.2.3.4 ', ' 1.2.3.4', '', '...',
             ('1:' x 20) . '1', "::1\0", undef) {
	ok(!Plugins::LyrionAnywhere::Gateway::valid_ip($bad), 'refused IP: ' . show($bad));
}
ok(!Plugins::LyrionAnywhere::Gateway::valid_hostport('[:::]:9443'), 'refused: [:::]:port');
ok(!Plugins::LyrionAnywhere::Gateway::valid_hostport('[1:2]:9443'), 'refused: [1:2]:port');
ok(!Plugins::LyrionAnywhere::Gateway::is_loopback('127.0.0.1:99999'), 'refused: loopback with a bad port');
ok(!Plugins::LyrionAnywhere::Gateway::is_loopback('127.999.0.1:9445'), 'refused: loopback with a bad address');
ok(Plugins::LyrionAnywhere::Gateway::is_loopback('127.0.0.1:9445'), 'loopback');
{
	my ($a, $e) = Plugins::LyrionAnywhere::Gateway::command_line(%ok, trusted_proxies => [':::']);
	ok(!$a, 'refused: malformed IPv6 trusted proxy');
}

# --- the admin API port: never anything but a number, so the secret stays on loopback ---
{
	require Slim::Utils::Prefs;
	my $prefs = Slim::Utils::Prefs::preferences('plugin.lyrionanywhere');
	my $listen = \&Plugins::LyrionAnywhere::Gateway::admin_listen;
	$prefs->set(admin_port => undef);
	is($listen->(), '127.0.0.1:9445', 'default admin port');
	$prefs->set(admin_port => 9500);
	is($listen->(), '127.0.0.1:9500', 'configured admin port');
	# Regression: "http://127.0.0.1:" . "1@evil.example.com:80" . "/admin/..." sends the
	# Authorization header (the admin secret) to evil.example.com.
	for my $bad ('1@evil.example.com:80', '80/x', '9445 ', "9445\n", '0', '65536', '-1', '1e3', 'abc', '9445#', '00000009445') {
		$prefs->set(admin_port => $bad);
		is($listen->(), '127.0.0.1:9445', "bad admin port falls back to the default: $bad");
	}
	$prefs->set(admin_port => 9445);
}

# --- the gateway log is rotated, keeping one old file ----------------------------------
{
	require File::Temp;
	my $dir = File::Temp::tempdir(CLEANUP => 1);
	my $log = "$dir/lyrion-anywhere.log";
	my $rotate = \&Plugins::LyrionAnywhere::Gateway::rotate_log;
	is($rotate->("$dir/none", 10), 0, 'no log: nothing to do');
	open my $fh, '>', $log or die; print $fh 'x' x 9; close $fh;
	is($rotate->($log, 10), 0, 'under the limit: left alone');
	open $fh, '>>', $log or die; print $fh "y\n"; close $fh;
	is($rotate->($log, 10), 1, 'at the limit: rotated');
	is(-s $log, 0, 'the log is emptied in place (the gateway appends to the same file)');
	is(-s "$log.1", 11, 'the old lines are kept in .1');
	open $fh, '>', $log or die; print $fh 'z' x 20; close $fh;
	$rotate->($log, 10);
	is(-s "$log.1", 20, 'only one old file: the next rotation replaces it');
	ok(!-e "$log.2", 'no second old file');
	is(Plugins::LyrionAnywhere::Gateway::LOG_MAX_BYTES(), 5 * 1024 * 1024, 'rotated at 5 MiB');
	SKIP: {
		skip 'no symlinks', 1 unless eval { symlink "$dir/target", "$dir/link.log" };
		open $fh, '>', "$dir/target" or die; print $fh 'x' x 20; close $fh;
		is($rotate->("$dir/link.log", 10), 0, 'a symlinked log is not followed');
	}
}

# --- only a process that verifiably is our gateway counts as one -------------------------
{
	my $root = File::Temp::tempdir(CLEANUP => 1);
	local $Plugins::LyrionAnywhere::Gateway::PROC_ROOT = $root;
	my $data = '/srv/lms/prefs/lyrion-anywhere';
	my %cmd = (
		1001 => ['/opt/lms/Plugins/LyrionAnywhere/Bin/x86_64-linux/anywhere-gateway', 'serve', '--listen', '127.0.0.1:9443', '--data-dir', $data],
		1002 => ['/usr/bin/sleep', 'serve', '--data-dir', $data],
		1003 => ['/x/anywhere-gateway', 'serve', '--data-dir', '/srv/other'],
		1004 => ['/x/anywhere-gateway', 'admin', '--data-dir', $data],
		1005 => ['/x/anywhere-gateway', 'serve', '--listen', $data],
		1006 => ['/x/anywhere-gateway', 'serve', '--data-dir', "$data/.."],
		1007 => ['/x/anywhere-gateway.exe', 'serve', '--data-dir', $data],
		1008 => ['/x/not-anywhere-gateway', 'serve', '--data-dir', $data],
	);
	for my $pid (keys %cmd) {
		mkdir "$root/$pid";
		open my $fh, '>:raw', "$root/$pid/cmdline" or die;
		print $fh join("\0", @{ $cmd{$pid} }), "\0";
		close $fh;
	}
	my $ours = \&Plugins::LyrionAnywhere::Gateway::is_our_gateway;
	ok($ours->(1001, $data), 'the bundled gateway serving our data dir');
	ok($ours->(1007, $data), 'the Windows binary name too');
	ok(!$ours->($_, $data), "pid $_ ($cmd{$_}[0] $cmd{$_}[1]) is not ours") for 1002 .. 1006, 1008;
	ok(!$ours->(1009, $data), 'no such process');
	ok(!$ours->($_, $data), "bad pid '" . ($_ // 'undef') . "'") for (0, 1, -5, '1001/../1001', "1001\n", 'abc', undef);
	ok(!$ours->(1001, ''), 'no data dir: nothing matches');
}

# --- whether a process holds a loopback listening port (Linux /proc) ----------------------
SKIP: {
	skip 'needs /proc/net/tcp', 4 unless -r '/proc/net/tcp' && -d "/proc/$$/fd";
	require IO::Socket::INET;
	my $s = IO::Socket::INET->new(Listen => 1, LocalAddr => '127.0.0.1', LocalPort => 0, ReuseAddr => 1) or die;
	my $port = $s->sockport;
	my $owned = \&Plugins::LyrionAnywhere::Gateway::listener_owned_by;
	is($owned->($port, $$), 1, 'this process holds its own listening port');
	is($owned->($port, getppid()), 0, 'another process does not');
	close $s;
	is($owned->($port, $$), 0, 'nobody listening: not held');
	is($owned->('9445; x', $$), undef, 'a malformed port is not looked up');
}


# --- trusted networks: one --trusted-network flag per network, normalised -------------------
{
	my $flags = sub { my $a = shift; [ map { $a->[$_ + 1] } grep { $a->[$_] eq '--trusted-network' } 0 .. $#$a - 1 ] };
	my ($a, $e) = Plugins::LyrionAnywhere::Gateway::command_line(%ok, trusted_networks => ['10.8.0.5/24', 'FD12:3456:789A::/48', '100.101.102.103']);
	ok($a, 'trusted networks: a command line') or diag $e;
	is_deeply($flags->($a), ['10.8.0.0/24', 'fd12:3456:789a::/48', '100.101.102.103/32'], 'one --trusted-network per network, each its own argument');
	is(scalar(grep { $_ eq '--trusted-network' } @$a), 3, 'and no more');
	is_deeply($flags->($argv), [], 'none configured: no flag at all');
	($a) = Plugins::LyrionAnywhere::Gateway::command_line(%ok, trusted_networks => []);
	is_deeply($a, $argv, 'an empty list: the command line is exactly as before');
	for my $bad (['8.8.8.0/24'], ['10.8.0.0/24', '0.0.0.0/0'], ['127.0.0.1'], ['::/0'], ["10.8.0.0/24 --listen 0.0.0.0:1"], ['--listen'], ['']) {
		my ($x, $err) = Plugins::LyrionAnywhere::Gateway::command_line(%ok, trusted_networks => $bad);
		ok(!$x, 'refused: trusted networks ' . join(', ', map { show($_) } @$bad));
		like($err // '', qr/^bad trusted network: /, 'with the reason');
	}
}

# --- whether a process holds a listening port, read from a fake /proc ----------------------
{
	my $root = File::Temp::tempdir(CLEANUP => 1);
	local $Plugins::LyrionAnywhere::Gateway::PROC_ROOT = $root;
	my $owned = \&Plugins::LyrionAnywhere::Gateway::listener_owned_by;
	mkdir "$root/net";
	# Lines as Linux writes them: local address in the kernel's (little-endian) byte order.
	my $tcp = sub {
		my ($file, @rows) = @_;
		open my $fh, '>', "$root/net/$file" or die;
		print $fh "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n";
		my $i = 0;
		for my $r (@rows) {
			my ($addr, $port, $state, $inode) = @$r;
			printf $fh "  %d: %s:%04X %s:0000 %s 00000000:00000000 00:00000000 00000000  1000        0 %s 1 0000000000000000 100 0 0 10 0\n",
				$i++, $addr, $port, '0' x length($addr), $state, $inode;
		}
		close $fh;
	};
	my $pid = 4242;
	mkdir "$root/$pid";
	mkdir "$root/$pid/fd";
	symlink 'socket:[1001]', "$root/$pid/fd/3" or die;   # 127.0.0.1:9445
	symlink 'socket:[1002]', "$root/$pid/fd/4" or die;   # 10.8.0.1:9443
	symlink 'socket:[1003]', "$root/$pid/fd/5" or die;   # 0.0.0.0:9500
	symlink 'socket:[1004]', "$root/$pid/fd/6" or die;   # [fd00::1]:9443
	symlink '/dev/null', "$root/$pid/fd/0" or die;
	$tcp->('tcp',
		['0100007F', 9445, '0A', 1001],
		['0100080A', 9443, '0A', 1002],
		['00000000', 9500, '0A', 1003],
		['0100080A', 9600, '01', 1002],   # established, not listening
		['0100080A', 9700, '0A', 9999],   # someone else's
	);
	# fd00::1, each 32-bit word little-endian.
	$tcp->('tcp6', ['000000FD000000000000000001000000', 9443, '0A', 1004]);

	is($owned->(9445, $pid), 1, 'the admin port on 127.0.0.1 (the default address)');
	is($owned->(9445, $pid, '127.0.0.1'), 1, 'the same, asked for by address');
	is($owned->(9443, $pid), 0, 'a listener on another address is not the loopback one');
	is($owned->(9443, $pid, '10.8.0.1'), 1, 'the listen address on a VPN interface');
	is($owned->(9443, $pid, '10.8.0.2'), 0, 'not another address on it');
	is($owned->(9500, $pid, '0.0.0.0'), 1, 'every IPv4 address (0.0.0.0)');
	is($owned->(9500, $pid), 0, 'a listener on every address does not count as the loopback one');
	is($owned->(9500, $pid, '10.8.0.1'), 0, 'nor as a particular address');
	is($owned->(9600, $pid, '10.8.0.1'), 0, 'a connection that is not listening does not count');
	is($owned->(9700, $pid, '10.8.0.1'), 0, 'a socket held by another process does not count');
	is($owned->(9443, $pid, 'fd00::1'), 1, 'an IPv6 listen address');
	is($owned->(9443, $pid, 'FD00:0:0:0:0:0:0:1'), 1, 'written another way');
	is($owned->(9443, $pid, 'fd00::2'), 0, 'not another IPv6 address');
	is($owned->(9443, 4343, '10.8.0.1'), 0, 'no such process');
	for my $bad ('localhost', '10.8.0.1:9443', '[fd00::1]', 'fe80::1%eth0', '010.8.0.1', '', "10.8.0.1\n") {
		is($owned->(9443, $pid, $bad), 0, 'not a literal address, never held: ' . show($bad));
	}
	is($owned->('9443x', $pid, '10.8.0.1'), undef, 'a malformed port is not looked up');
	# The kernel's other byte order (big-endian systems) is read too.
	$tcp->('tcp', ['0A080001', 9443, '0A', 1002], ['7F000001', 9445, '0A', 1001]);
	is($owned->(9443, $pid, '10.8.0.1'), 1, 'big-endian: the VPN address');
	is($owned->(9445, $pid), 1, 'big-endian: loopback');
	unlink "$root/net/tcp6";
	is($owned->(9443, $pid, 'fd00::1'), undef, 'no tcp6 table: cannot tell');
}
# --- pref validators: the same rules as the settings page ------------------------------
{
	my $v = \&Plugins::LyrionAnywhere::Gateway::valid_pref;
	my $exe = File::Temp->new;
	chmod 0755, $exe->filename;
	my $plain = File::Temp->new;
	chmod 0644, $plain->filename;
	my $tmpdir = File::Temp::tempdir(CLEANUP => 1);
	# ffmpeg candidates: only an executable file actually named ffmpeg is accepted.
	my $ffdir = File::Temp::tempdir(CLEANUP => 1);
	my $mk = sub { my ($name, $mode) = @_; my $p = "$ffdir/$name"; open my $fh, '>', $p or die; close $fh; chmod $mode, $p; $p };
	my $ffmpeg       = $mk->('ffmpeg', 0755);
	my $ffmpeg_exe   = $mk->('ffmpeg.exe', 0755);
	$mk->('ffmpeg-evil', 0755);
	$mk->('sh', 0755);
	mkdir "$ffdir/plain";
	my $plain_ffmpeg = do { my $p = "$ffdir/plain/ffmpeg"; open my $fh, '>', $p or die; close $fh; chmod 0644, $p; $p };
	my %cases = (
		listen          => [['127.0.0.1:9443', '[::1]:9443', '10.0.0.20:9443'],
		                    ['', 'localhost:9443', '0.0.0.0', "127.0.0.1:9443\n", '[:::]:1', '1.2.3.4:0']],
		public_url      => [['', 'https://music.example.com', 'http://music.example.com:8443'],
		                    ['javascript:alert(1)', 'https://music.example.com/path', "https://music.example.com\n"]],
		trusted_proxies => [['', '10.0.0.10', '10.0.0.10, 10.0.0.11', '::1 10.0.0.1'],
		                    ['proxy.example', '10.0.0.1, evil', "10.0.0.1\n10.0.0.2", ':::']],
		trusted_networks => [['', '10.8.0.0/24', '10.8.0.0/24, fd00::/8', '10.8.0.5/24 100.64.0.0/10', 'fe80::/64'],
		                    ['8.8.8.0/24', '10.8.0.0/24, evil', "10.8.0.0/24\n10.9.0.0/24", '0.0.0.0/0', '::/0', '127.0.0.1',
		                     '::ffff:10.0.0.0/104', '10.8.0.0/33', 'localhost', ['10.8.0.0/24'], "10.8.0.0/24\x00"]],
		admin_port      => [['9445', 1, 65535],
		                    ['1@evil.example.com:80', '', 0, 65536, '9445x', undef]],
		gateway_path    => [['', $exe->filename],
		                    ['relative/gw', '/nonexistent/gw', $plain->filename, $tmpdir, $exe->filename . "\n"]],
		ffmpeg_path     => [['', $ffmpeg, $ffmpeg_exe],
		                    ['ffmpeg', 'bin/ffmpeg', $exe->filename, $plain_ffmpeg, "$ffdir/nonexistent/ffmpeg",
		                     $ffmpeg . "\n", "$ffdir/ffmpeg-evil", "$ffdir/sh", ['x']]],
		download_cache_mb => [['', 16, 2048, '1048576'],
		                    [0, 15, 1048577, '2048 ', "2048\n", '-1', '1e3', '2048MB', 'abc']],
		transcode       => [['', 0, 1, '0', '1'], ['2', 'yes', "1\n", ['1']]],
		analysis        => [['', 0, 1, '0', '1'], ['2', 'no', "0\n", '--no-analysis', ['0'], {}]],
	);
	for my $name (sort keys %cases) {
		my ($good, $bad) = @{ $cases{$name} };
		ok($v->($name, $_),  "pref $name accepts " . show($_)) for @$good;
		ok(!$v->($name, $_), "pref $name refuses " . show($_)) for @$bad;
	}
	ok($v->('enabled', 1), 'prefs without a rule are accepted');
}

done_testing;
