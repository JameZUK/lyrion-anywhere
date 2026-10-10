package Slim::Utils::Prefs;
# Test stub: in-memory prefs per namespace.
use strict; use warnings;
use Exporter 'import';
our @EXPORT = qw(preferences);
our %STORE;
our $DIR = '/tmp/anywhere-test-prefs';
sub dir { $DIR }
sub preferences { my $ns = shift; bless { ns => $ns }, 'Slim::Utils::Prefs::NS' }
package Slim::Utils::Prefs::NS;
sub get  { $Slim::Utils::Prefs::STORE{ $_[0]{ns} }{ $_[1] } }
sub set  { $Slim::Utils::Prefs::STORE{ $_[0]{ns} }{ $_[1] } = $_[2] }
sub init { my ($s, $d) = @_; $Slim::Utils::Prefs::STORE{ $s->{ns} }{$_} //= $d->{$_} for keys %$d }
1;
