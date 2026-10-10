package Proc::Background;
# Test stub: records the command; never runs anything.
use strict; use warnings;
our @STARTED;
our @OBJECTS;
sub new { my ($class, $opts, @argv) = @_; push @STARTED, \@argv; my $o = bless { alive => 1, argv => \@argv }, $class; push @OBJECTS, $o; $o }
sub alive { $_[0]{alive} }
sub die   { $_[0]{alive} = 0; 1 }
sub pid   { 4242 }
1;
