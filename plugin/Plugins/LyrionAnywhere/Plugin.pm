package Plugins::LyrionAnywhere::Plugin;

# Lyrion Anywhere: secure remote access to this server through the anywhere-gateway
# (https://github.com/JameZUK/lyrion-anywhere). The plugin supervises the gateway and
# provides its settings and account management.

use strict;
use warnings;
use base qw(Slim::Plugin::Base);

use Slim::Utils::Log;
use Slim::Utils::Prefs;
use Slim::Utils::Timers;

use Plugins::LyrionAnywhere::Commands;
use Plugins::LyrionAnywhere::Gateway;

use File::Basename ();
use File::Spec;

# The version has one source: install.xml, which LMS and the release job read too.
sub version_from {
	my $file = shift;
	open my $fh, '<', $file or return 'unknown';
	local $/;
	my $xml = <$fh> // '';
	close $fh;
	return $xml =~ m{<version>\s*([0-9]+(?:\.[0-9]+)*)\s*</version>} ? $1 : 'unknown';
}

use constant VERSION => version_from(File::Spec->catfile(File::Basename::dirname(__FILE__), 'install.xml'));

my $log = Slim::Utils::Log->addLogCategory({
	category     => 'plugin.lyrionanywhere',
	defaultLevel => 'INFO',
	description  => 'PLUGIN_LYRIONANYWHERE',
});

my $prefs = preferences('plugin.lyrionanywhere');

sub initPlugin {
	my $class = shift;
	$prefs->init({
		enabled         => 0,
		listen          => '127.0.0.1:9443',
		admin_port      => 9445,
		public_url      => '',
		trusted_proxies => '',
		trusted_networks => '',
		client_ip_header => '',
		gateway_path    => '',
		transcode       => 1,
		ffmpeg_path     => '',
		download_cache_mb => 2048,
		analysis        => 1,
	});
	# The same checks as the settings page, for changes made with LMS's `pref` command.
	Plugins::LyrionAnywhere::Gateway::register_validators();
	$class->SUPER::initPlugin(@_);

	if (main::WEBUI) {
		require Plugins::LyrionAnywhere::Settings;
		Plugins::LyrionAnywhere::Settings->new;
	}
	Plugins::LyrionAnywhere::Commands::register();

	# Give LMS a moment to finish starting before the gateway connects to it.
	my $basedir = $class->_pluginDataFor('basedir');
	Plugins::LyrionAnywhere::Gateway->set_basedir($basedir);
	Slim::Utils::Timers::setTimer(undef, time() + 5, sub {
		Plugins::LyrionAnywhere::Gateway->init($basedir);
	});
	$log->info('Lyrion Anywhere ' . VERSION . ' initialised');
}

sub shutdownPlugin {
	Plugins::LyrionAnywhere::Gateway->stop_for_shutdown;
}

1;
