package AnywhereTest;
# Shared test helpers.
use strict; use warnings;
use Exporter 'import';
our @EXPORT_OK = qw(start_stub_gateway);

use Plugins::LyrionAnywhere::Gateway;
use Proc::Background;
use Slim::Utils::Prefs;

# Start the gateway through the Proc::Background stub (nothing really runs), so the plugin
# has a gateway it started and knows the admin port of. Uses a development binary path in
# $dir. %o: admin_port (default 9445), pid (a real child process of the test, optional).
sub start_stub_gateway {
	my ($dir, %o) = @_;
	$ENV{ANYWHERE_ALLOW_GATEWAY_PATH} = '1';
	my $bin = "$dir/anywhere-gateway";
	unless (-x $bin) {
		open my $fh, '>', $bin or die "$bin: $!";
		print $fh "#!/bin/sh\n";
		close $fh;
		chmod 0755, $bin;
	}
	my $prefs = preferences('plugin.lyrionanywhere');
	$prefs->set(gateway_path => $bin);
	$prefs->set(listen => '127.0.0.1:9443');
	$prefs->set(admin_port => $o{admin_port} // 9445);
	preferences('server')->set(httpport => 9000);
	$Proc::Background::NEXT_PID = $o{pid};
	Plugins::LyrionAnywhere::Gateway->start;
	return $Proc::Background::OBJECTS[-1];
}

1;
