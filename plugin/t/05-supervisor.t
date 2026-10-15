# Supervision: start, restart with backoff after a crash, stop. Proc::Background is stubbed.
use strict; use warnings;
use Test::More;
use POSIX ();
use Time::HiRes ();
use File::Temp qw(tempdir);
use Plugins::LyrionAnywhere::Gateway;
use Proc::Background;
use Slim::Utils::Log;
use Slim::Utils::Prefs;
use Slim::Utils::Timers;

# The development gateway path is honoured only with this set in LMS's environment.
$ENV{ANYWHERE_ALLOW_GATEWAY_PATH} = '1';

my $dir = tempdir(CLEANUP => 1);
$Slim::Utils::Prefs::DIR = $dir;
my $bin = "$dir/fake-gateway";
open my $fh, '>', $bin or die; print $fh "#!/bin/sh\n"; close $fh; chmod 0755, $bin;
my $prefs = preferences('plugin.lyrionanywhere');
$prefs->set(gateway_path => $bin);
$prefs->set(listen => '127.0.0.1:9443');
$prefs->set(admin_port => 9445);
$prefs->set(enabled => 1);
preferences('server')->set(httpport => 9000);

sub timer_for {
	my $name = shift;
	my $cb = \&{"Plugins::LyrionAnywhere::Gateway::$name"};
	my ($t) = grep { $_->[1] == $cb } @Slim::Utils::Timers::TIMERS;
	return $t;
}

Plugins::LyrionAnywhere::Gateway->start;
is(scalar @Proc::Background::STARTED, 1, 'started once');
is($Proc::Background::STARTED[0][0], $bin);
ok(grep({ $_ eq '--admin-listen' } @{ $Proc::Background::STARTED[0] }), 'admin listener passed');
ok(-d "$dir/lyrion-anywhere", 'data dir created');
is((stat "$dir/lyrion-anywhere")[2] & 0777, 0700, 'data dir is private');
is(Plugins::LyrionAnywhere::Gateway->state->{running}, 1);
ok(timer_for('_check'), 'liveness check scheduled');

# The gateway crashes: the next check schedules a restart after the first backoff step.
$Proc::Background::OBJECTS[-1]->die;
my $check = timer_for('_check');
@Slim::Utils::Timers::TIMERS = ();
$check->[1]->();
my $launch = timer_for('_launch');
ok($launch, 'restart scheduled');
cmp_ok($launch->[0] - time(), '>=', 4, 'after a backoff delay');
cmp_ok($launch->[0] - time(), '<=', 5);
$launch->[1]->();
is(scalar @Proc::Background::STARTED, 2, 'restarted');
is(Plugins::LyrionAnywhere::Gateway->state->{running}, 1);

# A second crash waits longer.
$Proc::Background::OBJECTS[-1]->die;
@Slim::Utils::Timers::TIMERS = ();
Plugins::LyrionAnywhere::Gateway::_check();
cmp_ok(timer_for('_launch')->[0] - time(), '>=', 9, 'backoff grows');

# Stop kills the process and cancels every timer.
Plugins::LyrionAnywhere::Gateway->stop;
is(Plugins::LyrionAnywhere::Gateway->state->{running}, 0, 'stopped');
ok(!timer_for('_check') && !timer_for('_launch'), 'no timers left after stop');

# A missing binary is reported, not crashed on.
$prefs->set(gateway_path => "$dir/missing");
Plugins::LyrionAnywhere::Gateway::settings_saved();   # as the settings page would
my $started = @Proc::Background::STARTED;
Plugins::LyrionAnywhere::Gateway->start;
like(Plugins::LyrionAnywhere::Gateway->state->{error}, qr/not found/);
is(scalar @Proc::Background::STARTED, $started, 'nothing started');
Plugins::LyrionAnywhere::Gateway->stop;
$prefs->set(gateway_path => $bin);
Plugins::LyrionAnywhere::Gateway::settings_saved();   # as the settings page would

# --- crash loop: one pending restart at a time, so the backoff really reaches 60 s -------
{
	my $now = 1_000_000;
	no warnings 'redefine';
	local *Plugins::LyrionAnywhere::Gateway::_now = sub { $now };
	@Slim::Utils::Timers::TIMERS = ();
	my $launches = sub { grep { $_->[1] == \&Plugins::LyrionAnywhere::Gateway::_launch } @Slim::Utils::Timers::TIMERS };
	Plugins::LyrionAnywhere::Gateway->restart;

	my @waits;
	for my $crash (1 .. 6) {
		$Proc::Background::OBJECTS[-1]->die;
		# Several liveness checks pass before the restart is due (every 10 s for up to 60 s):
		# none of them may add another restart or count another attempt.
		my $first;
		for (1 .. 6) {
			$now += 10;
			Plugins::LyrionAnywhere::Gateway::_check();
			$first //= ($launches->())[0];
		}
		is(scalar(my @l = $launches->()), 1, "crash $crash: exactly one restart pending");
		push @waits, $first->[0] - ($now - 50);
		# The restart fires: the timer goes and the gateway runs again.
		@Slim::Utils::Timers::TIMERS = grep { $_ != $first } @Slim::Utils::Timers::TIMERS;
		$first->[1]->();
		is(Plugins::LyrionAnywhere::Gateway->state->{running}, 1, "crash $crash: restarted");
	}
	is_deeply(\@waits, [5, 10, 20, 40, 80, 160], 'backoff doubles each time');

	# A run longer than five minutes starts the count over.
	$now += 301;
	$Proc::Background::OBJECTS[-1]->die;
	Plugins::LyrionAnywhere::Gateway::_check();
	is(($launches->())[0][0] - $now, 5, 'after a stable run the backoff starts again at 5 s');

	# stop() clears the pending restart: a later crash schedules a new one.
	Plugins::LyrionAnywhere::Gateway->stop;
	ok(!$launches->(), 'stop cancels the pending restart');
	Plugins::LyrionAnywhere::Gateway->start;
	$Proc::Background::OBJECTS[-1]->die;
	Plugins::LyrionAnywhere::Gateway::_check();
	is(scalar(my @l = $launches->()), 1, 'after stop/start a crash schedules a restart again');
	Plugins::LyrionAnywhere::Gateway->stop;
}

# Regression: LMS ties STDERR to its log (Slim::Utils::Log::Trapper). Re-opening a tied
# handle died inside the start-up timer, so the gateway never started after an LMS restart.
{
	package Test::Trapper;
	sub TIEHANDLE { bless {}, shift }
	sub PRINT { 1 }
	sub PRINTF { 1 }
	sub OPEN { die "\n" }
	package main;
	@Slim::Utils::Timers::TIMERS = ();
	my $before = @Proc::Background::OBJECTS;
	tie *STDERR, 'Test::Trapper';
	my $ok = eval { Plugins::LyrionAnywhere::Gateway->start; 1 };
	untie *STDERR;
	ok($ok, 'starting with a tied STDERR does not die') or diag($@);
	is(scalar @Proc::Background::OBJECTS, $before + 1, 'the gateway was started');
	my @st = stat Plugins::LyrionAnywhere::Gateway::stderr_file();
	is($Proc::Background::OBJECTS[-1]{stderr}, "$st[0]:$st[1]", "its stderr still goes to the file");
	Plugins::LyrionAnywhere::Gateway->stop;
}

# --- the gateway's stderr goes to a file; its last lines explain an exit ---------------
{
	@Slim::Utils::Timers::TIMERS = ();
	Plugins::LyrionAnywhere::Gateway->start;
	my $file = Plugins::LyrionAnywhere::Gateway::stderr_file();
	is($file, "$dir/lyrion-anywhere/gateway.stderr", 'stderr file in the data dir');
	my @st = stat $file;
	is($Proc::Background::OBJECTS[-1]{stderr}, "$st[0]:$st[1]", "the gateway's stderr is that file");
	my @mine = (POSIX::fstat(2))[0, 1];
	isnt("$mine[0]:$mine[1]", "$st[0]:$st[1]", "the plugin's own stderr is put back");
	open my $fh, '>>', $file or die;
	print $fh "line $_\n" for 1 .. 8;
	print $fh "Error: address in use \e[31m(127.0.0.1:9443)\n";
	close $fh;
	$Proc::Background::OBJECTS[-1]->die;
	Plugins::LyrionAnywhere::Gateway::_check();
	my $error = Plugins::LyrionAnywhere::Gateway->state->{error};
	like($error, qr/^gateway exited: /, 'exit reported');
	like($error, qr/Error: address in use \[31m\(127\.0\.0\.1:9443\)\z/, 'with the last line (control characters removed)');
	unlike($error, qr/line [1-4]\b/, 'only the last few lines');
	unlike($error, qr/\e/, 'no escape characters');
	Plugins::LyrionAnywhere::Gateway->stop;
	is(Plugins::LyrionAnywhere::Gateway::stderr_tail("$dir/none"), '', 'no file: no tail');
}

# --- the development gateway path needs ANYWHERE_ALLOW_GATEWAY_PATH=1 ------------------
{
	local $ENV{ANYWHERE_ALLOW_GATEWAY_PATH};
	@Slim::Utils::Log::MESSAGES = ();
	my $n = @Proc::Background::STARTED;
	Plugins::LyrionAnywhere::Gateway->set_basedir("$dir/plugin");
	Plugins::LyrionAnywhere::Gateway->start;
	is(scalar @Proc::Background::STARTED, $n, 'arbitrary binary not run without the variable');
	like(Plugins::LyrionAnywhere::Gateway->state->{binary}, qr{^\Q$dir\E/plugin/Bin/}, 'the bundled binary is used instead');
	ok(grep({ $_->[0] eq 'warn' && $_->[1] =~ /ANYWHERE_ALLOW_GATEWAY_PATH=1/ } @Slim::Utils::Log::MESSAGES),
		'a warning explains why the setting is ignored');
	Plugins::LyrionAnywhere::Gateway->stop;
	$ENV{ANYWHERE_ALLOW_GATEWAY_PATH} = 'yes';
	is(Plugins::LyrionAnywhere::Gateway::gateway_override(), undef, 'only the exact value 1 enables it');
	$ENV{ANYWHERE_ALLOW_GATEWAY_PATH} = '1';
	is(Plugins::LyrionAnywhere::Gateway::gateway_override(), $bin, 'honoured with the variable');
}

# --- a failing launch backs off too, even after a long good run --------------------------
# Regression: the backoff reset whenever the last *successful* start was over five minutes
# ago, so a gateway that then couldn't start at all was retried every 5 s for ever.
{
	my $now = 2_000_000;
	no warnings 'redefine';
	local *Plugins::LyrionAnywhere::Gateway::_now = sub { $now };
	@Slim::Utils::Timers::TIMERS = ();
	Plugins::LyrionAnywhere::Gateway->restart;
	$now += 3600;   # a long, stable run
	$prefs->set(gateway_path => "$dir/missing");
	Plugins::LyrionAnywhere::Gateway::settings_saved();   # as the settings page would
	$Proc::Background::OBJECTS[-1]->die;
	my @waits;
	for (1 .. 4) {
		Plugins::LyrionAnywhere::Gateway::_check();
		my ($l) = grep { $_->[1] == \&Plugins::LyrionAnywhere::Gateway::_launch } @Slim::Utils::Timers::TIMERS;
		push @waits, $l->[0] - $now;
		@Slim::Utils::Timers::TIMERS = ();
		$now = $l->[0];
		$l->[1]->();   # fails: no binary
	}
	is_deeply(\@waits, [5, 10, 20, 40], 'failed launches keep backing off');
	Plugins::LyrionAnywhere::Gateway->stop;
	$prefs->set(gateway_path => $bin);
	Plugins::LyrionAnywhere::Gateway::settings_saved();   # as the settings page would
}

# --- stopping never blocks LMS; a gateway that ignores SIGTERM gets SIGKILL ---------------

# A real child process standing in for the gateway. Returns its pid once it is ready.
sub child {
	my (%o) = @_;
	pipe my $r, my $w or die;
	my $pid = fork // die;
	if (!$pid) {
		close $r;
		$SIG{TERM} = 'IGNORE' if $o{ignore_term};
		print $w "ready\n";
		close $w;
		sleep 30;
		POSIX::_exit(0);
	}
	close $w;
	my $ready = <$r>;
	close $r;
	return $pid;
}
sub gone { my $pid = shift; for (1 .. 50) { return 1 if waitpid($pid, POSIX::WNOHANG()) != 0 || !kill(0, $pid); select undef, undef, undef, 0.1 } 0 }
sub pid_in_file { my $f = Plugins::LyrionAnywhere::Gateway::pid_file(); open my $fh, '<', $f or return undef; my $l = <$fh>; chomp $l if defined $l; $l }

{
	@Slim::Utils::Timers::TIMERS = ();
	my $pid = child();
	$Proc::Background::NEXT_PID = $pid;
	Plugins::LyrionAnywhere::Gateway->start;
	is(pid_in_file(), $pid, 'the running gateway\'s pid is kept in the data dir');
	is((stat Plugins::LyrionAnywhere::Gateway::pid_file())[2] & 0777, 0600, 'pid file is private');
	my $t0 = Time::HiRes::time();
	Plugins::LyrionAnywhere::Gateway->stop;
	cmp_ok(Time::HiRes::time() - $t0, '<', 0.5, 'stop returns at once');
	my ($reap) = grep { $_->[1] == \&Plugins::LyrionAnywhere::Gateway::_reap } @Slim::Utils::Timers::TIMERS;
	ok($reap, 'a timer collects the stopped gateway');
	select undef, undef, undef, 0.3;
	@Slim::Utils::Timers::TIMERS = ();
	Plugins::LyrionAnywhere::Gateway::_reap();
	ok(!kill(0, $pid), 'SIGTERM stopped it');
	ok(!defined pid_in_file(), 'pid file removed once it has gone');
	ok(!grep({ $_->[1] == \&Plugins::LyrionAnywhere::Gateway::_reap } @Slim::Utils::Timers::TIMERS), 'nothing left to collect');
}

{
	my $now = 3_000_000;
	no warnings 'redefine';
	local *Plugins::LyrionAnywhere::Gateway::_now = sub { $now };
	@Slim::Utils::Timers::TIMERS = ();
	my $pid = child(ignore_term => 1);
	$Proc::Background::NEXT_PID = $pid;
	Plugins::LyrionAnywhere::Gateway->start;
	my $started = @Proc::Background::STARTED;
	Plugins::LyrionAnywhere::Gateway->stop;
	# A restart now must wait for the old gateway to let go of its ports.
	Plugins::LyrionAnywhere::Gateway->start;
	is(scalar @Proc::Background::STARTED, $started, 'the new gateway waits for the old one');
	ok(grep({ $_->[1] == \&Plugins::LyrionAnywhere::Gateway::_launch } @Slim::Utils::Timers::TIMERS), 'and is started a moment later');
	select undef, undef, undef, 0.3;
	Plugins::LyrionAnywhere::Gateway::_reap();
	ok(kill(0, $pid), 'it ignored SIGTERM');
	$now += Plugins::LyrionAnywhere::Gateway::TERM_GRACE();
	Plugins::LyrionAnywhere::Gateway::_reap();
	ok(gone($pid), 'SIGKILL after the grace period');
	Plugins::LyrionAnywhere::Gateway::_reap();
	my ($l) = grep { $_->[1] == \&Plugins::LyrionAnywhere::Gateway::_launch } @Slim::Utils::Timers::TIMERS;
	@Slim::Utils::Timers::TIMERS = ();
	$l->[1]->();
	is(scalar @Proc::Background::STARTED, $started + 1, 'then the new gateway starts');
	Plugins::LyrionAnywhere::Gateway->stop;
}

# LMS shutting down waits for the gateway, but only briefly.
{
	@Slim::Utils::Timers::TIMERS = ();
	my $pid = child(ignore_term => 1);
	$Proc::Background::NEXT_PID = $pid;
	Plugins::LyrionAnywhere::Gateway->start;
	my $t0 = Time::HiRes::time();
	Plugins::LyrionAnywhere::Gateway->stop_for_shutdown;
	my $took = Time::HiRes::time() - $t0;
	cmp_ok($took, '<=', Plugins::LyrionAnywhere::Gateway::SHUTDOWN_WAIT() + 1, "shutdown waits at most a few seconds (${took}s)");
	ok(gone($pid), 'and kills a gateway that would not stop');

	my $pid2 = child();
	$Proc::Background::NEXT_PID = $pid2;
	Plugins::LyrionAnywhere::Gateway->start;
	$t0 = Time::HiRes::time();
	Plugins::LyrionAnywhere::Gateway->stop_for_shutdown;
	cmp_ok(Time::HiRes::time() - $t0, '<', 2, 'a gateway that stops at SIGTERM is not waited on for long');
	ok(gone($pid2), 'it stopped');
	ok(!defined pid_in_file(), 'pid file removed');
}

# --- a gateway left behind by an LMS that crashed is stopped, and only ours ---------------
{
	my $root = tempdir(CLEANUP => 1);
	local $Plugins::LyrionAnywhere::Gateway::PROC_ROOT = $root;
	my $data = "$dir/lyrion-anywhere";
	my $fake_cmdline = sub {
		my ($pid, @argv) = @_;
		mkdir "$root/$pid";
		open my $fh, '>:raw', "$root/$pid/cmdline" or die;
		print $fh join("\0", @argv), "\0";
		close $fh;
	};
	my $write_pid = sub {
		open my $fh, '>', Plugins::LyrionAnywhere::Gateway::pid_file() or die;
		print $fh shift, "\n";
		close $fh;
	};

	# Ours: the pid file names it and its command line is our gateway with our data dir.
	@Slim::Utils::Timers::TIMERS = ();
	my $orphan = child();
	$fake_cmdline->($orphan, '/opt/lms/Plugins/LyrionAnywhere/Bin/x86_64-linux/anywhere-gateway', 'serve', '--listen', '127.0.0.1:9443', '--data-dir', $data);
	$write_pid->($orphan);
	my $n = @Proc::Background::STARTED;
	Plugins::LyrionAnywhere::Gateway->start;
	is(scalar @Proc::Background::STARTED, $n, 'not started while the old gateway holds the ports');
	ok(gone($orphan), 'the orphaned gateway was stopped');
	my ($l) = grep { $_->[1] == \&Plugins::LyrionAnywhere::Gateway::_launch } @Slim::Utils::Timers::TIMERS;
	@Slim::Utils::Timers::TIMERS = ();
	$l->[1]->();
	is(scalar @Proc::Background::STARTED, $n + 1, 'then the new one starts');
	ok(!defined pid_in_file(), 'the stale pid file is gone');
	Plugins::LyrionAnywhere::Gateway->stop;

	# Not ours: never signalled, whatever the pid file says.
	for my $case (
		['another program', ['/usr/bin/sleep', '30']],
		['a gateway for another data dir', ['/x/anywhere-gateway', 'serve', '--data-dir', '/srv/other']],
		['no process information', undef],
	) {
		my ($name, $argv) = @$case;
		my $victim = child();
		$fake_cmdline->($victim, @$argv) if $argv;
		$write_pid->($victim);
		@Slim::Utils::Log::MESSAGES = ();
		$n = @Proc::Background::STARTED;
		Plugins::LyrionAnywhere::Gateway->start;
		select undef, undef, undef, 0.2;
		ok(kill(0, $victim) && !waitpid($victim, POSIX::WNOHANG()), "$name: left alone");
		is(scalar @Proc::Background::STARTED, $n + 1, "$name: the gateway starts at once");
		ok(grep({ $_->[1] =~ /not this plugin's gateway/ } @Slim::Utils::Log::MESSAGES), "$name: a warning is logged");
		Plugins::LyrionAnywhere::Gateway->stop;
		kill 'KILL', $victim;
		waitpid($victim, 0);
	}
	for my $junk ($$, 1, 0, 'abc', "1\n2", '') {
		$write_pid->($junk);
		$n = @Proc::Background::STARTED;
		Plugins::LyrionAnywhere::Gateway->start;
		is(scalar @Proc::Background::STARTED, $n + 1, "pid file '" . ($junk =~ s/\n/\\n/r) . "': ignored, gateway started");
		Plugins::LyrionAnywhere::Gateway->stop;
	}
	ok(1, 'this test process was not signalled');
}

done_testing;
