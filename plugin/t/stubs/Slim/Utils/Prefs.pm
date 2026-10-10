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
our %VALIDATORS;   # namespace => pref => code, as registered with setValidate
# Like LMS: a value its validator refuses is not stored (the old one stays).
sub set  {
	my ($s, $k, $v) = @_;
	my $check = $VALIDATORS{ $s->{ns} }{$k};
	return undef if $check && !$check->($k, $v, {}, $s->get($k), undef);
	$Slim::Utils::Prefs::STORE{ $s->{ns} }{$k} = $v;
}
sub setValidate {
	my ($s, $v, @names) = @_;
	my $code = ref $v eq 'HASH' ? $v->{validator} : $v;
	die 'stub supports code validators only' unless ref $code eq 'CODE';
	$VALIDATORS{ $s->{ns} }{$_} = $code for @names;
}
sub init { my ($s, $d) = @_; $Slim::Utils::Prefs::STORE{ $s->{ns} }{$_} //= $d->{$_} for keys %$d }
1;
