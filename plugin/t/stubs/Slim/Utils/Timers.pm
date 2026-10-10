package Slim::Utils::Timers;
# Test stub: records timers; tests fire them by hand.
use strict; use warnings;
our @TIMERS;
sub setTimer   { my (undef, $when, $cb) = @_; push @TIMERS, [$when, $cb] }
sub killTimers { my (undef, $cb) = @_; @TIMERS = grep { $_->[1] != $cb } @TIMERS }
1;
