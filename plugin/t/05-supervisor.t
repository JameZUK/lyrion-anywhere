# Supervision: start, restart with backoff after a crash, stop. Proc::Background is stubbed.
use strict; use warnings;
use Test::More;
use File::Temp qw(tempdir);
use Plugins::LyrionAnywhere::Gateway;
use Proc::Background;
use Slim::Utils::Prefs;
use Slim::Utils::Timers;

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

done_testing;
