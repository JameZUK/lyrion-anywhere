package Proc::Background;
use POSIX ();
# Test stub: records the command (and where its stderr would go); never runs anything.
# Objects have no pid unless a test sets $NEXT_PID to a child process it made itself: then
# alive() reaps and reports that real process, as Proc::Background does.
# Where Linux says (/proc/self/fdinfo), it also records which descriptors above 2 a real
# child would inherit at that moment: those open without close-on-exec.
use strict; use warnings;
our @STARTED;
our @OBJECTS;
our $NEXT_PID;
sub new { my ($class, $opts, @argv) = @_; push @STARTED, \@argv; my @st = (POSIX::fstat(2))[0, 1]; my $o = bless { alive => 1, argv => \@argv, pid => $NEXT_PID, stderr => (@st ? "$st[0]:$st[1]" : ''), inherit => inheritable() }, $class; $NEXT_PID = undef; push @OBJECTS, $o; $o }
sub inheritable {
	opendir my $d, '/proc/self/fd' or return undef;
	my @fds;
	for my $fd (grep { /^[0-9]+\z/ && $_ > 2 } readdir $d) {
		open my $info, '<', "/proc/self/fdinfo/$fd" or next;
		while (my $line = <$info>) {
			next unless $line =~ /^flags:\s*([0-7]+)/;
			push @fds, $fd unless oct($1) & 02000000;   # O_CLOEXEC
			last;
		}
	}
	return [ sort { $a <=> $b } @fds ];
}
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
