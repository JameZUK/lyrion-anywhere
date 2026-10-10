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
use File::Spec::Functions qw(abs2rel catdir catfile file_name_is_absolute);
use Socket qw(inet_pton AF_INET AF_INET6);

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
	return (\@argv);
}

# Seconds to wait before restart attempt $n (1-based).
sub backoff {
	my $n = shift;
	my @steps = (5, 10, 30, 60);
	return $steps[ $n <= 0 ? 0 : ($n > @steps ? $#steps : $n - 1) ];
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

# The admin API address. Only a plain port number is ever used: the admin secret is sent
# here, so a crafted pref (e.g. "1@evil.example.com:80") must never redirect it.
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

sub stderr_file { catfile(data_dir(), STDERR_FILE) }

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
	admin_port      => \&valid_port,
	gateway_path    => \&valid_gateway_path,
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
	my ($logdir) = Slim::Utils::OSDetect::dirsFor('log');
	my $public = $prefs->get('public_url') || '';
	return (
		binary          => binary_path(basedir => $basedir, override => gateway_override()),
		listen          => $prefs->get('listen') || '127.0.0.1:9443',
		admin_listen    => admin_listen(),
		data_dir        => data_dir(),
		http_port       => $server->get('httpport') || 9000,
		cli_port        => ($cli && $cli->get('cliport')) || 9090,
		log_file        => $logdir ? catfile($logdir, 'lyrion-anywhere.log') : undef,
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

sub stop {
	my $class = shift;
	$wanted = 0;
	Slim::Utils::Timers::killTimers(undef, \&_check);
	Slim::Utils::Timers::killTimers(undef, \&_launch);
	$restart_pending = 0;
	if ($proc) {
		$log->info('stopping gateway');
		eval { $proc->die };
		undef $proc;
	}
}

sub restart {
	my $class = shift;
	$class->stop;
	$attempt = 0;
	$class->start;
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
	require Proc::Background;
	$proc = _spawn($argv, stderr_file());
	if ($proc) {
		$started_at = _now();
		$last_error = '';
		$log->info('gateway started (pid ' . ($proc->pid // '?') . ")");
	} else {
		$last_error = "could not start gateway: " . ($@ || 'unknown error');
		$log->error($last_error);
	}
}

# Start the gateway with its stderr going to a file, so the reason for an early exit (a
# bad flag, a port in use) can be shown in the status. The child inherits the handle; the
# plugin's own stderr is put back straight away.
sub _spawn {
	my ($argv, $errfile) = @_;
	my ($saved, $redirected);
	if ($errfile && open($saved, '>&', \*STDERR)) {
		$redirected = open(STDERR, '>', $errfile);
		open(STDERR, '>&', $saved) unless $redirected;
	}
	my $p = eval { Proc::Background->new({ die_upon_destroy => 1 }, @$argv) };
	my $err = $@;
	open(STDERR, '>&', $saved) if $redirected;
	$@ = $err;
	return $p;
}

sub _check {
	return unless $wanted;
	if ((!$proc || !eval { $proc->alive }) && !$restart_pending) {
		$attempt = 0 if $started_at && _now() - $started_at >= STABLE_AFTER;
		$attempt++;
		my $wait = backoff($attempt);
		if ($proc) {
			my $tail = stderr_tail();
			$last_error = 'gateway exited' . (length $tail ? ": $tail" : '');
		}
		$last_error ||= 'gateway exited';
		$log->warn("gateway not running; restarting in ${wait}s (attempt $attempt)");
		undef $proc;
		$restart_pending = 1;
		Slim::Utils::Timers::setTimer(undef, _now() + $wait, \&_launch);
	}
	Slim::Utils::Timers::setTimer(undef, _now() + CHECK_INTERVAL, \&_check);
}

1;
