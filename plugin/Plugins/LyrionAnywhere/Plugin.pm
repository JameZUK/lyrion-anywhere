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

use constant VERSION => '0.1.0';

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
		gateway_path    => '',
	});
	$class->SUPER::initPlugin(@_);

	if (main::WEBUI) {
		require Plugins::LyrionAnywhere::Settings;
		Plugins::LyrionAnywhere::Settings->new;
	}
	Plugins::LyrionAnywhere::Commands::register();

	# Give LMS a moment to finish starting before the gateway connects to it.
	my $basedir = $class->_pluginDataFor('basedir');
	Slim::Utils::Timers::setTimer(undef, time() + 5, sub {
		Plugins::LyrionAnywhere::Gateway->init($basedir);
	});
	$log->info('Lyrion Anywhere ' . VERSION . ' initialised');
}

sub shutdownPlugin {
	Plugins::LyrionAnywhere::Gateway->stop;
}

1;
