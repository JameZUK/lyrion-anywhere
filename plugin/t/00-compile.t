# Every module compiles against the stubs.
use strict; use warnings;
use Test::More;
BEGIN { package main; use constant WEBUI => 1; }
for my $m (qw(Gateway Admin Plays Commands Settings Plugin)) {
	require_ok("Plugins::LyrionAnywhere::$m");
}
done_testing;
