package Slim::Web::Settings;
# Test stub: SUPER::handler saves pref_* params like LMS does.
use strict; use warnings;
our @HANDLED;
sub new { bless {}, shift }
sub handler {
	my ($class, $client, $params) = @_;
	push @HANDLED, { %$params };
	my ($prefs, @names) = $class->prefs;
	if ($params->{saveSettings}) {
		for my $n (@names) { $prefs->set($n, $params->{"pref_$n"}) if exists $params->{"pref_$n"} }
	}
	return 'page';
}
1;
