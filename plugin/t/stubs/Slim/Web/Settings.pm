package Slim::Web::Settings;
# Test stub: SUPER::handler saves prefs as LMS 9.1's base settings handler does. On a save it
# sets EVERY pref the page lists to its pref_<name> param, whether the form sent one or not:
# a missing param sets the pref to undef (which most validators accept). A missing
# pref_<name> is first taken from an unprefixed <name> param, if there is one. Then it puts
# each pref's stored value in prefs.pref_<name> for the template and calls beforeRender, as
# LMS does; the page would be rendered after that.
use strict; use warnings;
our @HANDLED;
sub new { bless {}, shift }
sub handler {
	my ($class, $client, $params) = @_;
	push @HANDLED, { %$params };
	my ($prefs, @names) = $class->prefs;
	for my $n (@names) {
		if ($params->{saveSettings}) {
			$params->{"pref_$n"} = $params->{$n} if !defined $params->{"pref_$n"} && defined $params->{$n};
			my (undef, $ok) = $prefs->set($n, $params->{"pref_$n"});
			$params->{validated}{$n} = $ok ? 1 : 0;
			$params->{warning} .= "invalid value for $n<br/>" unless $ok;
		}
		$params->{prefs}{"pref_$n"} = $prefs->get($n);
	}
	# Then, as LMS does, the subclass's hook, before the page is rendered.
	$class->beforeRender($params, $client);
	return 'page';
}
sub beforeRender {}
1;
