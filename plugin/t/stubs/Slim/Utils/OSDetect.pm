package Slim::Utils::OSDetect;
use strict; use warnings;
our $LOGDIR = '/tmp/anywhere-test-logs';
sub dirsFor { return $_[0] eq 'log' ? ($LOGDIR) : () }
1;
