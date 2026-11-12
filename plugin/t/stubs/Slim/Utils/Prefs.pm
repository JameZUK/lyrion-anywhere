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
# Like LMS (Slim::Utils::Prefs::Base::set): setting the value already stored does nothing; a
# value its validator refuses is not stored (the old one stays). Returns (value, ok) in list
# context, the value in scalar context.
sub set  {
	my ($s, $k, $v) = @_;
	my $old = $s->get($k);
	if ((!defined $v && !defined $old) || (!ref $v && defined $v && defined $old && $v eq $old)) {
		return wantarray ? ($v, 1) : $v;
	}
	my $check = $VALIDATORS{ $s->{ns} }{$k};
	return wantarray ? ($old, 0) : $old if $check && !$check->($k, $v, {}, $old, undef);
	$Slim::Utils::Prefs::STORE{ $s->{ns} }{$k} = $v;
	return wantarray ? ($v, 1) : $v;
}
sub setValidate {
	my ($s, $v, @names) = @_;
	my $code = ref $v eq 'HASH' ? $v->{validator} : $v;
	die 'stub supports code validators only' unless ref $code eq 'CODE';
	$VALIDATORS{ $s->{ns} }{$_} = $code for @names;
}
sub init { my ($s, $d) = @_; $Slim::Utils::Prefs::STORE{ $s->{ns} }{$_} //= $d->{$_} for keys %$d }
1;
