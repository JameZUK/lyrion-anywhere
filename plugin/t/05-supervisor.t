# Supervision: start, restart with backoff after a crash, stop. Proc::Background is stubbed.
use strict; use warnings;
use Test::More;
use File::Temp qw(tempdir);
use Plugins::LyrionAnywhere::Gateway;
use Proc::Background;
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
my $started = @Proc::Background::STARTED;
Plugins::LyrionAnywhere::Gateway->start;
like(Plugins::LyrionAnywhere::Gateway->state->{error}, qr/not found/);
is(scalar @Proc::Background::STARTED, $started, 'nothing started');
Plugins::LyrionAnywhere::Gateway->stop;
$prefs->set(gateway_path => $bin);

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
	is_deeply(\@waits, [5, 10, 30, 60, 60, 60], 'backoff grows to 60 s and stays there');

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

# --- the gateway's stderr goes to a file; its last lines explain an exit ---------------
{
	@Slim::Utils::Timers::TIMERS = ();
	Plugins::LyrionAnywhere::Gateway->start;
	my $file = Plugins::LyrionAnywhere::Gateway::stderr_file();
	is($file, "$dir/lyrion-anywhere/gateway.stderr", 'stderr file in the data dir');
	my @st = stat $file;
	is($Proc::Background::OBJECTS[-1]{stderr}, "$st[0]:$st[1]", "the gateway's stderr is that file");
	my @mine = stat STDERR;
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

done_testing;
