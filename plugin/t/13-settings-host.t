# DNS rebinding (review 2026-11-12, M14): a web site can point a name of its own at this
# server's LAN address and then read LMS's pages as its own, the settings page and its token
# included. LMS never checks the Host header, so the settings page and the commands that carry
# its token do: they answer only under an IP address, localhost, this machine's own name or a
# name the admin listed on the page. Everything else is refused, saying why and what to do.
use strict; use warnings;
use Test::More;
use File::Temp qw(tempdir);
BEGIN { package main; use constant WEBUI => 1; }
use Slim::Utils::Log;
use Slim::Utils::Prefs;
use Slim::Control::Request;
use Plugins::LyrionAnywhere::Gateway;
use Plugins::LyrionAnywhere::Settings;
use Plugins::LyrionAnywhere::Commands;

sub show { my $s = $_[0] // 'undef'; $s = ref $s if ref $s; $s =~ s/([^\x20-\x7e])/sprintf('\\x{%x}', ord $1)/ge; $s }

my $G = 'Plugins::LyrionAnywhere::Gateway';
my $prefs = preferences('plugin.lyrionanywhere');
preferences('server')->set(httpport => 9000);
$G->set_basedir(tempdir(CLEANUP => 1));
no warnings 'redefine';
*Plugins::LyrionAnywhere::Gateway::restart = sub { 1 };
*Plugins::LyrionAnywhere::Gateway::stop = sub { };
# This machine's name, for the tests (the real one comes from the system).
my $own = 'lms-box.example.lan';
my $real_own = \&Plugins::LyrionAnywhere::Gateway::own_host_names;
*Plugins::LyrionAnywhere::Gateway::own_host_names = sub { @_ ? $real_own->(@_) : $real_own->($own) };
use warnings 'redefine';

my $token = Plugins::LyrionAnywhere::Commands::csrf_token();

sub fresh {
	$Slim::Utils::Prefs::DIR = tempdir(CLEANUP => 1);
	%{ $Slim::Utils::Prefs::STORE{'plugin.lyrionanywhere'} } = ();
	$prefs->init({ enabled => 0, listen => '127.0.0.1:9443', public_url => '', trusted_proxies => '', trusted_networks => '',
		client_ip_header => '', gateway_path => '', ffmpeg_path => '', download_cache_mb => 2048,
		transcode => 1, analysis => 1, admin_port => 9445, settings_hosts => '' });
	$G->register_validators;
}

sub page {
	my ($host, %p) = @_;
	my $p = { (defined $host ? (host => $host) : ()), %p };
	Plugins::LyrionAnywhere::Settings->handler(undef, $p, undef, $p->{_http_client});
	return $p;
}

# --- the Host header, parsed ---------------------------------------------------------------
{
	my %parsed = (
		'10.0.0.10'             => ['10.0.0.10', 1],
		'10.0.0.10:9000'        => ['10.0.0.10', 1],
		'[fd00::1]:9000'        => ['fd00::1', 1],
		'[FD00::1]'             => ['fd00::1', 1],
		'[::ffff:10.0.0.10]:9000' => ['::ffff:10.0.0.10', 1],
		'localhost:9000'        => ['localhost', 0],
		'LMS.Example.LAN:9000'  => ['lms.example.lan', 0],
		'lms.example.lan.:9000' => ['lms.example.lan', 0],
		'010.0.0.1:9000'        => ['010.0.0.1', 0],   # not an address: some parsers read octal
	);
	for my $h (sort keys %parsed) {
		is_deeply([ $G->can('host_of')->($h) ], $parsed{$h}, "Host $h parsed");
	}
	for my $bad (undef, '', ' ', ':9000', '[fd00::1', '[fe80::1%25eth0]:9000', '[fe80::1%eth0]', '[fd00::g]', '[10.0.0.10]',
	             'lms example', "lms\r\nX-Evil: 1", "10.0.0.10\n", 'lms_box', '-lms.example.lan', 'lms-.example.lan', 'lms..example',
	             'user@10.0.0.10', '10.0.0.10:', '10.0.0.10:123456', '10.0.0.10:90a', 'lms.example.lan/x', "\x{0661}0.0.0.1",
	             'a' x 64 . '.example', ('a.' x 150) . 'lan', ['10.0.0.10'], { host => 1 }) {
		is_deeply([ $G->can('host_of')->($bad) ], [], 'not a Host: ' . show($bad));
	}
	srand(20261112);
	my @alphabet = ('a' .. 'f', '0' .. '9', '.', ':', '[', ']', '-', '%', ' ', "\n", 'Z', '@', "\x{663}");
	my $died = 0;
	for (1 .. 5000) {
		my $s = join '', map { $alphabet[rand @alphabet] } 1 .. 1 + int rand 30;
		eval { $G->can('host_of')->($s); $G->can('page_host_allowed')->($s, 'lms'); 1 } or $died++;
	}
	is($died, 0, 'random Host headers: the check never dies');
}

# --- which names the page answers under ------------------------------------------------------
{
	my $ok = $G->can('page_host_allowed');
	my @names = ('lms-box', 'lms-box.local', 'proxy.example.com');
	ok($ok->($_, @names), "allowed: $_") for ('10.0.0.10:9000', '10.0.0.10', '127.0.0.1:9000', '[fd00::1]:9000', '[::1]:9000',
		'localhost:9000', 'LOCALHOST', 'localhost.:9000', 'lms-box:9000', 'LMS-BOX.local:9000', 'proxy.example.com');
	ok(!$ok->($_, @names), 'refused: ' . show($_)) for ('evil.example.com:9000', 'lms-box.evil.example:9000', 'localhost.evil.example',
		'10.0.0.10.evil.example', 'evil.example.com.lms-box', 'proxy.example.com.evil.example', 'example.com', 'local', 'lms',
		'010.0.0.1:9000', '0x0a000001', '167772161', '[fe80::1%25eth0]', '', undef, ['10.0.0.10'], "10.0.0.10\r\n");
	is_deeply([ $real_own->('LMS-Box.example.lan.') ], [qw(lms-box lms-box.example.lan lms-box.local)], 'own names: the host name, its first label, and that under .local');
	is_deeply([ $real_own->('lms-box') ], [qw(lms-box lms-box.local)], 'a host name without a domain');
	is_deeply([ $real_own->('not a name') ], [], 'a host name that is not a DNS name: none');
	is_deeply([ $real_own->(undef) ], [], 'no host name: none');
}

# --- the page under another name: no settings, no token, nothing saved -------------------------
{
	fresh();
	page('10.0.0.10:9000', saveSettings => 1, anywhere_form_token => $token, pref_enabled => 1, pref_listen => '127.0.0.1:9444',
		pref_transcode => 1, pref_analysis => 1);
	is($prefs->get('listen'), '127.0.0.1:9444', 'set up: saved at the address');
	for my $case (
		['a rebinding name', 'evil.example.com:9000'],
		['a name ending in this server\'s', 'lms-box.example.lan.evil.example:9000'],
		['no Host header', undef],
		['a malformed Host', "10.0.0.10\r\nX-Evil: 1"],
	) {
		my ($name, $host) = @$case;
		@Slim::Utils::Log::MESSAGES = ();
		@Slim::Web::Settings::HANDLED = ();
		my $view = page($host);
		is($view->{anywhere_csrf}, '', "$name: viewing gives out no token");
		ok($view->{anywhere_host_refused}, "$name: the page shows the refusal");
		my $save = page($host, saveSettings => 1, anywhere_form_token => $token, pref_enabled => 1, pref_listen => '0.0.0.0:9443',
			pref_transcode => 1, pref_analysis => 1);
		is($prefs->get('listen'), '127.0.0.1:9444', "$name: a save carrying the right token saves nothing");
		ok(!$Slim::Web::Settings::HANDLED[-1]{saveSettings}, "$name: LMS is not asked to save");
		ok(!grep({ /^pref_/ } keys %{ $Slim::Web::Settings::HANDLED[-1] }), "$name: no submitted values passed on");
		is($save->{anywhere_csrf}, '', "$name: and no token in the answer");
		ok(grep({ $_->[0] eq 'warn' && $_->[1] =~ /refused the settings page under the name/ } @Slim::Utils::Log::MESSAGES), "$name: logged");
	}
	my $view = page('evil.example.com:9000');
	is($view->{anywhere_refused_host}, 'evil.example.com', 'the refusal names the name it was opened under');
	is($view->{anywhere_own_names}, 'lms-box, lms-box.example.lan, lms-box.local', 'and this server\'s own names');
	# Where to open it instead: the address the browser's connection reached.
	my $client = sub { my ($a, $p) = @_; bless { a => $a, p => $p }, 'FakeHttpClient' };
	{ package FakeHttpClient; sub sockhost { $_[0]{a} } sub sockport { $_[0]{p} } }
	my %url = (
		'10.0.0.10'        => 'http://10.0.0.10:9000/plugins/LyrionAnywhere/settings/basic.html',
		'::ffff:10.0.0.10' => 'http://10.0.0.10:9000/plugins/LyrionAnywhere/settings/basic.html',
		'fd00::10'         => 'http://[fd00::10]:9000/plugins/LyrionAnywhere/settings/basic.html',
		'127.0.0.1'        => '',   # behind a proxy on this machine: no address to suggest
		'::1'              => '',
		'not-an-address'   => '',
	);
	for my $a (sort keys %url) {
		my $v = page('evil.example.com:9000', _http_client => $client->($a, 9000));
		is($v->{anywhere_address_url}, $url{$a}, "suggested address for a connection to $a");
	}
	is(page('evil.example.com:9000', _http_client => undef)->{anywhere_address_url}, '', 'no connection known: none suggested');
	# Values arriving with the request never reach the page's own fields.
	my $p = page('10.0.0.10:9000', anywhere_host_refused => 1, anywhere_refused_host => '<b>x</b>', anywhere_address_url => 'https://evil.example');
	ok(!$p->{anywhere_host_refused} && !exists $p->{anywhere_refused_host} && !exists $p->{anywhere_address_url},
		'at the address: request values for the refusal fields are dropped');
	is($p->{anywhere_csrf}, $token, 'and the page gets its token');
}

# --- names the admin lists on the page (a reverse proxy, a router-given name) ---------------------
{
	fresh();
	page('10.0.0.10:9000', saveSettings => 1, anywhere_form_token => $token, pref_listen => '127.0.0.1:9443',
		pref_settings_hosts => ' LMS.Example.LAN., proxy.example.com  lms.example.lan ');
	is($prefs->get('settings_hosts'), 'lms.example.lan, proxy.example.com', 'saved lower case, without the final dot, once each');
	is(page('lms.example.lan:9000')->{anywhere_csrf}, $token, 'a listed name opens the page');
	is(page('proxy.example.com')->{anywhere_csrf}, $token, 'so does the other');
	ok(page('other.example.com')->{anywhere_host_refused}, 'an unlisted one is still refused');
	for my $bad ('evil_example', '*.example.com', 'lms.example.lan/x', "lms\nevil", '<script>', 'a' x 64 . '.lan', 'http://lms.example.lan') {
		my $p = page('10.0.0.10:9000', saveSettings => 1, anywhere_form_token => $token, pref_listen => '127.0.0.1:9443', pref_settings_hosts => $bad);
		is($prefs->get('settings_hosts'), 'lms.example.lan, proxy.example.com', 'refused: ' . show($bad));
		like($p->{warning}, qr/Other names for this page must be host names|line breaks/, 'with the reason');
		unlike($p->{warning}, qr/<script>/, 'escaped');
	}
	# `pref` cannot add one: the page's record decides, and the pref is put back.
	$prefs->set(settings_hosts => 'lms.example.lan, proxy.example.com, evil.example.com');
	is($prefs->get('settings_hosts'), 'lms.example.lan, proxy.example.com, evil.example.com', 'the pref command stored a name (the attack)');
	my $p = page('evil.example.com:9000');
	ok($p->{anywhere_host_refused}, 'a name added outside the page does not open it');
	is($p->{anywhere_csrf}, '', 'nor give out the token');
	is($prefs->get('settings_hosts'), 'lms.example.lan, proxy.example.com', 'and the pref is put back');
	like($G->state->{settings_notice}, qr/settings_hosts/, 'the status names it');
	ok(!$G->can('valid_pref')->('settings_hosts', 'evil_example'), 'the pref command is refused a value that is not names');
}

# --- the setting is newer than the record: a value set beforehand is never taken ---------------
{
	fresh();
	$prefs->set(settings_hosts => 'evil.example.com');   # set before the first run
	my ($auth) = $G->authorised_values;
	is($auth->{settings_hosts}, '', 'first run: Other names starts empty, whatever the pref says');
	is($prefs->get('settings_hosts'), '', 'and the pref is put back');
	# An older record (without the setting) and a pref set before the upgrade.
	page('10.0.0.10:9000', saveSettings => 1, anywhere_form_token => $token, pref_listen => '127.0.0.1:9444');
	my $file = $G->settings_file;
	open my $fh, '<', $file or die; my $text = do { local $/; <$fh> }; close $fh;
	$text =~ s/^settings_hosts=.*\n//m;
	open $fh, '>', $file or die; print $fh $text; close $fh;
	$prefs->set(settings_hosts => 'evil.example.com');
	($auth) = $G->authorised_values;
	is($auth->{settings_hosts}, '', 'after an upgrade: empty, not the pref set beforehand');
	is($auth->{listen}, '127.0.0.1:9444', 'while the older settings keep their recorded values');
	ok(page('evil.example.com')->{anywhere_host_refused}, 'so that name does not open the page');
}

# --- commands carrying the page's token, over JSON-RPC under another name -------------------------
{
	fresh();
	page('10.0.0.10:9000', saveSettings => 1, anywhere_form_token => $token, pref_listen => '127.0.0.1:9443',
		pref_settings_hosts => 'lms.example.lan');
	{ package FakeHttpRequest; sub header { $_[1] eq 'Host' ? $_[0]{host} : undef } }
	{ package FakeHttpResponse; sub request { $_[0]{request} } }
	my $n = 0;
	my $rpc = sub {
		my ($host, $words, $params) = @_;
		my $conn = 'conn' . ++$n;
		$Slim::Web::JSONRPC::contexts{$conn} = { httpResponse => bless({ request => bless({ host => $host }, 'FakeHttpRequest') }, 'FakeHttpResponse') }
			if defined $host;
		my $r = Slim::Control::Request->new($words, $params);
		@$r{qw(source connection)} = ('JSONRPC', $conn);
		return $r;
	};
	my @refused = (['a rebinding name', 'evil.example.com:9000'], ['no Host found', undef], ['a malformed Host', "lms\r\n"]);
	for my $case (@refused) {
		my ($name, $host) = @$case;
		my $r = $rpc->($host, ['anywhere', 'users'], { csrf => $token });
		Plugins::LyrionAnywhere::Commands::admin($r);
		like($r->{results}{error}, qr/^refused: open the Lyrion Anywhere settings page at this server's IP address/, "$name: users refused, saying where to open the page");
		$r = $rpc->($host, ['anywhere', 'restart'], { csrf => $token });
		Plugins::LyrionAnywhere::Commands::restart($r);
		like($r->{results}{error}, qr/^refused/, "$name: restart refused");
		$r = $rpc->($host, ['anywhere', 'status'], { csrf => $token });
		Plugins::LyrionAnywhere::Commands::status($r);
		is_deeply([sort keys %{ $r->{results} }], [sort qw(enabled running binary_ok uptime error)], "$name: status gives the public fields only");
	}
	for my $case (['the address', '10.0.0.10:9000'], ['localhost', 'localhost:9000'], ['a listed name', 'lms.example.lan:9000'],
	              ['this server\'s own name', 'lms-box:9000']) {
		my ($name, $host) = @$case;
		my $r = $rpc->($host, ['anywhere', 'status'], { csrf => $token });
		Plugins::LyrionAnywhere::Commands::status($r);
		ok(exists $r->{results}{data_dir}, "$name: status with the token gives everything");
		$r = $rpc->($host, ['anywhere', 'restart'], { csrf => $token });
		Plugins::LyrionAnywhere::Commands::restart($r);
		ok(!defined $r->{results}{error}, "$name: restart accepted") or diag $r->{results}{error};
	}
	# Cometd carries no Host this can check; LMS's telnet CLI has none (and a browser can't
	# read its answers); a request made inside LMS has no source.
	my $r = Slim::Control::Request->new(['anywhere', 'users'], { csrf => $token });
	@$r{qw(source connection)} = ('/slim/request|1|0|abcd1234|ua', 'abcd1234');
	Plugins::LyrionAnywhere::Commands::admin($r);
	like($r->{results}{error}, qr/^refused/, 'over Cometd: refused');
	$r = Slim::Control::Request->new(['anywhere', 'restart'], { csrf => $token });
	@$r{qw(source connection)} = ('CLI', 'socket');
	Plugins::LyrionAnywhere::Commands::restart($r);
	ok(!defined $r->{results}{error}, 'over the CLI: accepted');
	# Without the token the answer is the same as before, whatever the Host.
	$r = $rpc->('evil.example.com', ['anywhere', 'users'], {});
	Plugins::LyrionAnywhere::Commands::admin($r);
	like($r->{results}{error}, qr/^refused: use the Lyrion Anywhere settings page/, 'no token: the usual refusal');
}

# --- the template: the refusal carries no settings, no token and no script ----------------------
{
	open my $fh, '<', 'Plugins/LyrionAnywhere/HTML/EN/plugins/LyrionAnywhere/settings/basic.html' or die;
	my $html = do { local $/; <$fh> };
	my ($refusal, $rest) = $html =~ /\[% IF anywhere_host_refused %\](.*?)\[% ELSE %\](.*)\z/s;
	ok(defined $refusal, 'the page has a refusal branch');
	unlike($refusal, qr/anywhere_csrf|<script|name="pref_|<input/, 'which has no token, field or script');
	like($refusal, qr/\[% anywhere_refused_host \| html %\]/, 'the name is escaped');
	like($refusal, qr/href="\[% anywhere_address_url \| html %\]"/, 'the suggested address too');
	like($refusal, qr/Other names for this page/, 'and it says how to add a name');
	like($rest, qr/anywhere_form_token.*data-csrf.*anywhere\.js.*\[% END %\]\s*\[% PROCESS settings\/footer\.html %\]/s,
		'the settings, the token and the script are in the other branch only');
	ok(index($html, 'anywhere_csrf') > index($html, '[% ELSE %]'), 'the token appears nowhere before it');
}

done_testing;
