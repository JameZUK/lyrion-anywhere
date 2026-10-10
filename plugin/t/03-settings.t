# Settings: submitted values are cleaned; bad ones are rejected with escaped messages.
use strict; use warnings;
use Test::More;
use File::Temp qw(tempdir);
BEGIN { package main; use constant WEBUI => 1; }
use Plugins::LyrionAnywhere::Settings;
use Slim::Utils::Prefs;

my $clean = \&Plugins::LyrionAnywhere::Settings::clean;
my ($out, $err) = $clean->({
	pref_listen => ' 127.0.0.1:9443 ', pref_public_url => 'https://music.example.com/',
	pref_trusted_proxies => '10.0.0.10,  10.0.0.11', pref_gateway_path => '', pref_enabled => 'on',
});
is_deeply($err, [], 'valid input');
is($out->{pref_listen}, '127.0.0.1:9443');
is($out->{pref_public_url}, 'https://music.example.com', 'trailing slash removed');
is($out->{pref_trusted_proxies}, '10.0.0.10, 10.0.0.11');
is($out->{pref_enabled}, 1);

for my $case (
	[pref_listen => 'example.com:80'],
	[pref_listen => "127.0.0.1:9443\r\nX: y"],
	[pref_public_url => 'http://x/path'],
	[pref_public_url => 'javascript:alert(1)'],
	[pref_trusted_proxies => '10.0.0.1, evil'],
	[pref_gateway_path => 'relative/gw'],
	[pref_gateway_path => '/nonexistent/gw'],
) {
	my ($o, $e) = $clean->({ @$case });
	ok(!exists $o->{ $case->[0] } && @$e, "rejected $case->[0] = $case->[1]");
}

is(Plugins::LyrionAnywhere::Settings::escape_html(q{<script>"x"&'y'</script>}),
	'&lt;script&gt;&quot;x&quot;&amp;&#39;y&#39;&lt;/script&gt;');

# The handler saves good values, keeps the old value for bad ones, and escapes messages.
$Slim::Utils::Prefs::DIR = tempdir(CLEANUP => 1);
my $prefs = preferences('plugin.lyrionanywhere');
$prefs->set(listen => '127.0.0.1:9443');
$prefs->set(enabled => 0);
my $params = { saveSettings => 1, pref_listen => '<img src=x onerror=alert(1)>', pref_trusted_proxies => '<b>x</b>' };
Plugins::LyrionAnywhere::Settings->handler(undef, $params);
is($prefs->get('listen'), '127.0.0.1:9443', 'invalid listen not saved');
unlike($params->{warning}, qr/<img|<b>/, 'warning text is escaped');
like($params->{warning}, qr/&lt;b&gt;/);

done_testing;
