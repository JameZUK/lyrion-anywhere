package Plugins::LyrionAnywhere::Gateway;

# Finds, starts and supervises the anywhere-gateway binary.
#
# Everything that decides *what* to run is a pure function (platform, binary_path,
# command_line, backoff) so it can be tested without LMS. The gateway is started directly
# (never through a shell), its admin API is bound to loopback only, and it is restarted
# with backoff if it exits.

use strict;
use warnings;

use Config;
use Digest::SHA;
use File::Spec::Functions qw(abs2rel catdir catfile file_name_is_absolute splitpath);
use File::Basename ();
use File::Copy ();
use Socket qw(inet_pton AF_INET AF_INET6);
use POSIX ();
use Time::HiRes ();

use Slim::Utils::Log;
use Slim::Utils::OSDetect;
use Slim::Utils::Prefs;
use Slim::Utils::Timers;

my $log   = logger('plugin.lyrionanywhere');
my $prefs = preferences('plugin.lyrionanywhere');

use constant CHECK_INTERVAL => 10;      # seconds between liveness checks
use constant STABLE_AFTER   => 300;     # a run this long resets the backoff
use constant ADMIN_PORT     => 9445;    # default loopback port of the admin API
use constant STDERR_FILE    => 'gateway.stderr';   # the gateway's stderr, in the data dir
use constant STDERR_LINES   => 5;       # how many of its last lines the status shows
use constant PID_FILE       => 'gateway.pid';      # the running gateway's pid, in the data dir
use constant BACKOFF_FIRST  => 5;       # first restart delay (seconds); doubles each time
use constant BACKOFF_MAX    => 300;     # ... up to this
use constant TERM_GRACE     => 5;       # seconds after SIGTERM before SIGKILL
use constant SHUTDOWN_WAIT  => 3;       # at most this long blocking LMS's own shutdown
use constant LOG_MAX_BYTES  => 5 * 1024 * 1024;    # the gateway log is rotated at this size

# The development "Gateway path" pref runs any binary as the LMS user, so it is honoured
# only when LMS itself was started with this variable set to 1.
use constant ALLOW_GATEWAY_PATH_ENV => 'ANYWHERE_ALLOW_GATEWAY_PATH';

my $proc;            # Proc::Background object while running
my $wanted = 0;      # should the gateway be running?
my $attempt = 0;     # consecutive failed starts
my $started_at = 0;
my $last_error = '';
my $basedir = '';
my $restart_pending = 0;   # a restart timer is set: never more than one at a time
my $warned_override = '';  # the ignored gateway path last warned about
my $launched_admin;        # the admin address the running gateway was started with
my @dying;                 # gateways sent SIGTERM and not yet reaped: { proc, pid, kill_at, killed }
my $orphan_tries = 0;      # signals sent to a gateway left behind by an earlier LMS

# Where process information comes from (Linux). Tests point it at a fake tree.
our $PROC_ROOT = '/proc';

sub _now { time() }        # the clock (tests replace it)

# --- pure helpers -------------------------------------------------------------------

# Directory name under Bin/ for this OS and CPU.
sub platform {
	my ($os, $arch) = @_;
	$os   //= $^O;
	$arch //= $Config{archname};
	my $cpu = $arch =~ /^(x86_64|amd64)/i ? 'x86_64'
	        : $arch =~ /^(aarch64|arm64)/i ? 'aarch64'
	        : $arch =~ /^arm/i             ? 'armhf'
	        : undef;
	return undef unless $cpu;
	return "$cpu-linux"   if $os eq 'linux';
	return "$cpu-darwin"  if $os eq 'darwin';
	return "$cpu-windows" if $os eq 'MSWin32' && $cpu eq 'x86_64';
	return undef;
}

# The binary to run: an explicit path from settings, else the bundled one.
sub binary_path {
	my (%o) = @_;
	if (my $p = $o{override}) {
		return file_name_is_absolute($p) ? $p : undef;
	}
	my $plat = platform($o{os}, $o{arch}) or return undef;
	my $exe  = ($o{os} // $^O) eq 'MSWin32' ? 'anywhere-gateway.exe' : 'anywhere-gateway';
	return catfile($o{basedir}, 'Bin', $plat, $exe);
}

# Strict: dotted-quad IPv4 without leading zeros (which some parsers read as octal), or
# IPv6 as the system's inet_pton parses it (no zone ids). Anything else is refused.
my $IPV4 = qr/^(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\z/;

sub valid_ip {
	my $ip = shift // return 0;
	return 0 if ref $ip || length($ip) > 45;
	if ($ip =~ /^[0-9.]+\z/) {
		return ($ip =~ $IPV4 && defined eval { inet_pton(AF_INET, $ip) }) ? 1 : 0;
	}
	if ($ip =~ /^[0-9a-f:.]+\z/i && $ip =~ /:/) {
		# An embedded IPv4 tail (::ffff:10.0.0.1) must itself be strict.
		if ($ip =~ /([^:]*\.[^:]*)\z/) { return 0 unless $1 =~ $IPV4 }
		return defined(eval { inet_pton(AF_INET6, $ip) }) ? 1 : 0;
	}
	return 0;
}

sub valid_hostport {
	my $hp = shift // return 0;
	my ($host, $port);
	if ($hp =~ /^\[([0-9a-f:.]+)\]:(\d{1,5})\z/i) { ($host, $port) = ($1, $2) }
	elsif ($hp =~ /^([0-9.]+):(\d{1,5})\z/)      { ($host, $port) = ($1, $2) }
	else { return 0 }
	return valid_ip($host) && $port >= 1 && $port <= 65535;
}

sub is_loopback {
	my $hp = shift // return 0;
	return 0 unless valid_hostport($hp);
	return $hp =~ /^127\.\d+\.\d+\.\d+:\d+\z/ || $hp =~ /^\[::1\]:\d+\z/;
}

sub valid_port {
	my $p = shift // return 0;
	return $p =~ /^\d{1,5}\z/ && $p >= 1 && $p <= 65535 ? 1 : 0;
}

# A comma/space separated list of proxy addresses (empty is fine).
sub valid_proxy_list {
	my $s = shift // '';
	return 0 if ref $s || $s =~ /[\r\n\x00]/;
	return (grep { !valid_ip($_) } split_list($s)) ? 0 : 1;
}

# A development gateway binary: an absolute path to an executable regular file (or empty).
sub valid_gateway_path {
	my $p = shift // '';
	return 1 if $p eq '';
	return 0 if ref $p || $p =~ /[\r\n\x00]/;
	return file_name_is_absolute($p) && -f $p && -x _ ? 1 : 0;
}

# ffmpeg for transcoded downloads (or empty: the gateway looks on PATH). The gateway runs
# this program, so it must be an absolute path to an executable regular file actually
# named ffmpeg: a changed pref can't make it run some other program.
sub valid_ffmpeg_path {
	my $p = shift // '';
	return 1 if $p eq '';
	return 0 if ref $p || $p =~ /[\r\n\x00]/ || length($p) > 4096;
	return 0 unless file_name_is_absolute($p);
	my (undef, undef, $name) = splitpath($p);
	return 0 unless $name =~ /^ffmpeg(?:\.exe)?\z/i;
	return -f $p && -x _ ? 1 : 0;
}

# Disk space for transcoded downloads, in MiB (or empty: the default).
sub valid_cache_mb {
	my $v = shift // '';
	return 1 if $v eq '';
	return 0 if ref $v;
	return $v =~ /^\d{1,7}\z/ && $v >= 16 && $v <= 1048576 ? 1 : 0;
}

# A CDN's client-address header for --client-ip-header (e.g. CF-Connecting-IP), or empty
# for none. The gateway refuses headers with another meaning; so do we, to fail early.
my %RESERVED_HEADER = map { $_ => 1 }
	qw(host origin cookie authorization x-forwarded-for x-forwarded-proto x-anywhere-client forwarded);
sub valid_client_ip_header {
	my $h = shift // '';
	return 0 if ref $h;
	return 1 if $h eq '';
	return $h =~ /^[A-Za-z0-9-]{1,64}\z/ && !$RESERVED_HEADER{lc $h} ? 1 : 0;
}

sub valid_origin {
	my $o = shift // return 0;
	return $o =~ m{^https?://[a-z0-9.-]+(?::\d{1,5})?\z}i && length($o) <= 255;
}

# The gateway's argument list. Returns (\@argv) or (undef, $error).
sub command_line {
	my (%o) = @_;
	return (undef, 'gateway binary not found for this platform') unless $o{binary};
	return (undef, 'listen address must be IP:port')              unless valid_hostport($o{listen});
	return (undef, 'admin API must listen on loopback')            unless is_loopback($o{admin_listen});
	return (undef, 'data directory must be an absolute path')      unless $o{data_dir} && file_name_is_absolute($o{data_dir});
	return (undef, 'web directory must be an absolute path')       if $o{static_dir} && !file_name_is_absolute($o{static_dir});
	for my $p (qw(http_port cli_port)) {
		return (undef, "bad $p") unless valid_port($o{$p});
	}
	my @proxies = @{ $o{trusted_proxies} || [] };
	my @origins = @{ $o{allowed_origins} || [] };
	for (@proxies) { return (undef, "bad trusted proxy address: $_") unless valid_ip($_) }
	for (@origins) { return (undef, "bad public URL: $_") unless valid_origin($_) }
	my $ip_header = $o{client_ip_header} // '';
	return (undef, 'client address header must be a header name such as CF-Connecting-IP')
		unless valid_client_ip_header($ip_header);
	my $transcode = $o{transcode} // 1;
	my $analysis = $o{analysis} // 1;
	my $ffmpeg = $o{ffmpeg} // '';
	return (undef, 'ffmpeg must be an absolute path')
		if $ffmpeg ne '' && (ref $ffmpeg || $ffmpeg =~ /[\r\n\x00]/ || !file_name_is_absolute($ffmpeg));
	return (undef, 'download cache size must be 16-1048576 MiB')
		unless valid_cache_mb($o{download_cache_mb});

	my @argv = (
		$o{binary}, 'serve',
		'--listen',        $o{listen},
		'--admin-listen',  $o{admin_listen},
		'--data-dir',      $o{data_dir},
		'--lms',           '127.0.0.1',
		'--lms-http-port', $o{http_port},
		'--lms-cli-port',  $o{cli_port},
	);
	push @argv, '--log-file', $o{log_file} if $o{log_file};
	push @argv, '--static-dir', $o{static_dir} if $o{static_dir};
	push @argv, map { ('--trusted-proxy', $_) } @proxies;
	push @argv, map { ('--allowed-origin', $_) } @origins;
	push @argv, '--client-ip-header', $ip_header if $ip_header ne '';
	if (!$transcode)         { push @argv, '--no-transcode' }
	elsif ($ffmpeg ne '')    { push @argv, '--ffmpeg', $ffmpeg }
	push @argv, '--download-cache-mb', 0 + $o{download_cache_mb}
		if defined $o{download_cache_mb} && $o{download_cache_mb} ne '';
	push @argv, '--no-analysis' unless $analysis;
	return (\@argv);
}

# Seconds to wait before restart attempt $n (1-based): 5, 10, 20, 40 ... capped at 300.
sub backoff {
	my $n = shift // 1;
	$n = 1 if $n < 1;
	return BACKOFF_MAX if $n > 16;
	my $wait = BACKOFF_FIRST * 2**($n - 1);
	return $wait > BACKOFF_MAX ? BACKOFF_MAX : $wait;
}

# Copy-and-truncate the gateway log once it reaches $max bytes, keeping one old file
# ($file.1). The gateway opens its log for appending, so it carries on writing at the start
# of the emptied file. Returns 1 if the log was rotated.
sub rotate_log {
	my ($file, $max) = @_;
	$max //= LOG_MAX_BYTES;
	return 0 unless $file && !-l $file && -f $file;
	my $size = -s _;
	return 0 unless $size && $size >= $max;
	File::Copy::copy($file, "$file.1") or return 0;
	truncate($file, 0) or return 0;
	return 1;
}

# Is $pid an anywhere-gateway serving $dir? Read from /proc/<pid>/cmdline, so it is only
# ever true where that exists (Linux) and the process's command line says exactly that: a
# binary named anywhere-gateway, run as `serve`, with our data directory. Anything else, or
# anything unreadable, is not ours.
sub is_our_gateway {
	my ($pid, $dir) = @_;
	return 0 unless defined $pid && $pid =~ /^[1-9]\d{0,9}\z/ && $pid > 1 && $dir;
	open my $fh, '<:raw', "$PROC_ROOT/$pid/cmdline" or return 0;
	my $cmd = do { local $/; <$fh> } // '';
	close $fh;
	my @argv = split /\0/, $cmd;
	return 0 unless @argv >= 4;
	return 0 unless File::Basename::basename($argv[0]) =~ /^anywhere-gateway(?:\.exe)?\z/;
	return 0 unless $argv[1] eq 'serve';
	for my $i (2 .. $#argv - 1) {
		return 1 if $argv[$i] eq '--data-dir' && $argv[$i + 1] eq $dir;
	}
	return 0;
}

# Does process $pid hold the loopback listening socket on $port? 1 yes, 0 no, undef when this
# system can't tell (no /proc). Read from /proc/net/tcp and /proc/<pid>/fd.
sub listener_owned_by {
	my ($port, $pid) = @_;
	return undef unless $pid && valid_port($port);
	open my $tcp, '<', "$PROC_ROOT/net/tcp" or return undef;
	my %inodes;
	while (my $line = <$tcp>) {
		my @f = split ' ', $line;
		next unless @f >= 10 && $f[1] =~ /^([0-9A-F]{8}):([0-9A-F]{4})\z/;
		my ($addr, $p) = ($1, hex $2);
		next unless $p == $port && $f[3] eq '0A';               # LISTEN
		next unless $addr eq '0100007F' || $addr eq '7F000001';  # 127.0.0.1, either byte order
		$inodes{ $f[9] } = 1 if $f[9] =~ /^[1-9]\d*\z/;
	}
	close $tcp;
	return 0 unless %inodes;
	return 0 unless -d "$PROC_ROOT/$pid";
	opendir my $dh, "$PROC_ROOT/$pid/fd" or return undef;
	my $found = 0;
	while (defined(my $fd = readdir $dh)) {
		my $link = readlink("$PROC_ROOT/$pid/fd/$fd") // next;
		if ($link =~ /^socket:\[(\d+)\]\z/ && $inodes{$1}) { $found = 1; last }
	}
	closedir $dh;
	return $found;
}

# Check a bundled binary against Bin/SHA256SUMS (written by tools/build-release.sh), so a
# corrupt, truncated or accidentally replaced binary is never run. This is an integrity
# check, not an authenticity one: the list ships in the same zip, so it cannot detect a
# maliciously altered release. Returns an error message, or undef if the binary matches.
# Binaries outside Bin/ (the development "Gateway path") are not checked.
sub verify_binary {
	my ($bin, $base) = @_;
	return 'no gateway binary' unless $bin && -f $bin;
	my $bindir = catdir($base // '', 'Bin');
	my $rel = abs2rel($bin, $bindir);
	return undef if !$base || $rel =~ m{^\.\.} || file_name_is_absolute($rel);
	$rel =~ s{\\}{/}g;
	my $sums = catfile($bindir, 'SHA256SUMS');
	open my $fh, '<', $sums or return 'gateway checksum list missing (reinstall the plugin)';
	my $want;
	while (my $line = <$fh>) {
		next unless $line =~ /^([0-9a-f]{64})\s+\*?(\S+)\s*$/;
		$want = $1 if $2 eq $rel;
	}
	close $fh;
	return "no checksum for $rel (reinstall the plugin)" unless $want;
	my $got = eval { Digest::SHA->new(256)->addfile($bin, 'b')->hexdigest } // '';
	return "gateway binary $rel failed its checksum (reinstall the plugin)" unless $got eq $want;
	return undef;
}

# The browser player bundled with the plugin (release builds), if present.
sub web_dir {
	my $base = shift or return undef;
	my $dir = catdir($base, 'web');
	return -f catfile($dir, 'index.html') ? $dir : undef;
}

# Split a comma/space separated list.
sub split_list {
	my $s = shift // '';
	return grep { length } map { s/^\s+|\s+$//gr } split /[,\s]+/, $s;
}

# --- LMS integration ----------------------------------------------------------------

sub data_dir {
	return catdir(Slim::Utils::Prefs::dir(), 'lyrion-anywhere');
}

# The admin API address to start the gateway with. Only a plain port number is ever used.
# The admin secret is never sent to this directly: see admin_target().
sub admin_listen {
	my $port = $prefs->get('admin_port');
	if (defined $port && $port ne '' && !valid_port($port)) {
		$log->warn('ignoring invalid admin_port pref; using ' . ADMIN_PORT);
		$port = undef;
	}
	return '127.0.0.1:' . ($port || ADMIN_PORT);
}

# The development gateway path, if it is set and allowed; otherwise undef (the bundled,
# checksum-verified binary is used).
sub gateway_override {
	my $p = $prefs->get('gateway_path');
	return undef unless defined $p && $p ne '';
	if (($ENV{+ALLOW_GATEWAY_PATH_ENV} // '') ne '1') {
		if ($warned_override ne $p) {
			$log->warn('ignoring the Gateway path setting: start LMS with '
				. ALLOW_GATEWAY_PATH_ENV . '=1 to use a development gateway');
			$warned_override = $p;
		}
		return undef;
	}
	return $p;
}

# Where to send the admin secret: the admin address the running gateway was started with,
# or undef if this plugin didn't start a gateway that is still running. `admin_port` is an
# ordinary pref that LMS's `pref` command can change, so it is read only when the gateway
# starts; a later change never redirects the secret. Where the system can tell (Linux), the
# port must also be held by the gateway process itself, not by some other local program.
sub admin_target {
	return undef unless $proc && $launched_admin && eval { $proc->alive };
	my $pid = eval { $proc->pid };
	my ($port) = $launched_admin =~ /:(\d+)\z/;
	my $owned = listener_owned_by($port, $pid);
	if (defined $owned && !$owned) {
		# Just after a start the gateway may not be listening yet; later, this is worth a warning.
		my $msg = "admin port $port is not held by the gateway (pid $pid): not sending the admin secret";
		if ($started_at && _now() - $started_at < CHECK_INTERVAL) { $log->info($msg) }
		else                                                      { $log->warn($msg) }
		return undef;
	}
	return $launched_admin;
}

sub stderr_file { catfile(data_dir(), STDERR_FILE) }
sub pid_file    { catfile(data_dir(), PID_FILE) }

sub log_file {
	my ($logdir) = Slim::Utils::OSDetect::dirsFor('log');
	return $logdir ? catfile($logdir, 'lyrion-anywhere.log') : undef;
}

sub _write_pid_file {
	my $pid = shift // return;
	my $file = pid_file();
	my $tmp = "$file.tmp";
	unlink $tmp;
	my $fh;
	unless (sysopen($fh, $tmp, POSIX::O_WRONLY() | POSIX::O_CREAT() | POSIX::O_EXCL(), 0600)) {
		$log->warn("could not write $file: $!");
		return;
	}
	print $fh "$pid\n";
	close $fh;
	rename $tmp, $file or unlink $tmp;
}

sub _read_pid_file {
	open my $fh, '<', pid_file() or return undef;
	my $line = <$fh> // '';
	close $fh;
	return $line =~ /^([1-9]\d{0,9})\s*\z/ ? $1 : undef;
}

# Forget the pid file if it names $pid (or whatever it names, given no pid).
sub _remove_pid_file {
	my $pid = shift;
	my $in = _read_pid_file();
	unlink pid_file() if !defined $pid || !defined $in || $in == $pid;
}

# The last few lines the gateway wrote to stderr (control characters removed), or ''.
sub stderr_tail {
	my $file = shift // stderr_file();
	open my $fh, '<', $file or return '';
	my $size = -s $fh || 0;
	seek $fh, ($size > 4096 ? $size - 4096 : 0), 0;
	local $/;
	my $text = <$fh> // '';
	close $fh;
	my @lines = grep { /\S/ } split /\r?\n/, $text;
	@lines = @lines[-STDERR_LINES() .. -1] if @lines > STDERR_LINES;
	s/[\x00-\x08\x0b-\x1f\x7f]//g for @lines;
	return join "\n", @lines;
}

# Validators for every pref, so changes through LMS's `pref` command get the same checks
# as the settings page.
my %VALIDATE = (
	listen          => \&valid_hostport,
	public_url      => sub { my $v = shift // ''; $v eq '' || valid_origin($v) },
	trusted_proxies => \&valid_proxy_list,
	client_ip_header => \&valid_client_ip_header,
	admin_port     => \&valid_port,
	gateway_path    => \&valid_gateway_path,
	ffmpeg_path     => \&valid_ffmpeg_path,
	download_cache_mb => \&valid_cache_mb,
	transcode       => sub { my $v = shift // ''; !ref $v && $v =~ /^[01]?\z/ },
	analysis        => sub { my $v = shift // ''; !ref $v && $v =~ /^[01]?\z/ },
);

sub valid_pref {
	my ($name, $value) = @_;
	my $check = $VALIDATE{$name // ''} or return 1;
	return $check->($value) ? 1 : 0;
}

sub register_validators {
	for my $name (sort keys %VALIDATE) {
		my $check = $VALIDATE{$name};
		$prefs->setValidate({ validator => sub { my (undef, $new) = @_; $check->($new) ? 1 : 0 } }, $name);
	}
}

sub _options {
	my $server = preferences('server');
	my $cli    = preferences('plugin.cli');
	my $public = $prefs->get('public_url') || '';
	# Stored values are checked again: one that is no longer valid (e.g. ffmpeg removed) is
	# left out rather than stopping the gateway from starting.
	my $ffmpeg = $prefs->get('ffmpeg_path') // '';
	$ffmpeg = '' unless valid_ffmpeg_path($ffmpeg);
	my $cache = $prefs->get('download_cache_mb') // '';
	$cache = '' unless valid_cache_mb($cache);
	my $transcode = $prefs->get('transcode');
	my $analysis = $prefs->get('analysis');
	my $ip_header = $prefs->get('client_ip_header') // '';
	$ip_header = '' unless valid_client_ip_header($ip_header);
	return (
		client_ip_header  => $ip_header,
		ffmpeg            => $ffmpeg,
		transcode         => (defined $transcode && $transcode eq '0') ? 0 : 1,
		analysis          => (defined $analysis && $analysis eq '0') ? 0 : 1,
		download_cache_mb => $cache,
		binary          => binary_path(basedir => $basedir, override => gateway_override()),
		listen          => $prefs->get('listen') || '127.0.0.1:9443',
		admin_listen    => admin_listen(),
		data_dir        => data_dir(),
		http_port       => $server->get('httpport') || 9000,
		cli_port        => ($cli && $cli->get('cliport')) || 9090,
		log_file        => log_file(),
		static_dir      => web_dir($basedir),
		trusted_proxies => [ split_list($prefs->get('trusted_proxies')) ],
		allowed_origins => [ $public ? $public : () ],
	);
}

# Where the plugin lives (for the bundled binary). Set at load, so status is right at once.
sub set_basedir {
	my ($class, $dir) = @_;
	$basedir = $dir;
}

sub init {
	my ($class, $dir) = @_;
	$class->set_basedir($dir);
	$class->start if $prefs->get('enabled');
}

sub start {
	my $class = shift;
	$wanted = 1;
	_launch();
	Slim::Utils::Timers::killTimers(undef, \&_check);
	Slim::Utils::Timers::setTimer(undef, _now() + CHECK_INTERVAL, \&_check);
}

# Stop the gateway. This never blocks LMS: the gateway is sent SIGTERM and reaped from a
# timer, with SIGKILL if it hasn't gone TERM_GRACE seconds later.
sub stop {
	my $class = shift;
	$wanted = 0;
	Slim::Utils::Timers::killTimers(undef, \&_check);
	Slim::Utils::Timers::killTimers(undef, \&_launch);
	$restart_pending = 0;
	$launched_admin = undef;
	if ($proc) {
		$log->info('stopping gateway');
		_terminate($proc);
		undef $proc;
	}
}

# LMS is shutting down: stop, then wait (at most SHUTDOWN_WAIT seconds) for the gateway to
# go, so it isn't left running without LMS.
sub stop_for_shutdown {
	my $class = shift;
	$class->stop;
	my $until = Time::HiRes::time() + SHUTDOWN_WAIT;
	while (@dying && Time::HiRes::time() < $until) {
		Time::HiRes::sleep(0.1);
		@dying = grep {
			my $alive = eval { $_->{proc}->alive };
			_remove_pid_file($_->{pid}) unless $alive;
			$alive;
		} @dying;
	}
	_signal('KILL', $_->{pid}) for @dying;
	Slim::Utils::Timers::killTimers(undef, \&_reap);
}

# Restart, but only if remote access is turned on: a disabled gateway is stopped and stays
# stopped. Returns 1 if the gateway was started, 0 if not.
sub restart {
	my $class = shift;
	$class->stop;
	$attempt = 0;
	unless ($prefs->get('enabled')) {
		$log->info('not restarting the gateway: remote access is turned off');
		return 0;
	}
	$class->start;
	return 1;
}

sub _signal {
	my ($sig, $pid) = @_;
	return 0 unless defined $pid && $pid =~ /^[1-9]\d{0,9}\z/ && $pid > 1 && $pid != $$;
	return kill($sig, $pid) ? 1 : 0;
}

# Ask a running gateway to stop, without waiting for it.
sub _terminate {
	my $p = shift or return;
	return unless eval { $p->alive };
	my $pid = eval { $p->pid };
	if ($pid && _signal('TERM', $pid)) {
		push @dying, { proc => $p, pid => $pid, kill_at => _now() + TERM_GRACE, killed => 0 };
		_schedule_reap();
		return;
	}
	# No pid to signal: Proc::Background's own way (which can block).
	eval { $p->die };
}

sub _schedule_reap {
	Slim::Utils::Timers::killTimers(undef, \&_reap);
	Slim::Utils::Timers::setTimer(undef, Time::HiRes::time() + 1, \&_reap) if @dying;
}

# Collect stopped gateways (alive() reaps them); SIGKILL any that outstay TERM_GRACE.
sub _reap {
	my @left;
	for my $d (@dying) {
		if (!eval { $d->{proc}->alive }) {
			_remove_pid_file($d->{pid});
			next;
		}
		if (!$d->{killed} && _now() >= $d->{kill_at}) {
			$log->warn("gateway (pid $d->{pid}) did not stop; killing it");
			_signal('KILL', $d->{pid});
			$d->{killed} = 1;
		}
		push @left, $d;
	}
	@dying = @left;
	_schedule_reap();
}

# A gateway left running by an earlier LMS (one that crashed, so never stopped it) still
# holds the ports. Returns 1 if one was found and signalled (start again shortly), else 0.
# Only a process that verifiably is our gateway, serving our data directory, is signalled.
sub _stop_orphan {
	my $pid = _read_pid_file() or return 0;
	my $mine = $proc ? eval { $proc->pid } : undef;
	return 0 if (defined $mine && $mine == $pid) || grep { $_->{pid} == $pid } @dying;
	if ($pid == $$ || !kill(0, $pid)) {
		$orphan_tries = 0;
		_remove_pid_file($pid);
		return 0;
	}
	if (!is_our_gateway($pid, data_dir())) {
		# Some other process has that pid now (or this system has no /proc): leave it alone.
		$log->warn("the pid file names process $pid, which is not this plugin's gateway: leaving it alone");
		$orphan_tries = 0;
		_remove_pid_file($pid);
		return 0;
	}
	my $sig = $orphan_tries++ >= 3 ? 'KILL' : 'TERM';
	$log->warn("stopping a gateway left running by an earlier LMS (pid $pid, SIG$sig)");
	_signal($sig, $pid);
	return 1;
}

sub state {
	my %o = _options();
	return {
		enabled   => $prefs->get('enabled') ? 1 : 0,
		running   => ($proc && eval { $proc->alive }) ? 1 : 0,
		pid       => $proc ? eval { $proc->pid } : undef,
		binary    => $o{binary},
		binary_ok => ($o{binary} && -x $o{binary}) ? 1 : 0,
		data_dir  => $o{data_dir},
		listen    => $o{listen},
		error     => $last_error,
		uptime    => ($proc && $started_at) ? _now() - $started_at : 0,
	};
}

sub _launch {
	$restart_pending = 0;
	return unless $wanted;
	return if $proc && eval { $proc->alive };
	# Wait for an earlier gateway to let go of the ports: one this plugin is stopping, or one
	# an earlier LMS left behind.
	_reap() if @dying;
	if (@dying || _stop_orphan()) {
		$restart_pending = 1;
		Slim::Utils::Timers::setTimer(undef, _now() + 1, \&_launch);
		return;
	}
	$orphan_tries = 0;
	my %o = _options();
	my ($argv, $err) = command_line(%o);
	if (!$argv) {
		$last_error = $err;
		$log->error("cannot start gateway: $err");
		return;
	}
	if (!-f $argv->[0]) {
		$last_error = "gateway binary not found: $argv->[0]";
		$log->error($last_error);
		return;
	}
	if (!gateway_override()) {
		if (my $bad = verify_binary($argv->[0], $basedir)) {
			$last_error = $bad;
			$log->error($last_error);
			return;
		}
		# Zip extraction can drop the executable bit.
		chmod 0755, $argv->[0] unless -x $argv->[0];
	}
	if (!-x $argv->[0]) {
		$last_error = "gateway binary is not executable: $argv->[0]";
		$log->error($last_error);
		return;
	}
	mkdir $o{data_dir}, 0700 unless -d $o{data_dir};
	rotate_log($o{log_file});
	require Proc::Background;
	$proc = _spawn($argv, stderr_file());
	if ($proc) {
		$started_at = _now();
		$last_error = '';
		$launched_admin = $o{admin_listen};
		my $pid = eval { $proc->pid };
		_write_pid_file($pid) if $pid;
		$log->info('gateway started (pid ' . ($pid // '?') . ")");
	} else {
		$last_error = "could not start gateway: " . ($@ || 'unknown error');
		$log->error($last_error);
	}
}

# Start the gateway with its stderr going to a file, so the reason for an early exit (a
# bad flag, a port in use) can be shown in the status. The child inherits file descriptor
# 2; the plugin's own is put back straight away. This works on descriptors, not Perl
# handles: LMS ties STDERR to its log, and re-opening a tied handle dies. If anything here
# fails, the gateway starts anyway, without the capture.
sub _spawn {
	my ($argv, $errfile) = @_;
	my ($fd, $saved);
	if ($errfile) {
		eval {
			$fd = POSIX::open($errfile, POSIX::O_WRONLY() | POSIX::O_CREAT() | POSIX::O_TRUNC(), 0600);
			$saved = POSIX::dup(2) if defined $fd;
			if (!defined $saved || !defined POSIX::dup2($fd, 2)) {
				POSIX::close($saved) if defined $saved;
				$saved = undef;
			}
			1;
		};
	}
	my $p = eval { Proc::Background->new({ die_upon_destroy => 1 }, @$argv) };
	my $err = $@;
	if (defined $saved) {
		POSIX::dup2($saved, 2);
		POSIX::close($saved);
	}
	POSIX::close($fd) if defined $fd;
	$@ = $err;
	return $p;
}

sub _check {
	return unless $wanted;
	if ((!$proc || !eval { $proc->alive }) && !$restart_pending) {
		if ($proc) {
			# It ran and exited: a long run starts the backoff over. A launch that failed
			# (no binary, a bad setting) never resets it, so that backs off too.
			$attempt = 0 if $started_at && _now() - $started_at >= STABLE_AFTER;
			my $tail = stderr_tail();
			$last_error = 'gateway exited' . (length $tail ? ": $tail" : '');
			_remove_pid_file(eval { $proc->pid });
		}
		$started_at = 0;
		$launched_admin = undef;
		$attempt++;
		my $wait = backoff($attempt);
		$last_error ||= 'gateway exited';
		$log->warn("gateway not running; restarting in ${wait}s (attempt $attempt)");
		undef $proc;
		$restart_pending = 1;
		Slim::Utils::Timers::setTimer(undef, _now() + $wait, \&_launch);
	}
	rotate_log(log_file());
	Slim::Utils::Timers::setTimer(undef, _now() + CHECK_INTERVAL, \&_check);
}

1;
