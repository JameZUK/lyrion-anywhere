package Proc::Background;
use POSIX ();
# Test stub: records the command (and where its stderr would go); never runs anything.
# Objects have no pid unless a test sets $NEXT_PID to a child process it made itself: then
# alive() reaps and reports that real process, as Proc::Background does.
use strict; use warnings;
our @STARTED;
our @OBJECTS;
our $NEXT_PID;
sub new { my ($class, $opts, @argv) = @_; push @STARTED, \@argv; my @st = (POSIX::fstat(2))[0, 1]; my $o = bless { alive => 1, argv => \@argv, pid => $NEXT_PID, stderr => (@st ? "$st[0]:$st[1]" : '') }, $class; $NEXT_PID = undef; push @OBJECTS, $o; $o }
sub alive {
	my $s = shift;
	return 0 unless $s->{alive};
	if ($s->{pid}) {
		my $r = waitpid($s->{pid}, POSIX::WNOHANG());
		$s->{alive} = 0 if $r == $s->{pid} || $r == -1;
	}
	return $s->{alive};
}
sub die   { my $s = shift; kill 'KILL', $s->{pid} if $s->{pid} && $s->{alive}; waitpid($s->{pid}, 0) if $s->{pid}; $s->{alive} = 0; 1 }
sub pid   { $_[0]{pid} }
1;
