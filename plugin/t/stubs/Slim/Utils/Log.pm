package Slim::Utils::Log;
# Test stub: records messages.
use strict; use warnings;
use Exporter 'import';
our @EXPORT = qw(logger);
our @MESSAGES;
my $obj = bless {}, __PACKAGE__;
sub logger { $obj }
sub addLogCategory { $obj }
for my $level (qw(debug info warn error)) {
	no strict 'refs';
	*{$level} = sub { push @MESSAGES, [$level, $_[1]] };
}
sub is_debug { 0 }
1;
