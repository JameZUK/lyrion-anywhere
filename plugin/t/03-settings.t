# Settings: submitted values are cleaned; bad ones are rejected with escaped messages.
use strict; use warnings;
use Test::More;
use File::Temp qw(tempdir);
BEGIN { package main; use constant WEBUI => 1; }
use Plugins::LyrionAnywhere::Settings;
use Slim::Utils::Prefs;
sub show { my $s = $_[0] // 'undef'; $s = ref $s if ref $s; $s =~ s/([^\x20-\x7e])/sprintf('\\x{%x}', ord $1)/ge; $s }

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

# --- a rejected http:// Public URL stays on the page with its box; the box shows ticked while
# the saved address is http://; saving one is confirmed (a forum report, 2026-10-25: "the tick
# disappears when I click Apply") ---------------------------------------------------------
{
	# clean(): the typed address comes back in %keep, cleaned as it would have been saved.
	my ($o, $e, $n, $k) = $clean->({ pref_public_url => ' http://music.example.com/ ' });
	ok(!exists $o->{pref_public_url}, 'not saved without the box');
	is($k->{pref_public_url}, 'http://music.example.com', 'but kept for the field, trimmed as it would be saved');
	like($e->[0], qr/below the address/, 'and the error says where the box is');
	($o, $e, $n, $k) = $clean->({ pref_public_url => 'http://music.example.com', anywhere_allow_http => 1 });
	is_deeply($k, {}, 'nothing kept when it is saved');
	($o, $e, $n, $k) = $clean->({ pref_public_url => 'https://music.example.com' });
	is_deeply($k, {}, 'nor for https');
	($o, $e, $n, $k) = $clean->({ pref_public_url => 'http://bad url' });
	is_deeply($k, {}, 'an address that is not one is not kept either');
	($o, $e, $n, $k) = $clean->({ pref_listen => 'nonsense' });
	is_deeply($k, {}, 'other fields are never kept');

	# The handler hands it to the page under its own name, never as a pref_ value LMS would save.
	require Plugins::LyrionAnywhere::Gateway;
	no warnings 'redefine';
	local *Plugins::LyrionAnywhere::Gateway::restart = sub { 1 };
	local *Plugins::LyrionAnywhere::Gateway::stop = sub { };
	$prefs->set(enabled => 0);
	$prefs->set(public_url => '');
	Plugins::LyrionAnywhere::Gateway::settings_saved();
	my $p = { saveSettings => 1, anywhere_form_token => $token, pref_public_url => 'http://music.example.com' };
	@Slim::Web::Settings::HANDLED = ();
	Plugins::LyrionAnywhere::Settings->handler(undef, $p);
	is($prefs->get('public_url'), '', 'without the box: not saved');
	is($p->{anywhere_keep_public_url}, 'http://music.example.com', 'the typed address is handed to the page');
	ok(!exists $Slim::Web::Settings::HANDLED[-1]{pref_public_url}, 'and not as a pref LMS would save');
	like($p->{warning}, qr/not saved.*Allow http/, 'with the error');
	$p = { saveSettings => 1, anywhere_form_token => $token, pref_public_url => 'http://music.example.com', anywhere_allow_http => 1 };
	Plugins::LyrionAnywhere::Settings->handler(undef, $p);
	is($prefs->get('public_url'), 'http://music.example.com', 'with the box: saved');
	ok(!exists $p->{anywhere_keep_public_url}, 'nothing to put back');
	like($p->{warning}, qr/Public URL saved as http:\/\/music\.example\.com\. Connections to it are unencrypted/, 'and the page says it was saved');
	$p = {};
	Plugins::LyrionAnywhere::Settings->handler(undef, $p);
	ok(!exists $p->{anywhere_keep_public_url}, 'viewing the page: nothing kept');
	$prefs->set(public_url => '');

	# The template: the field and the warning follow the kept address, the box the saved one.
	open my $fh, '<', 'Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere/settings/basic.html' or die;
	my $html = do { local $/; <$fh> };
	like($html, qr/\[% shown_public_url = anywhere_keep_public_url \|\| prefs\.pref_public_url %\]/, 'the shown address is the kept one, else the saved one');
	like($html, qr/name="pref_public_url"[^>]*value="\[% shown_public_url \| html %\]"/, 'the field shows it, escaped');
	like($html, qr/id="aw-http-warning"[^>]*\[% UNLESS shown_public_url\.match\('\^\(\?i\)http:\/\/'\) %\] hidden\[% END %\]/, 'the warning is shown for it');
	like($html, qr/name="anywhere_allow_http"[^>]*\[% IF prefs\.pref_public_url\.match\('\^\(\?i\)http:\/\/'\) %\] checked\[% END %\]/, 'the box is ticked while the saved address is http://');
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

# --- trusted networks (docs/plans/trusted-networks.md): the same rule as core::privnet ------
# The vectors are shared with the gateway's Rust check and the web app's JavaScript check.
my $vectors = do {
	require JSON::PP;
	require File::Basename;
	my $file = File::Basename::dirname(__FILE__) . '/../../web/app/tests/fixtures/private-networks.json';
	open my $fh, '<:raw', $file or die "$file: $!";
	JSON::PP::decode_json(do { local $/; <$fh> });
};
{
	my $net = \&Plugins::LyrionAnywhere::Gateway::valid_trusted_network;
	is_deeply([ Plugins::LyrionAnywhere::Gateway::private_ranges() ], $vectors->{ranges}, 'the private ranges are the vectors\' ranges');
	for my $case (@{ $vectors->{networks}{accepted} }) {
		my ($got, $why) = $net->($case->{input});
		is($got, $case->{normalised}, "accepted: '$case->{input}'") or diag $why;
		is(($net->($case->{normalised}))[0], $case->{normalised}, "its normalised form parses to itself: $case->{normalised}");
	}
	for my $input (@{ $vectors->{networks}{refused} }) {
		my ($got, $why) = $net->($input);
		ok(!defined $got, "refused: '$input'");
		ok(defined $why && length $why && $why !~ /\.\z/, "with a reason: " . ($why // 'none'));
	}
	for my $ip (@{ $vectors->{addresses}{private} }) {
		ok(Plugins::LyrionAnywhere::Gateway::is_private_ip($ip), "$ip is private");
		ok(defined(($net->($ip))[0]), "and a trusted network of its own");
	}
	for my $ip (@{ $vectors->{addresses}{not_private} }) {
		ok(!Plugins::LyrionAnywhere::Gateway::is_private_ip($ip), "$ip is not private");
		ok(!defined(($net->($ip))[0]), "and never a trusted network");
	}
	# The reasons, as the gateway gives them.
	like(($net->('127.0.0.1'))[1], qr/loopback/, 'loopback: said so');
	like(($net->('::1'))[1], qr/loopback/, 'IPv6 loopback: said so');
	like(($net->('0.0.0.0/0'))[1], qr/^0\.0\.0\.0\/0 includes loopback/, 'the catch-all includes loopback');
	like(($net->('8.8.8.0/24'))[1], qr/^8\.8\.8\.0\/24 is not a private network: only networks within 10\.0\.0\.0\/8/, 'public: said so');
	like(($net->('::ffff:10.0.0.0/104'))[1], qr/written as IPv6/, 'mapped IPv6: said so');
	like(($net->('10.8.0.0/33'))[1], qr/^"10\.8\.0\.0\/33" has a prefix length longer/, 'prefix: said so');
	like(($net->('10.8.0/24'))[1], qr/^"10\.8\.0\/24" is not a network/, 'syntax: said so, quoting the input');
	like(($net->('   '))[1], qr/^enter a network/, 'empty: said so');
	# IPv6 is written as the gateway writes it (Rust's Display): lower case, the longest run of
	# zeros compressed (the first of two as long), never a single zero group.
	my %display = (
		'fd00:0:0:1:0:0:0:0/64'    => 'fd00:0:0:1::/64',
		'fd00:0:1:0:0:1:0:0'       => 'fd00:0:1::1:0:0/128',
		'fd00:1:0:1:1:1:1:1'       => 'fd00:1:0:1:1:1:1:1/128',
		'fd01::10.0.0.1/128'       => 'fd01::a00:1/128',
		'FE80:0000:0000:0000::/64' => 'fe80::/64',
		'fd00::1:2:3:4:5:6'        => 'fd00:0:1:2:3:4:5:6/128',
		'fdff:ffff:ffff:ffff:ffff:ffff:ffff:ffff/7' => 'fc00::/7',
		'10.255.255.255/9'         => '10.128.0.0/9',
		'172.20.1.2/0'             => undef,
	);
	for my $in (sort keys %display) {
		is(($net->($in))[0], $display{$in}, "normalised: $in");
	}
	# Unusual text that must be refused, never read some other way.
	for my $bad ('010.8.0.0/24', '10.8.0.0/024x', '10.8.0.0/1000', '10.8.0.0/+1', '10.8.0.0/ 24', 'fe80::1%eth0', 'fe80::1%25eth0/64',
	             '[fd00::]/8', 'fd00::/08x', '1:2:3:4:5:6:7:8:9', '1::2::3', ':fd00::1', 'fd00::1:', 'fd00:12345::/32', 'fd00::g',
	             '1:2:3:4:5:6:7:8::', '::1.2.3', 'fd00::1.2.3.4.5', 'fd00:1.2.3.4::', "10.8.0.0/24\x00", "10.8.0.0/24\x{a0}x",
	             "\x{0661}0.8.0.0/24", "\x{967}\x{966}.\x{96e}.\x{966}.\x{966}/\x{968}\x{96a}", "10.8.0.0/\x{663}", ['10.8.0.0/24'], { a => 1 }, undef) {
		my ($got, $why) = $net->($bad);
		ok(!defined $got && defined $why, 'refused: ' . show($bad));
	}
	# Surrounding whitespace is trimmed, as the gateway trims it; the setting itself refuses
	# line breaks (see below), so one never reaches the gateway.
	is(($net->("\t10.8.0.0/24\n"))[0], '10.8.0.0/24', 'surrounding whitespace is trimmed');
	# Leading zeros in a prefix are read as the gateway reads them (Rust's u8 parse): /08 is /8.
	is(($net->('fd00::/08'))[0], 'fd00::/8', 'a prefix with a leading zero is read as a number');
	# Robustness: random and long input never dies, and is refused unless it really is a network.
	srand(20261024);
	my @alphabet = ('0' .. '9', 'a' .. 'f', 'A', 'F', ':', '.', '/', ' ', ',', '%', '[', ']', '-', "\n", "\x{663}");
	my ($died, $wrong) = (0, 0);
	for (1 .. 5000) {
		my $s = join '', map { $alphabet[rand @alphabet] } 1 .. 1 + int rand 24;
		my @r = eval { $net->($s) };
		$died++, diag(show($s)) if $@;
		$wrong++, diag(show($s)) if defined $r[0] && !Plugins::LyrionAnywhere::Gateway::valid_trusted_networks($r[0]);
		eval { Plugins::LyrionAnywhere::Gateway::is_private_ip($s); Plugins::LyrionAnywhere::Gateway::private_http_origin("http://$s"); 1 }
			or $died++;
	}
	is($died, 0, 'random input: the checks never die');
	is($wrong, 0, 'random input: anything accepted is a valid network');
	for my $long ('1' x 10_000, '10.8.0.0/' . '1' x 10_000, 'fd00:' x 5000, ('10.8.0.0/24, ' x 1000)) {
		ok(!defined(($net->($long))[0]), 'a long input is refused');
		ok(!Plugins::LyrionAnywhere::Gateway::valid_trusted_networks($long), 'also as the whole setting');
	}
	# The whole setting: a comma/space separated list, all valid or refused.
	my $list = \&Plugins::LyrionAnywhere::Gateway::valid_trusted_networks;
	ok($list->($_), "setting accepted: '$_'") for ('', '10.8.0.0/24', '10.8.0.0/24, fd00::/8', '10.8.0.0/24 192.168.7.0/24', ' 100.64.0.0/10 ,');
	ok(!$list->($_), 'setting refused: ' . show($_)) for ('10.8.0.0/24, 8.8.8.0/24', 'evil', "10.8.0.0/24\n10.9.0.0/24", "10.8.0.0/24\x00", '127.0.0.1', ['10.8.0.0/24']);
	is_deeply([ Plugins::LyrionAnywhere::Gateway::trusted_network_list('10.8.0.5/24, FD00::/8') ], ['10.8.0.0/24', 'fd00::/8'], 'the list, normalised');
	is_deeply([ Plugins::LyrionAnywhere::Gateway::trusted_network_list('10.8.0.0/24, 8.8.8.0/24') ], [], 'a list with a bad entry gives nothing');
	is_deeply([ Plugins::LyrionAnywhere::Gateway::trusted_network_list(undef) ], [], 'no setting: nothing');
}

# --- the Trusted networks setting on the page ---------------------------------------------
{
	$prefs->set(listen => '10.8.0.1:9443');
	my ($o, $e, $n) = $clean->({ pref_trusted_networks => ' 10.8.0.5/24,FD12:3456:789A::/48  192.168.7.42 ' });
	is_deeply($e, [], 'trusted networks: valid');
	is($o->{pref_trusted_networks}, '10.8.0.0/24, fd12:3456:789a::/48, 192.168.7.42/32', 'stored normalised, comma separated');
	is_deeply($n, [], 'listening on the VPN address: nothing to point out');
	($o, $e) = $clean->({ pref_trusted_networks => '' });
	is_deeply($e, [], 'empty is fine');
	is($o->{pref_trusted_networks}, '', 'none');
	for my $case (
		['10.8.0.0/24, 8.8.8.0/24', qr/8\.8\.8\.0\/24 is not a private network/],
		['0.0.0.0/0', qr/includes loopback/],
		['::/0', qr/includes loopback/],
		['127.0.0.1', qr/loopback/],
		['10.8.0.0/24 music.example.com', qr/"music\.example\.com" is not a network/],
		['http://10.8.0.0/24', qr/"http:\/\/10\.8\.0\.0\/24" is not a network/],
		['::ffff:10.8.0.0/120', qr/written as IPv6/],
		['10.8.0.0/40', qr/prefix length/],
		["10.8.0.0/24\n--trusted-network 0.0.0.0/0", qr/line breaks/],
	) {
		my ($pat, $why) = @$case[0, 1];
		($o, $e) = $clean->({ pref_trusted_networks => $pat });
		ok(!exists $o->{pref_trusted_networks}, 'refused whole: ' . show($pat));
		like(join(' ', @$e), $why, 'naming the entry and why');
		like(join(' ', @$e), qr/Trusted networks not saved|line breaks/, 'saying it was not saved');
	}
	($o, $e) = $clean->({ pref_trusted_networks => '10.8.0.0/24, 8.8.8.0/24, 1.1.1.1' });
	like($e->[0], qr/8\.8\.8\.0\/24.*1\.1\.1\.1\/32/, 'every bad entry is named');

	# The listen address must be one the VPN reaches: a notice, not an error.
	for my $case (['127.0.0.1:9443', qr/listens on 127\.0\.0\.1:9443, which devices on your VPN cannot reach/],
	              ['[::1]:9443', qr/listens on \[::1\]:9443/],
	              ['0.0.0.0:9443', qr/listens on every network \(0\.0\.0\.0:9443\)/],
	              ['[::]:9443', qr/every network/]) {
		my ($listen, $want) = @$case;
		($o, $e, $n) = $clean->({ pref_trusted_networks => '10.8.0.0/24', pref_listen => $listen });
		is_deeply($e, [], "listen $listen with trusted networks: no error");
		is($o->{pref_trusted_networks}, '10.8.0.0/24', 'the networks are saved');
		is($o->{pref_listen}, $listen, 'and the listen address');
		like(join(' ', @$n), $want, "listen $listen: a notice says why it won't work");
	}
	($o, $e, $n) = $clean->({ pref_trusted_networks => '', pref_listen => '127.0.0.1:9443' });
	is_deeply($n, [], 'no trusted networks: no notice about the listen address');
	# Networks saved earlier, the listen address changed now (and the other way round).
	$prefs->set(trusted_networks => '10.8.0.0/24');
	($o, $e, $n) = $clean->({ pref_listen => '127.0.0.1:9443' });
	like(join(' ', @$n), qr/cannot reach/, 'the saved networks count');
	$prefs->set(listen => '127.0.0.1:9443');
	($o, $e, $n) = $clean->({ pref_trusted_networks => '10.8.0.0/24' });
	like(join(' ', @$n), qr/cannot reach/, 'the saved listen address counts');
	($o, $e, $n) = $clean->({ pref_trusted_networks => '10.8.0.0/24', pref_listen => '' });
	like(join(' ', @$n), qr/127\.0\.0\.1:9443/, 'an empty listen address is the loopback default');
	$prefs->set(trusted_networks => '');

	# Through the handler: a bad list keeps the old value and the message is escaped; a notice
	# is shown but the values are saved.
	$prefs->set(trusted_networks => '10.8.0.0/24');
	$prefs->set(listen => '10.8.0.1:9443');
	Plugins::LyrionAnywhere::Gateway::settings_saved();
	my $p = { saveSettings => 1, anywhere_form_token => $form, pref_trusted_networks => '10.9.0.0/24, <script>alert(1)</script>' };
	Plugins::LyrionAnywhere::Settings->handler(undef, $p);
	is($prefs->get('trusted_networks'), '10.8.0.0/24', 'a bad list is not saved: the old value stays');
	unlike($p->{warning}, qr/<script>/, 'the message is escaped');
	like($p->{warning}, qr/Trusted networks not saved: &quot;&lt;script&gt;/, 'and names the entry');
	$p = { saveSettings => 1, anywhere_form_token => $form, pref_trusted_networks => '10.9.0.0/24', pref_listen => '127.0.0.1:9443' };
	Plugins::LyrionAnywhere::Settings->handler(undef, $p);
	is($prefs->get('trusted_networks'), '10.9.0.0/24', 'a good list is saved');
	is($prefs->get('listen'), '127.0.0.1:9443', 'with the loopback listen address');
	like($p->{warning}, qr/cannot reach/, 'and the notice shown');
	is(Plugins::LyrionAnywhere::Gateway::read_authorised()->{trusted_networks}, '10.9.0.0/24', 'the page\'s copy holds it');
	$prefs->set(trusted_networks => '');
	$prefs->set(listen => '127.0.0.1:9443');
	Plugins::LyrionAnywhere::Gateway::settings_saved();

	# The page: the field, and its warning only when the field has something in it.
	open my $fh, '<', 'Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere/settings/basic.html' or die;
	my $html = do { local $/; <$fh> };
	like($html, qr/name="pref_trusted_networks" id="pref_trusted_networks" value="\[% prefs\.pref_trusted_networks \| html %\]"/, 'the field, escaped');
	like($html, qr/<p id="aw-trusted-warning" class="aw-note aw-warning"\[% UNLESS prefs\.pref_trusted_networks %\] hidden\[% END %\]>/, 'the warning is hidden while empty');
	like($html, qr/aw-trusted-warning.*passwords and sign-ins cross them in clear/s, 'it says what crosses in clear');
	like($html, qr/aw-trusted-warning.*not 127\.0\.0\.1.*not 0\.0\.0\.0/s, 'and which listen addresses will not do');
	like($html, qr/<p id="aw-trusted-listen" class="aw-note aw-warning" hidden><\/p>/, 'the live listen note starts hidden');
	like($html, qr/name="pref_listen" id="pref_listen"/, 'the listen field has an id for the script');
	ok(index($html, 'title="Trusted proxies"') < index($html, 'title="Trusted networks"')
		&& index($html, 'title="Trusted networks"') < index($html, 'title="Client address header"'), 'after Trusted proxies');
	my (undef, @names) = Plugins::LyrionAnywhere::Settings::prefs();
	ok(grep({ $_ eq 'trusted_networks' } @names), 'a pref of the page');
}

# --- the Public URL over http:// to a private address (pairing links and QR codes) --------
{
	my $origin = \&Plugins::LyrionAnywhere::Gateway::private_http_origin;
	my %ok = (
		'http://10.8.0.1:9443'         => 'http://10.8.0.1:9443',
		'HTTP://10.8.0.1:9443/'        => 'http://10.8.0.1:9443',
		'http://10.8.0.1'              => 'http://10.8.0.1',
		'http://10.8.0.1:80'           => 'http://10.8.0.1',
		'http://100.101.102.103:09443' => 'http://100.101.102.103:9443',
		'http://[FD00::1]:9443'        => 'http://[fd00::1]:9443',
		'http://[fe80:0:0:0::1]'       => 'http://[fe80::1]',
	);
	is($origin->($_), $ok{$_}, "pairing over http:// allowed: $_") for sort keys %ok;
	for my $ip (@{ $vectors->{addresses}{private} }) {
		my $host = $ip =~ /:/ ? "[$ip]" : $ip;
		ok(defined $origin->("http://$host:9443"), "private address $ip: allowed");
	}
	for my $ip (@{ $vectors->{addresses}{not_private} }) {
		my $host = $ip =~ /:/ ? "[$ip]" : $ip;
		ok(!defined $origin->("http://$host:9443"), "not a private address, $ip: not allowed");
	}
	for my $bad ('https://10.8.0.1:9443', 'http://music.example.com', 'http://lms.example.lan:9443', 'http://10.8.0.1.example.com',
	             'http://8.8.8.8', 'http://127.0.0.1:9443', 'http://10.8.0.1:9443/lms', 'http://10.8.0.1:9443?x', 'http://10.8.0.1:9443#x',
	             'http://user@10.8.0.1', 'http://10.8.0.1:0', 'http://10.8.0.1:70000', 'http://010.8.0.1', 'http://10.8.1',
	             'http://0x0a.8.0.1', 'http://167772161', 'http://[::ffff:10.0.0.1]', 'http://[fe80::1%25eth0]', 'http://fd00::1',
	             "http://10.8.0.1\n", ' http://10.8.0.1', 'ftp://10.8.0.1', 'http://', '', undef, ['http://10.8.0.1']) {
		ok(!defined $origin->($bad), 'pairing over http:// not allowed: ' . show($bad));
	}
	# Saving an http:// address that pairing can't use says so, once (it still needs the box).
	$prefs->set(public_url => '');
	my ($o, $e, $n) = $clean->({ pref_public_url => 'http://music.example.com', anywhere_allow_http => 1 });
	is($o->{pref_public_url}, 'http://music.example.com', 'an http:// name is saved with the box');
	like(join(' ', @$n), qr/without a QR code.*private address/, 'with a notice that pairing links need a private address');
	like($n->[0], qr/^Public URL saved as http:\/\/music\.example\.com\. Connections to it are unencrypted/, 'after saying it is saved and unencrypted');
	is(scalar @$n, 2, 'those two notices');
	($o, $e, $n) = $clean->({ pref_public_url => 'http://10.8.0.1:9443', anywhere_allow_http => 1 });
	is($o->{pref_public_url}, 'http://10.8.0.1:9443', 'an http:// private address is saved with the box');
	is(scalar @$n, 1, 'with one notice');
	like($n->[0], qr/^Public URL saved as http:\/\/10\.8\.0\.1:9443\. Connections to it are unencrypted/, 'that it is saved and unencrypted');
	unlike($n->[0], qr/QR code/, 'and nothing about QR codes: a private address pairs');
	($o, $e, $n) = $clean->({ pref_public_url => 'http://10.8.0.1:9443' });
	ok(!exists $o->{pref_public_url}, 'still not without the box');
	$prefs->set(public_url => 'http://music.example.com');
	($o, $e, $n) = $clean->({ pref_public_url => 'http://music.example.com', anywhere_allow_http => 1 });
	is_deeply($n, [], 'saved again unchanged: not said again');
	$prefs->set(public_url => '');
}

done_testing;
