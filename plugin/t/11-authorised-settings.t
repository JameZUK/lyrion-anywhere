# M1: the gateway's security-relevant settings can be changed only through the settings
# page. LMS's own `pref` command (reachable from any web page through jsonrpc.js, with LMS's
# CSRF check off by default) sets the prefs after a syntax check only; the gateway must
# start with the values the page last saved, put the prefs back and say so in the status.
use strict; use warnings;
use Test::More;
use File::Temp qw(tempdir);
BEGIN { package main; use constant WEBUI => 1; }
use Slim::Utils::Prefs;
use Plugins::LyrionAnywhere::Gateway;
use Plugins::LyrionAnywhere::Settings;
use Plugins::LyrionAnywhere::Commands;

my $G = 'Plugins::LyrionAnywhere::Gateway';
my $prefs = preferences('plugin.lyrionanywhere');
preferences('server')->set(httpport => 9000);
$G->set_basedir(tempdir(CLEANUP => 1));

no warnings 'redefine';
my ($restarts, $stops) = (0, 0);
*Plugins::LyrionAnywhere::Gateway::restart = sub { $restarts++; 1 };
*Plugins::LyrionAnywhere::Gateway::stop = sub { $stops++ };
use warnings 'redefine';

my $token = Plugins::LyrionAnywhere::Commands::csrf_token();

sub fresh {
	$Slim::Utils::Prefs::DIR = tempdir(CLEANUP => 1);
	%{ $Slim::Utils::Prefs::STORE{'plugin.lyrionanywhere'} } = ();
	$prefs->init({ enabled => 0, listen => '127.0.0.1:9443', public_url => '', trusted_proxies => '', trusted_networks => '',
		client_ip_header => '', gateway_path => '', ffmpeg_path => '', download_cache_mb => 2048,
		transcode => 1, analysis => 1, admin_port => 9445 });
	$G->register_validators;
}

sub save_page {
	my %v = @_;
	my $p = { saveSettings => 1, anywhere_form_token => $token, map { ("pref_$_" => $v{$_}) } keys %v };
	Plugins::LyrionAnywhere::Settings->handler(undef, $p);
	return $p;
}

sub copy_file { "$Slim::Utils::Prefs::DIR/lyrion-anywhere/settings.authorised" }

sub argv_has {
	my ($argv, $flag, $value) = @_;
	for my $i (0 .. $#$argv - 1) { return 1 if $argv->[$i] eq $flag && $argv->[$i + 1] eq $value }
	return 0;
}

# --- the page saves, then `pref` changes the values behind its back ----------------------
{
	fresh();
	save_page(enabled => 1, listen => '127.0.0.1:9444', trusted_proxies => '10.0.0.10',
		public_url => 'https://music.example.com', client_ip_header => 'CF-Connecting-IP');
	is($prefs->get('listen'), '127.0.0.1:9444', 'the page saved its values');
	ok(-f copy_file(), 'the page wrote the authoritative copy');
	SKIP: {
		skip 'file modes are not meaningful here', 2 if $^O eq 'MSWin32';
		is((stat copy_file())[2] & 07777, 0600, 'the copy is readable by the LMS user only');
		is((stat "$Slim::Utils::Prefs::DIR/lyrion-anywhere")[2] & 07777, 0700, 'inside the 0700 data dir');
	}
	is($G->state->{settings_notice} // '', '', 'no notice while the prefs match');

	# What `pref plugin.lyrionanywhere:<name> <value>` does: valid syntax, so stored.
	$prefs->set(listen => '0.0.0.0:9443');
	$prefs->set(trusted_proxies => '10.0.0.66');
	$prefs->set(public_url => 'http://music.example.com');
	$prefs->set(client_ip_header => 'X-Real-IP');
	is($prefs->get('listen'), '0.0.0.0:9443', 'the pref command stored the value (the attack)');

	my %o = Plugins::LyrionAnywhere::Gateway::_options();
	is($o{listen}, '127.0.0.1:9444', 'the gateway starts on the page\'s listen address');
	is_deeply($o{trusted_proxies}, ['10.0.0.10'], 'with the page\'s trusted proxies');
	is_deeply($o{allowed_origins}, ['https://music.example.com'], 'and the page\'s public URL');
	is($o{client_ip_header}, 'CF-Connecting-IP', 'and the page\'s client address header');
	my ($argv) = Plugins::LyrionAnywhere::Gateway::command_line(%o, binary => '/srv/gw');
	ok(argv_has($argv, '--listen', '127.0.0.1:9444'), 'command line: page\'s listen address');
	ok(argv_has($argv, '--trusted-proxy', '10.0.0.10'), 'command line: page\'s proxy');
	ok(!argv_has($argv, '--trusted-proxy', '10.0.0.66'), 'command line: never the forged proxy');
	ok(!grep({ $_ eq '0.0.0.0:9443' } @$argv), 'command line: never the forged listen address');

	is($prefs->get('listen'), '127.0.0.1:9444', 'the pref is put back');
	is($prefs->get('trusted_proxies'), '10.0.0.10', 'the proxies pref is put back');
	is($prefs->get('public_url'), 'https://music.example.com', 'the public URL pref is put back');
	my $notice = $G->state->{settings_notice} // '';
	like($notice, qr/outside the .*settings page.*reverted/i, 'the status says so');
	like($notice, qr/listen/, 'naming the setting');
	like($notice, qr/trusted_proxies/, 'each of them');

	# The page shows the page's values, not the forged ones, when opened.
	$prefs->set(listen => '0.0.0.0:9443');
	Plugins::LyrionAnywhere::Settings->handler(undef, {});
	is($prefs->get('listen'), '127.0.0.1:9444', 'opening the page puts the pref back before it is shown');

	# Saving the page records the new values and clears the notice.
	save_page(enabled => 1, listen => '127.0.0.1:9446', trusted_proxies => '10.0.0.11',
		public_url => 'https://music.example.com', client_ip_header => 'CF-Connecting-IP');
	is($G->state->{settings_notice} // '', '', 'a save on the page clears the notice');
	%o = Plugins::LyrionAnywhere::Gateway::_options();
	is($o{listen}, '127.0.0.1:9446', 'the newly saved listen address is used');
	is_deeply($o{trusted_proxies}, ['10.0.0.11'], 'the newly saved proxies are used');
	is($prefs->get('listen'), '127.0.0.1:9446', 'and not reverted');
}

# --- a refused (forged) page save never updates the copy ---------------------------------
{
	Plugins::LyrionAnywhere::Settings->handler(undef, { saveSettings => 1, pref_listen => '0.0.0.0:9443',
		pref_trusted_proxies => '10.0.0.66' });
	my %o = Plugins::LyrionAnywhere::Gateway::_options();
	is($o{listen}, '127.0.0.1:9446', 'a save without the page\'s token changes nothing');
	open my $fh, '<', copy_file() or die;
	my $text = do { local $/; <$fh> };
	unlike($text, qr/0\.0\.0\.0|10\.0\.0\.66/, 'and the copy holds no forged value');
}

# --- the development gateway path and ffmpeg path are guarded too ------------------------
{
	fresh();
	save_page(listen => '127.0.0.1:9443');
	my $exe = "$Slim::Utils::Prefs::DIR/gw";
	open my $fh, '>', $exe or die; close $fh; chmod 0755, $exe;
	local $ENV{ANYWHERE_ALLOW_GATEWAY_PATH} = '1';
	$prefs->set(gateway_path => $exe);
	is($prefs->get('gateway_path'), $exe, 'the pref command stored a gateway path');
	my %o = Plugins::LyrionAnywhere::Gateway::_options();
	isnt($o{binary}, $exe, 'the gateway path set outside the page is never run');
	is($prefs->get('gateway_path'), '', 'and the pref is put back');
	like($G->state->{settings_notice}, qr/gateway_path/, 'the status says so');

	my $ff = "$Slim::Utils::Prefs::DIR/ffmpeg";
	open $fh, '>', $ff or die; close $fh; chmod 0755, $ff;
	$prefs->set(ffmpeg_path => $ff);
	$prefs->set(download_cache_mb => 1048576);
	%o = Plugins::LyrionAnywhere::Gateway::_options();
	is($o{ffmpeg}, '', 'ffmpeg set outside the page is not used');
	is($o{download_cache_mb}, 2048, 'nor the cache size');
}

# --- first run: no copy yet, so the current values are accepted and recorded -------------
{
	fresh();
	ok(!-e copy_file(), 'no copy yet');
	$prefs->set(listen => '127.0.0.1:9450');
	my %o = Plugins::LyrionAnywhere::Gateway::_options();
	is($o{listen}, '127.0.0.1:9450', 'first run: the current value is used');
	ok(-f copy_file(), 'and the copy is created');
	SKIP: {
		skip 'file modes are not meaningful here', 1 if $^O eq 'MSWin32';
		is((stat copy_file())[2] & 07777, 0600, 'readable by the LMS user only');
	}
	is($G->state->{settings_notice} // '', '', 'no notice on the first run');
	$prefs->set(listen => '0.0.0.0:9443');
	%o = Plugins::LyrionAnywhere::Gateway::_options();
	is($o{listen}, '127.0.0.1:9450', 'later changes outside the page are reverted');
}

# --- values survive the round trip, including awkward characters -------------------------
{
	fresh();
	my $dir = "$Slim::Utils::Prefs::DIR/a dir=with%odd chars";
	mkdir $dir or die;
	my $ff = "$dir/ffmpeg";
	open my $fh, '>', $ff or die; close $fh; chmod 0755, $ff;
	save_page(ffmpeg_path => $ff, trusted_proxies => '10.0.0.10, 10.0.0.11');
	my %o = Plugins::LyrionAnywhere::Gateway::_options();
	is($o{ffmpeg}, $ff, 'a path with spaces, = and % comes back unchanged');
	is_deeply($o{trusted_proxies}, ['10.0.0.10', '10.0.0.11'], 'a list comes back unchanged');
	is($G->state->{settings_notice} // '', '', 'and is not mistaken for a change');
}

# --- a copy that is not ours (wrong header) is not trusted, and is replaced ---------------
{
	fresh();
	save_page(listen => '127.0.0.1:9444');
	open my $fh, '>', copy_file() or die;
	print $fh "listen=0.0.0.0:9443\n";
	close $fh;
	my %o = Plugins::LyrionAnywhere::Gateway::_options();
	is($o{listen}, '127.0.0.1:9444', 'an unrecognised copy is ignored: the current pref stands');
	open $fh, '<', copy_file() or die;
	my $text = do { local $/; <$fh> };
	like($text, qr/^listen=127\.0\.0\.1:9444$/m, 'and the copy is rewritten');
}

# --- trusted networks are guarded like the trusted proxies: only the page can widen them ----
{
	fresh();
	ok(scalar(grep { $_ eq 'trusted_networks' } $G->guarded_prefs), 'trusted networks are a guarded setting');
	save_page(enabled => 1, listen => '10.8.0.1:9443', trusted_networks => '10.8.0.0/24');
	is($prefs->get('trusted_networks'), '10.8.0.0/24', 'the page saved the network');
	my %o = Plugins::LyrionAnywhere::Gateway::_options();
	is_deeply($o{trusted_networks}, ['10.8.0.0/24'], 'and the gateway runs with it');

	# `pref` widens it to a valid but larger private network: stored, then put back.
	$prefs->set(trusted_networks => '10.0.0.0/8, 192.168.0.0/16');
	is($prefs->get('trusted_networks'), '10.0.0.0/8, 192.168.0.0/16', 'the pref command stored a wider list (the attack)');
	%o = Plugins::LyrionAnywhere::Gateway::_options();
	is_deeply($o{trusted_networks}, ['10.8.0.0/24'], 'the gateway starts with the page\'s network only');
	my ($argv) = Plugins::LyrionAnywhere::Gateway::command_line(%o, binary => '/srv/gw');
	ok(argv_has($argv, '--trusted-network', '10.8.0.0/24'), 'command line: the page\'s network');
	ok(!argv_has($argv, '--trusted-network', '10.0.0.0/8') && !argv_has($argv, '--trusted-network', '192.168.0.0/16'),
		'command line: never the forged ones');
	is($prefs->get('trusted_networks'), '10.8.0.0/24', 'the pref is put back');
	like($G->state->{settings_notice}, qr/trusted_networks/, 'the status names it');

	# `pref` with something that is not a private network is refused outright.
	for my $bad ('0.0.0.0/0', '8.8.8.0/24', '10.8.0.0/24, ::/0', "10.8.0.0/24\n0.0.0.0/0", 'evil') {
		$prefs->set(trusted_networks => $bad);
		is($prefs->get('trusted_networks'), '10.8.0.0/24', "pref trusted_networks: '" . ($bad =~ s/\n/\\n/r) . "' refused, old value kept");
	}

	# Clearing them on the page turns them off.
	save_page(enabled => 1, listen => '10.8.0.1:9443', trusted_networks => '');
	%o = Plugins::LyrionAnywhere::Gateway::_options();
	is_deeply($o{trusted_networks}, [], 'cleared on the page: none');
	$prefs->set(trusted_networks => '10.8.0.0/24');
	%o = Plugins::LyrionAnywhere::Gateway::_options();
	is_deeply($o{trusted_networks}, [], 'and `pref` cannot put one back');
	is($prefs->get('trusted_networks'), '', 'the pref is put back to empty');
}

# --- a copy written before trusted networks existed: the current value stands -----------
{
	fresh();
	save_page(listen => '127.0.0.1:9443');
	open my $fh, '<', copy_file() or die;
	my $text = do { local $/; <$fh> };
	close $fh;
	$text =~ s/^trusted_networks=.*\n//m;
	open $fh, '>', copy_file() or die;
	print $fh $text;
	close $fh;
	ok(!exists Plugins::LyrionAnywhere::Gateway::read_authorised()->{trusted_networks}, 'an older copy has no trusted networks');
	my %o = Plugins::LyrionAnywhere::Gateway::_options();
	is_deeply($o{trusted_networks}, [], 'after an upgrade: none (the default)');
	is($G->state->{settings_notice} // '', '', 'and no notice');
	ok(exists Plugins::LyrionAnywhere::Gateway::read_authorised()->{trusted_networks}, 'the copy now records the setting');
}

# --- a stored list that is not valid is never passed on, whole --------------------------
{
	fresh();
	save_page(listen => '10.8.0.1:9443', trusted_networks => '10.8.0.0/24');
	open my $fh, '<', copy_file() or die;
	my $text = do { local $/; <$fh> };
	close $fh;
	$text =~ s/^trusted_networks=.*$/trusted_networks=10.8.0.0\/24,%200.0.0.0\/0/m;
	open $fh, '>', copy_file() or die;
	print $fh $text;
	close $fh;
	my %o = Plugins::LyrionAnywhere::Gateway::_options();
	is_deeply($o{trusted_networks}, [], 'a list with a network that is not private is left out whole');
}

done_testing;
