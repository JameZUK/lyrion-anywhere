package Plugins::LyrionAnywhere::Settings;

# Settings page. Gateway options are ordinary LMS prefs, cleaned before they are saved, and
# the security-relevant ones are also recorded where LMS's `pref` command cannot change them
# (see Gateway::authorised_values). Accounts, devices and the audit log are managed live
# through `anywhere …` commands (see Commands.pm and the page's script).

use strict;
use warnings;
use base qw(Slim::Web::Settings);

use File::Basename ();
use File::Spec;
use Slim::Utils::Log;
use Slim::Utils::Prefs;

use Plugins::LyrionAnywhere::Commands;
use Plugins::LyrionAnywhere::Gateway;

my $log   = logger('plugin.lyrionanywhere');
my $prefs = preferences('plugin.lyrionanywhere');

sub name { 'PLUGIN_LYRIONANYWHERE' }
sub page { 'plugins/LyrionAnywhere/settings/basic.html' }

sub prefs {
	return ($prefs, qw(enabled listen public_url trusted_proxies trusted_networks client_ip_header gateway_path transcode ffmpeg_path download_cache_mb analysis));
}

sub escape_html {
	my $s = shift // '';
	$s =~ s/&/&amp;/g;
	$s =~ s/</&lt;/g;
	$s =~ s/>/&gt;/g;
	$s =~ s/"/&quot;/g;
	$s =~ s/'/&#39;/g;
	return $s;
}

sub _trim {
	my $s = shift // '';
	$s =~ s/\A\s+|\s+\z//g;
	return $s;
}

# Clean submitted values. Returns (\%clean, \@errors, \@notices, \%keep); invalid values keep
# the old pref. Notices point out a setting that is saved but will not work as it stands.
# %keep holds a value that is not saved but is put back in its field, so the person can
# finish what they started (an http:// Public URL waiting for its box to be ticked).
sub clean {
	my ($params) = @_;
	my (%out, @errors, @notices, %keep);
	for my $k (qw(listen public_url trusted_proxies trusted_networks client_ip_header gateway_path ffmpeg_path download_cache_mb)) {
		my $name = "pref_$k";
		next unless exists $params->{$name};
		my $v = _trim($params->{$name});
		if ($v =~ /[\r\n\x00]/) {
			push @errors, "$k: line breaks are not allowed";
			next;
		}
		if ($k eq 'listen') {
			$v ||= '127.0.0.1:9443';
			unless (Plugins::LyrionAnywhere::Gateway::valid_hostport($v)) {
				push @errors, 'Listen address must be IP:port, e.g. 127.0.0.1:9443';
				next;
			}
		}
		elsif ($k eq 'public_url') {
			$v =~ s{/+\z}{};
			if (length $v && !Plugins::LyrionAnywhere::Gateway::valid_origin($v)) {
				push @errors, 'Public URL must look like https://music.example.com (no path)';
				next;
			}
			# Over plain http, passwords and sign-in tokens cross the internet unencrypted:
			# setting or changing such an address needs the box saying so ticked as well. The
			# address already saved needs no tick (so Apply for another setting works), and the
			# box is never shown ticked: a change to a different http:// address always needs a
			# fresh confirmation.
			if ($v =~ m{\Ahttp://}i && $v ne ($prefs->get('public_url') // '') && !$params->{anywhere_allow_http}) {
				push @errors, 'Public URL not saved: an http:// address sends passwords unencrypted. Use https://, or tick "Allow http://" below the address to save it anyway.';
				# Left in the field, with the box under it, so a retype is not needed.
				$keep{pref_public_url} = $v;
				next;
			}
		}
		elsif ($k eq 'trusted_proxies') {
			my @ips = Plugins::LyrionAnywhere::Gateway::split_list($v);
			my @bad = grep { !Plugins::LyrionAnywhere::Gateway::valid_ip($_) } @ips;
			if (@bad) {
				push @errors, 'Trusted proxies must be IP addresses: ' . join(', ', @bad);
				next;
			}
			$v = join ', ', @ips;
		}
		elsif ($k eq 'trusted_networks') {
			# Every entry must be a private network, or nothing is saved.
			my (@nets, @bad);
			for my $entry (Plugins::LyrionAnywhere::Gateway::split_list($v)) {
				my ($net, $why) = Plugins::LyrionAnywhere::Gateway::valid_trusted_network($entry);
				if (defined $net) { push @nets, $net }
				else              { push @bad, $why }
			}
			if (@bad) {
				push @errors, 'Trusted networks not saved: ' . join('; ', @bad) . '.';
				next;
			}
			$v = join ', ', @nets;
		}
		elsif ($k eq 'client_ip_header') {
			unless (Plugins::LyrionAnywhere::Gateway::valid_client_ip_header($v)) {
				push @errors, 'Client address header must be a header name such as CF-Connecting-IP (or empty)';
				next;
			}
		}
		elsif ($k eq 'gateway_path') {
			unless (Plugins::LyrionAnywhere::Gateway::valid_gateway_path($v)) {
				push @errors, 'Gateway path must be an absolute path to an executable file (or empty)';
				next;
			}
		}
		elsif ($k eq 'ffmpeg_path') {
			unless (Plugins::LyrionAnywhere::Gateway::valid_ffmpeg_path($v)) {
				push @errors, 'ffmpeg must be an absolute path to an executable file named ffmpeg (or empty to look on PATH)';
				next;
			}
		}
		elsif ($k eq 'download_cache_mb') {
			$v = 2048 if $v eq '';
			unless (Plugins::LyrionAnywhere::Gateway::valid_cache_mb($v)) {
				push @errors, 'Download cache must be a whole number of MiB from 16 to 1048576';
				next;
			}
		}
		$out{$name} = $v;
	}
	if (exists $params->{pref_enabled}) {
		$out{pref_enabled} = $params->{pref_enabled} ? 1 : 0;
	}
	if (exists $params->{pref_transcode}) {
		$out{pref_transcode} = $params->{pref_transcode} ? 1 : 0;
	}
	if (exists $params->{pref_analysis}) {
		$out{pref_analysis} = $params->{pref_analysis} ? 1 : 0;
	}
	# What the gateway will run with once this is saved.
	my $after = sub { my $k = shift; exists $out{"pref_$k"} ? $out{"pref_$k"} : ($prefs->get($k) // '') };
	my ($networks, $listen, $public) = map { $after->($_) } qw(trusted_networks listen public_url);
	if ($networks ne '') {
		if (Plugins::LyrionAnywhere::Gateway::is_loopback($listen)) {
			push @notices, "Trusted networks are saved, but the gateway listens on $listen, which devices on "
				. 'your VPN cannot reach. Set the Listen address to the VPN interface\'s address, such as 10.8.0.1:9443.';
		}
		elsif ($listen =~ /^(?:0\.0\.0\.0|\[::\]):\d+\z/) {
			push @notices, "Trusted networks are saved, but the gateway listens on every network ($listen). "
				. 'Set the Listen address to the VPN interface\'s address, such as 10.8.0.1:9443, so that only the VPN reaches it.';
		}
	}
	# Said once, when such an address is saved: the box is a confirmation, not a setting, so
	# this is what tells the person it worked (the page then shows the box ticked).
	if (exists $out{pref_public_url} && $public ne ($prefs->get('public_url') // '') && $public =~ m{\Ahttp://}i) {
		push @notices, "Public URL saved as $public. Connections to it are unencrypted, so keep it to a private "
			. 'address reached only over your VPN (see Trusted networks).';
		if (!defined Plugins::LyrionAnywhere::Gateway::private_http_origin($public)) {
			push @notices, 'It is an http:// address with a host name or a public address, so Add a device '
				. 'shows codes without a QR code. Pairing links over http:// need a private address, such as http://10.8.0.1:9443.';
		}
	}
	return (\%out, \@errors, \@notices, \%keep);
}

# The latest modification time of the settings page's own files, as digits only.
sub assets_stamp {
	my $dir = File::Spec->catdir(File::Basename::dirname(__FILE__), 'HTML', 'EN', 'plugins', 'LyrionAnywhere', 'settings');
	my $latest = 0;
	for my $f (qw(anywhere.js qr.js anywhere.css basic.html)) {
		my $m = (stat(File::Spec->catfile($dir, $f)))[9] // 0;
		$latest = $m if $m > $latest;
	}
	return $latest =~ /\A\d+\z/ ? $latest : 0;
}

sub handler {
	my ($class, $client, $params) = @_;
	# The token the form came back with (read before the page's own copy is set below).
	my $form_token = $params->{anywhere_form_token};
	# Only this handler hands a refused address back to the page (below): one arriving with
	# the request, from a crafted link or another site's form, must not reach the field.
	delete $params->{anywhere_keep_public_url};
	# The page's script needs this to change accounts (see Commands.pm), and the form sends
	# it back when saving. Only this page, which other web sites cannot read, carries it.
	$params->{anywhere_csrf} = Plugins::LyrionAnywhere::Commands::csrf_token();
	# LMS serves the page's script and stylesheet with a week-long cache; stamping their
	# URLs with the files' modification time makes browsers fetch them again after an update.
	$params->{anywhere_assets} = assets_stamp();
	# Gateway settings changed outside this page (LMS's `pref` command) are put back before
	# the page shows them, so it never displays, or saves again, a value it did not set.
	Plugins::LyrionAnywhere::Gateway::authorised_values();
	# The address the apps use, for pairing QR codes (the page checks it again).
	$params->{anywhere_public_url} = $prefs->get('public_url') // '';
	$params->{anywhere_gateway_path_allowed} =
		($ENV{ Plugins::LyrionAnywhere::Gateway::ALLOW_GATEWAY_PATH_ENV() } // '') eq '1' ? 1 : 0;
	if ($params->{saveSettings} && !Plugins::LyrionAnywhere::Commands::token_ok($form_token)) {
		# LMS checks its own anti-CSRF token only if the server's "CSRF protection" setting
		# is raised (it is off by default), so any web page could otherwise post this form
		# and, say, point the gateway at a different binary. Nothing is saved without ours.
		$log->warn('refused a settings save without the settings page\'s token');
		delete $params->{$_} for grep { /^pref_/ } keys %$params;
		delete $params->{saveSettings};
		$params->{warning} = 'Settings not saved: the form was out of date (for example, LMS restarted after the page was opened). Reload the page and try again.';
		return $class->SUPER::handler($client, $params);
	}
	if ($params->{saveSettings}) {
		my %before = map { $_ => $prefs->get($_) } qw(enabled listen public_url trusted_proxies trusted_networks client_ip_header gateway_path transcode ffmpeg_path download_cache_mb analysis);
		my ($clean, $errors, $notices, $keep) = clean($params);
		# Invalid values are not saved; the others are.
		for my $k (keys %$params) {
			delete $params->{$k} if $k =~ /^pref_/ && !exists $clean->{$k};
		}
		$params->{$_} = $clean->{$_} for keys %$clean;
		# A rejected http:// Public URL stays in its field (the template reads this, never a
		# pref_* name, which LMS would save): the warning and its box stay on the page.
		$params->{anywhere_keep_public_url} = $keep->{pref_public_url} if exists $keep->{pref_public_url};
		$params->{pref_enabled} //= 0;   # an unticked checkbox is not submitted
		$params->{pref_transcode} //= 0;
		$params->{pref_analysis} //= 0;
		# Messages can echo submitted text: escape before they reach the page.
		# Notices are saved settings that won't work as they stand: shown after any errors.
		my @lines = (@$errors, @$notices);
		$params->{warning} = join('<br>', map { escape_html($_) } @lines) if @lines;
		my $result = $class->SUPER::handler($client, $params);
		# This page is the one authorised writer: what it saved is what the gateway runs with.
		Plugins::LyrionAnywhere::Gateway::settings_saved();
		my $changed = grep { ($before{$_} // '') ne ($prefs->get($_) // '') } keys %before;
		if ($changed) {
			if ($prefs->get('enabled')) { Plugins::LyrionAnywhere::Gateway->restart }
			else                        { Plugins::LyrionAnywhere::Gateway->stop }
		}
		return $result;
	}
	return $class->SUPER::handler($client, $params);
}

1;
