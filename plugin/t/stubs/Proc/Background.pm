package Proc::Background;
# Test stub: records the command (and where its stderr would go); never runs anything.
use strict; use warnings;
our @STARTED;
our @OBJECTS;
sub new { my ($class, $opts, @argv) = @_; push @STARTED, \@argv; my @st = stat(STDERR); my $o = bless { alive => 1, argv => \@argv, stderr => (@st ? "$st[0]:$st[1]" : '') }, $class; push @OBJECTS, $o; $o }
sub alive { $_[0]{alive} }
sub die   { $_[0]{alive} = 0; 1 }
sub pid   { 4242 }
1;
