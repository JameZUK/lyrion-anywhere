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

# The page gets the CSRF token for its script (and only through the template).
require Plugins::LyrionAnywhere::Commands;
my $token = Plugins::LyrionAnywhere::Commands::csrf_token();
$params = {};
Plugins::LyrionAnywhere::Settings->handler(undef, $params);
is($params->{anywhere_csrf}, $token, 'settings page is given the token');
$params = { saveSettings => 1, pref_listen => '127.0.0.1:9443' };
Plugins::LyrionAnywhere::Settings->handler(undef, $params);
is($params->{anywhere_csrf}, $token, 'also after saving');
{
	open my $fh, '<', 'Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere/settings/basic.html' or die;
	my $html = do { local $/; <$fh> };
	like($html, qr/data-csrf="\[% anywhere_csrf \| html %\]"/, 'template puts the token in an escaped data attribute');
	like($html, qr/9000/, 'page explains that LMS ports are open on the LAN');
}

# --- prefs changed outside the page (LMS's `pref` command) get the same checks ---------
{
	Plugins::LyrionAnywhere::Gateway::register_validators();
	my $exe = "$Slim::Utils::Prefs::DIR/gw";
	open my $fh, '>', $exe or die; close $fh; chmod 0755, $exe;
	my %good = (listen => '10.0.0.20:9443', public_url => 'https://music.example.com',
	            trusted_proxies => '10.0.0.10, 10.0.0.11', admin_port => 9446, gateway_path => $exe);
	my %bad = (
		listen          => ['0.0.0.0', 'localhost:9443'],
		public_url      => ['javascript:alert(1)', 'https://music.example.com/x'],
		trusted_proxies => ['evil', ':::'],
		admin_port      => ['1@evil.example.com:80', 0, 70000],
		gateway_path    => ['/bin/../nonexistent', 'sh', "$Slim::Utils::Prefs::DIR"],
	);
	for my $name (sort keys %good) {
		$prefs->set($name, $good{$name});
		is($prefs->get($name), $good{$name}, "pref $name: valid value stored");
		for my $v (@{ $bad{$name} }) {
			$prefs->set($name, $v);
			is($prefs->get($name), $good{$name}, "pref $name: '$v' refused, old value kept");
		}
	}
	for my $name (qw(public_url trusted_proxies gateway_path)) {
		$prefs->set($name, '');
		is($prefs->get($name), '', "pref $name can be cleared");
	}
}

done_testing;
