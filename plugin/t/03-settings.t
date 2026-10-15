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
	[pref_ffmpeg_path => 'ffmpeg'],
	[pref_ffmpeg_path => '/bin/sh'],
	[pref_ffmpeg_path => '/nonexistent/ffmpeg'],
	[pref_download_cache_mb => '8'],
	[pref_download_cache_mb => '2048; x'],
	[pref_download_cache_mb => "2048\nx"],
	[pref_client_ip_header => 'CF Connecting IP'],
	[pref_client_ip_header => 'X-Forwarded-For'],
	[pref_client_ip_header => 'Host'],
	[pref_client_ip_header => '<b>x</b>'],
	[pref_client_ip_header => "CF-Connecting-IP\r\nX-Evil: 1"],
	[pref_client_ip_header => 'a' x 65],
) {
	my ($o, $e) = $clean->({ @$case });
	ok(!exists $o->{ $case->[0] } && @$e, "rejected $case->[0] = $case->[1]");
}

{
	my ($o, $e) = $clean->({ pref_ffmpeg_path => '', pref_download_cache_mb => ' 4096 ', pref_transcode => 'on' });
	is_deeply($e, [], 'download options: valid');
	is($o->{pref_ffmpeg_path}, '', 'empty ffmpeg path: look on PATH');
	is($o->{pref_download_cache_mb}, 4096, 'cache size trimmed');
	is($o->{pref_transcode}, 1, 'transcoding ticked');
	($o, $e) = $clean->({ pref_download_cache_mb => '', pref_transcode => '' });
	is($o->{pref_download_cache_mb}, 2048, 'empty cache size: the default');
	is($o->{pref_transcode}, 0, 'transcoding unticked');
}

{
	my ($o, $e) = $clean->({ pref_analysis => 'on' });
	is_deeply($e, [], 'analysis: valid');
	is($o->{pref_analysis}, 1, 'analysis ticked');
	($o, $e) = $clean->({ pref_analysis => '' });
	is($o->{pref_analysis}, 0, 'analysis unticked');
	($o, $e) = $clean->({ pref_analysis => "1\n--evil" });
	is($o->{pref_analysis}, 1, 'anything truthy is just "on": never passed through');
	($o, $e) = $clean->({});
	ok(!exists $o->{pref_analysis}, 'not submitted: left alone');
}

{
	my ($o, $e) = $clean->({ pref_client_ip_header => ' CF-Connecting-IP ' });
	is_deeply($e, [], 'client address header: valid');
	is($o->{pref_client_ip_header}, 'CF-Connecting-IP', 'trimmed');
	($o, $e) = $clean->({ pref_client_ip_header => '' });
	is_deeply($e, [], 'client address header: empty is fine');
	is($o->{pref_client_ip_header}, '', 'none');
}

is(Plugins::LyrionAnywhere::Settings::escape_html(q{<script>"x"&'y'</script>}),
	'&lt;script&gt;&quot;x&quot;&amp;&#39;y&#39;&lt;/script&gt;');

# The handler saves good values, keeps the old value for bad ones, and escapes messages.
$Slim::Utils::Prefs::DIR = tempdir(CLEANUP => 1);
my $prefs = preferences('plugin.lyrionanywhere');
$prefs->set(listen => '127.0.0.1:9443');
$prefs->set(enabled => 0);
require Plugins::LyrionAnywhere::Commands;
my $form = Plugins::LyrionAnywhere::Commands::csrf_token();   # what the page's form sends back
my $params = { saveSettings => 1, anywhere_form_token => $form, pref_listen => '<img src=x onerror=alert(1)>', pref_trusted_proxies => '<b>x</b>' };
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
$params = { saveSettings => 1, anywhere_form_token => $token, pref_listen => '127.0.0.1:9443' };
Plugins::LyrionAnywhere::Settings->handler(undef, $params);
is($params->{anywhere_csrf}, $token, 'also after saving');

# --- CSRF: LMS's own form token is checked only if its "CSRF protection" setting is raised
# (off by default), so the form must carry ours, or nothing is saved. ---------------------
{
	open my $fh, '<', 'Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere/settings/basic.html' or die;
	my $html = do { local $/; <$fh> };
	like($html, qr{<input type="hidden" name="anywhere_form_token" value="\[% anywhere_csrf \| html %\]">}, 'the form carries the token, escaped');
	ok(index($html, 'anywhere_form_token') < index($html, 'pref_enabled'), 'inside the settings form, before the first setting');

	require Plugins::LyrionAnywhere::Gateway;
	my ($restarts, $stops) = (0, 0);
	no warnings 'redefine';
	local *Plugins::LyrionAnywhere::Gateway::restart = sub { $restarts++; 1 };
	local *Plugins::LyrionAnywhere::Gateway::stop = sub { $stops++ };
	my $exe = "$Slim::Utils::Prefs::DIR/evil-gw";
	open $fh, '>', $exe or die; close $fh; chmod 0755, $exe;
	$prefs->set(enabled => 0);
	$prefs->set(listen => '127.0.0.1:9443');
	$prefs->set(public_url => 'https://music.example.com');
	$prefs->set(gateway_path => '');
	Plugins::LyrionAnywhere::Gateway::settings_saved();   # as if the page had saved these
	my %forged = (saveSettings => 1, pref_enabled => 1, pref_listen => '0.0.0.0:9443',
		pref_public_url => 'https://evil.example.com', pref_gateway_path => $exe);
	for my $case (['no token', undef], ['empty token', ''], ['wrong token', 'ab' x 16],
	              ['token with a suffix', "${token}x"], ['token as a list', [$token]], ['upper-case token', uc $token]) {
		my ($name, $given) = @$case;
		my $p = { %forged, (defined $given ? (anywhere_form_token => $given) : ()) };
		@Slim::Web::Settings::HANDLED = ();
		Plugins::LyrionAnywhere::Settings->handler(undef, $p);
		is($prefs->get('enabled'), 0, "$name: Enabled not changed");
		is($prefs->get('listen'), '127.0.0.1:9443', "$name: listen address not changed");
		is($prefs->get('public_url'), 'https://music.example.com', "$name: public URL not changed");
		is($prefs->get('gateway_path'), '', "$name: gateway path not changed");
		ok(!$Slim::Web::Settings::HANDLED[-1]{saveSettings}, "$name: LMS is not asked to save");
		ok(!grep({ /^pref_/ } keys %{ $Slim::Web::Settings::HANDLED[-1] }), "$name: no submitted values passed on");
		like($p->{warning}, qr/not saved.*Reload the page/, "$name: the page says so");
		is($p->{anywhere_csrf}, $token, "$name: the reloaded form gets the right token");
	}
	is($restarts + $stops, 0, 'the gateway was not touched');
	# With the page's token the same form saves.
	Plugins::LyrionAnywhere::Settings->handler(undef, { %forged, anywhere_form_token => $token, pref_listen => '127.0.0.1:9444' });
	is($prefs->get('listen'), '127.0.0.1:9444', 'with the token: saved');
	is($prefs->get('enabled'), 1);
	is($restarts, 1, 'and the gateway restarted');
	# Just showing the page (no save) needs no token.
	my $p = {};
	Plugins::LyrionAnywhere::Settings->handler(undef, $p);
	ok(!$p->{warning}, 'viewing the page: no warning');
	$prefs->set(enabled => 0);
	$prefs->set(listen => '127.0.0.1:9443');
	$prefs->set(gateway_path => '');
}

# --- an http:// public URL is saved only when "Allow http://" is ticked as well -----------
{
	my ($o, $e) = $clean->({ pref_public_url => 'http://music.example.com' });
	ok(!exists $o->{pref_public_url}, 'http:// refused without the box');
	like($e->[0], qr/http:\/\/.*unencrypted.*Allow http/, 'and the reason given');
	($o, $e) = $clean->({ pref_public_url => 'HTTP://music.example.com' });
	ok(!exists $o->{pref_public_url}, 'in any case');
	($o, $e) = $clean->({ pref_public_url => 'http://music.example.com', anywhere_allow_http => 1 });
	is_deeply($e, [], 'allowed with the box ticked');
	is($o->{pref_public_url}, 'http://music.example.com');
	($o, $e) = $clean->({ pref_public_url => 'https://music.example.com' });
	is($o->{pref_public_url}, 'https://music.example.com', 'https needs no box');
	open my $fh, '<', 'Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere/settings/basic.html' or die;
	my $html = do { local $/; <$fh> };
	like($html, qr/name="anywhere_allow_http"/, 'the page has the box');
	like($html, qr/<details class="aw-advanced"[^>]*>\s*<summary>Advanced[^<]*<\/summary>\s*\[% WRAPPER setting title="Gateway path"/, 'the gateway path is under Advanced');
}
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
	            trusted_proxies => '10.0.0.10, 10.0.0.11', admin_port => 9446, gateway_path => $exe,
	            client_ip_header => 'CF-Connecting-IP');
	my %bad = (
		client_ip_header => ['X-Forwarded-For', 'a b', "x\ny"],
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
	for my $name (qw(public_url trusted_proxies gateway_path client_ip_header)) {
		$prefs->set($name, '');
		is($prefs->get($name), '', "pref $name can be cleared");
	}
}

# Browsers cache the page's script and stylesheet for a week: their URLs carry a stamp
# that changes with the files (regression: an old cached script left sections blank).
{
	my $stamp = Plugins::LyrionAnywhere::Settings::assets_stamp();
	like($stamp, qr/\A[1-9]\d*\z/, 'assets stamp is the files\' modification time');
	open my $fh, '<', 'Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere/settings/basic.html' or die;
	my $html = do { local $/; <$fh> };
	like($html, qr{anywhere\.js\?v=\[% anywhere_assets \| uri %\]}, 'script URL is stamped');
	like($html, qr{anywhere\.css\?v=\[% anywhere_assets \| uri %\]}, 'stylesheet URL is stamped');
	like($html, qr{qr\.js\?v=\[% anywhere_assets \| uri %\]}, 'QR encoder URL is stamped');
	like($html, qr{qr\.js[^\n]*\n<script src="/plugins/LyrionAnywhere/settings/anywhere\.js}, 'the encoder loads before the page script');
}

# "Add a device": the page gets the public URL for the QR code's link, only as an escaped
# data attribute (the script checks it is an https address before using it).
{
	$prefs->set(public_url => 'https://music.example.com');
	Plugins::LyrionAnywhere::Gateway::settings_saved();
	my $params = {};
	Plugins::LyrionAnywhere::Settings->handler(undef, $params);
	is($params->{anywhere_public_url}, 'https://music.example.com', 'page is given the public URL');
	$prefs->set(public_url => '');
	Plugins::LyrionAnywhere::Gateway::settings_saved();
	$params = {};
	Plugins::LyrionAnywhere::Settings->handler(undef, $params);
	is($params->{anywhere_public_url}, '', 'empty when unset');
	open my $fh, '<', 'Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere/settings/basic.html' or die;
	my $html = do { local $/; <$fh> };
	like($html, qr/data-public-url="\[% anywhere_public_url \| html %\]"/, 'public URL goes in an escaped data attribute');
	like($html, qr/id="aw-pair"[^>]*hidden/, 'the pairing panel starts hidden');
}

done_testing;
