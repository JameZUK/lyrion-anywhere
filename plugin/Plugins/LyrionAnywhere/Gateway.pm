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
use File::Spec::Functions qw(catdir catfile file_name_is_absolute);

use Slim::Utils::Log;
use Slim::Utils::OSDetect;
use Slim::Utils::Prefs;
use Slim::Utils::Timers;

my $log   = logger('plugin.lyrionanywhere');
my $prefs = preferences('plugin.lyrionanywhere');

use constant CHECK_INTERVAL => 10;      # seconds between liveness checks
use constant STABLE_AFTER   => 300;     # a run this long resets the backoff

my $proc;            # Proc::Background object while running
my $wanted = 0;      # should the gateway be running?
my $attempt = 0;     # consecutive failed starts
my $started_at = 0;
my $last_error = '';
my $basedir = '';

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

my $IPV4 = qr/^(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)$/;

sub valid_ip {
	my $ip = shift // return 0;
	return 1 if $ip =~ $IPV4;
	return 1 if $ip =~ /^[0-9a-f:]+$/i && $ip =~ /:/ && length($ip) <= 39;
	return 0;
}

sub valid_hostport {
	my $hp = shift // return 0;
	my ($host, $port);
	if ($hp =~ /^\[([0-9a-f:]+)\]:(\d{1,5})$/i) { ($host, $port) = ($1, $2) }
	elsif ($hp =~ /^([0-9.]+):(\d{1,5})$/)      { ($host, $port) = ($1, $2) }
	else { return 0 }
	return valid_ip($host) && $port >= 1 && $port <= 65535;
}

sub is_loopback {
	my $hp = shift // return 0;
	return $hp =~ /^127\.\d+\.\d+\.\d+:\d+$/ || $hp =~ /^\[::1\]:\d+$/;
}

sub valid_origin {
	my $o = shift // return 0;
	return $o =~ m{^https?://[a-z0-9.-]+(?::\d{1,5})?$}i && length($o) <= 255;
}

# The gateway's argument list. Returns (\@argv) or (undef, $error).
sub command_line {
	my (%o) = @_;
	return (undef, 'gateway binary not found for this platform') unless $o{binary};
	return (undef, 'listen address must be IP:port')              unless valid_hostport($o{listen});
	return (undef, 'admin API must listen on loopback')            unless is_loopback($o{admin_listen});
	return (undef, 'data directory must be an absolute path')      unless $o{data_dir} && file_name_is_absolute($o{data_dir});
	for my $p (qw(http_port cli_port)) {
		return (undef, "bad $p") unless defined $o{$p} && $o{$p} =~ /^\d{1,5}$/ && $o{$p} > 0 && $o{$p} <= 65535;
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

# Split a comma/space separated list.
sub split_list {
	my $s = shift // '';
	return grep { length } map { s/^\s+|\s+$//gr } split /[,\s]+/, $s;
}

# --- LMS integration ----------------------------------------------------------------

sub data_dir {
	return catdir(Slim::Utils::Prefs::dir(), 'lyrion-anywhere');
}

sub admin_listen {
	return '127.0.0.1:' . ($prefs->get('admin_port') || 9445);
}

sub _options {
	my $server = preferences('server');
	my $cli    = preferences('plugin.cli');
	my ($logdir) = Slim::Utils::OSDetect::dirsFor('log');
	my $public = $prefs->get('public_url') || '';
	return (
		binary          => binary_path(basedir => $basedir, override => $prefs->get('gateway_path')),
		listen          => $prefs->get('listen') || '127.0.0.1:9443',
		admin_listen    => admin_listen(),
		data_dir        => data_dir(),
		http_port       => $server->get('httpport') || 9000,
		cli_port        => ($cli && $cli->get('cliport')) || 9090,
		log_file        => $logdir ? catfile($logdir, 'lyrion-anywhere.log') : undef,
		trusted_proxies => [ split_list($prefs->get('trusted_proxies')) ],
		allowed_origins => [ $public ? $public : () ],
	);
}

sub init {
	my ($class, $dir) = @_;
	$basedir = $dir;
	$class->start if $prefs->get('enabled');
}

sub start {
	my $class = shift;
	$wanted = 1;
	_launch();
	Slim::Utils::Timers::killTimers(undef, \&_check);
	Slim::Utils::Timers::setTimer(undef, time() + CHECK_INTERVAL, \&_check);
}

sub stop {
	my $class = shift;
	$wanted = 0;
	Slim::Utils::Timers::killTimers(undef, \&_check);
	Slim::Utils::Timers::killTimers(undef, \&_launch);
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
		uptime    => ($proc && $started_at) ? time() - $started_at : 0,
	};
}

sub _launch {
	return unless $wanted;
	return if $proc && eval { $proc->alive };
	my %o = _options();
	my ($argv, $err) = command_line(%o);
	if (!$argv) {
		$last_error = $err;
		$log->error("cannot start gateway: $err");
		return;
	}
	if (!-x $argv->[0]) {
		$last_error = "gateway binary not found: $argv->[0]";
		$log->error($last_error);
		return;
	}
	mkdir $o{data_dir}, 0700 unless -d $o{data_dir};
	require Proc::Background;
	$proc = eval { Proc::Background->new({ die_upon_destroy => 1 }, @$argv) };
	if ($proc) {
		$started_at = time();
		$last_error = '';
		$log->info('gateway started (pid ' . ($proc->pid // '?') . ")");
	} else {
		$last_error = "could not start gateway: " . ($@ || 'unknown error');
		$log->error($last_error);
	}
}

sub _check {
	return unless $wanted;
	if (!$proc || !eval { $proc->alive }) {
		$attempt = 0 if $started_at && time() - $started_at >= STABLE_AFTER;
		$attempt++;
		my $wait = backoff($attempt);
		$last_error ||= 'gateway exited';
		$log->warn("gateway not running; restarting in ${wait}s (attempt $attempt)");
		undef $proc;
		Slim::Utils::Timers::setTimer(undef, time() + $wait, \&_launch);
	}
	Slim::Utils::Timers::setTimer(undef, time() + CHECK_INTERVAL, \&_check);
}

1;
