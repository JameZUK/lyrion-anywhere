package Plugins::LyrionAnywhere::Settings;

# Settings page. Gateway options are ordinary LMS prefs, cleaned before they are saved, and
# the security-relevant ones are also recorded where LMS's `pref` command cannot change them
# (see Gateway::authorised_values). Accounts, devices and the audit log are managed live
# through `anywhere …` commands (see Commands.pm and the page's script).

use strict;
use warnings;
use base qw(Slim::Web::Settings);

use Digest::SHA ();
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
	return ($prefs, qw(enabled listen public_url trusted_proxies trusted_networks client_ip_header admin_port gateway_path transcode ffmpeg_path download_cache_mb analysis settings_hosts));
}

# The page's checkboxes: a browser sends nothing for one that is not ticked.
my %CHECKBOX = map { $_ => 1 } qw(enabled transcode analysis);

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
	for my $k (qw(listen public_url trusted_proxies trusted_networks client_ip_header admin_port gateway_path ffmpeg_path download_cache_mb settings_hosts)) {
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
		elsif ($k eq 'admin_port') {
			$v = Plugins::LyrionAnywhere::Gateway::ADMIN_PORT() if $v eq '';
			unless (Plugins::LyrionAnywhere::Gateway::valid_port($v)) {
				push @errors, 'Admin port must be a port number from 1 to 65535 (or empty for ' . Plugins::LyrionAnywhere::Gateway::ADMIN_PORT() . ')';
				next;
			}
			$v += 0;
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
		elsif ($k eq 'settings_hosts') {
			my @names = Plugins::LyrionAnywhere::Gateway::split_list($v);
			my @bad = grep { !Plugins::LyrionAnywhere::Gateway::valid_host_name(Plugins::LyrionAnywhere::Gateway::_plain_host($_)) } @names;
			if (@bad || length($v) > 2048) {
				push @errors, 'Other names for this page must be host names such as lms.example.lan, comma-separated'
					. (@bad ? ': not ' . join(', ', map { substr($_, 0, 64) } @bad) : '');
				next;
			}
			my %seen;
			$v = join ', ', grep { !$seen{$_}++ } Plugins::LyrionAnywhere::Gateway::settings_host_list($v);
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

# A stamp for the settings page's own files, put in their URLs so browsers fetch them again
# when they change (LMS tells browsers to keep plugin files for a week). It is a hash of
# their contents, never a date: the release zip dates every file the same, LMS keeps that
# date when it installs the plugin, and a stamp made of dates left browsers running the
# previous release's script against the new plugin (issue #29). Sixteen hex digits.
sub assets_stamp {
	my $dir = shift // File::Spec->catdir(File::Basename::dirname(__FILE__), 'HTML', 'EN', 'plugins', 'LyrionAnywhere', 'settings');
	my $sha = Digest::SHA->new(256);
	for my $f (qw(anywhere.js qr.js anywhere.css basic.html)) {
		$sha->add("$f\n");
		if (open my $fh, '<:raw', File::Spec->catfile($dir, $f)) {
			$sha->addfile($fh);
			close $fh;
		} else {
			$sha->add("(missing)\n");
		}
		$sha->add("\n");
	}
	return substr($sha->hexdigest, 0, 16);
}

# Where to open the page instead of a refused name: this server's address as the browser's
# connection reached it, if that is not loopback (behind a proxy on this machine it is).
sub address_url {
	my ($class, $http_client) = @_;
	my ($addr, $port) = map { my $m = $_; scalar eval { $http_client->$m } } qw(sockhost sockport);
	return undef unless defined $addr && defined $port && $port =~ /\A[0-9]{1,5}\z/;
	$addr =~ s/\A::ffff:(?=[0-9.]+\z)//i;
	return undef unless Plugins::LyrionAnywhere::Gateway::valid_ip($addr);
	return undef if $addr =~ /\A127\./ || $addr eq '::1' || $addr eq '0.0.0.0' || $addr eq '::';
	my $host = $addr =~ /:/ ? "[$addr]" : $addr;
	return "http://$host:$port/" . $class->page;
}

sub handler {
	my ($class, $client, $params, undef, $http_client) = @_;
	# The token the form came back with (read before the page's own copy is set below).
	my $form_token = $params->{anywhere_form_token};
	# Only this handler hands a refused address back to the page (below), and only it marks a
	# save it accepted: values arriving with the request must not reach the page.
	delete $params->{$_} for qw(anywhere_keep_public_url anywhere_saving anywhere_host_refused anywhere_refused_host
		anywhere_address_url anywhere_own_names);
	# LMS serves the page's script and stylesheet with a week-long cache; stamping their
	# URLs with the files' modification time makes browsers fetch them again after an update.
	$params->{anywhere_assets} = assets_stamp();
	# Gateway settings changed outside this page (LMS's `pref` command) are put back before
	# the page shows them, so it never displays, or saves again, a value it did not set. If
	# the page's record of them cannot be used, the page says so: saving records them again.
	my (undef, $record_problem) = Plugins::LyrionAnywhere::Gateway::authorised_values();
	# DNS rebinding (review 2026-11-12, M14): a web site that points a name of its own at this
	# server could read this page, and its token, as its own. LMS never checks the Host
	# header, so this page does: under any other name it shows only why, and how to open it
	# (at the server's IP address, or under a name the admin lists), saves nothing and gives
	# out no token. ($params->{host} is the request's Host header, set by LMS after the form's
	# own values, so a form field cannot stand in for it.)
	unless (Plugins::LyrionAnywhere::Gateway::settings_host_allowed($params->{host})) {
		my ($name) = Plugins::LyrionAnywhere::Gateway::host_of($params->{host});
		$log->warn('refused the settings page under the name ' . (defined $name ? "'$name'" : 'given')
			. ', which is not this server\'s address or one of its names');
		delete $params->{$_} for grep { /^pref_/ } keys %$params;
		delete $params->{saveSettings};
		$params->{anywhere_csrf} = '';
		$params->{anywhere_host_refused} = 1;
		$params->{anywhere_refused_host} = $name // '';
		$params->{anywhere_address_url} = $class->address_url($http_client) // '';
		$params->{anywhere_own_names} = join ', ', Plugins::LyrionAnywhere::Gateway::own_host_names();
		return $class->SUPER::handler($client, $params);
	}
	# The page's script needs this to change accounts (see Commands.pm), and the form sends
	# it back when saving. Only this page, which other web sites cannot read, carries it.
	$params->{anywhere_csrf} = Plugins::LyrionAnywhere::Commands::csrf_token();
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
		my (undef, @names) = $class->prefs;
		my %before = map { $_ => $prefs->get($_) } @names;
		my ($clean, $errors, $notices, $keep) = clean($params);
		# LMS's base handler sets every pref of the page to its pref_<name> param whether the
		# form sent one or not (a missing one stores undef), and takes an unprefixed <name>
		# param for a missing pref_<name>. So every pref gets its value here: the cleaned one,
		# or, for a value refused or not sent, the saved one. An unticked checkbox is not
		# sent, and means off.
		delete $params->{$_} for grep { /^pref_/ } keys %$params;
		for my $n (@names) {
			delete $params->{$n};
			my $k = "pref_$n";
			$params->{$k} = exists $clean->{$k} ? $clean->{$k} : $CHECKBOX{$n} ? 0 : $before{$n};
		}
		# A rejected http:// Public URL stays in its field (the template reads this, never a
		# pref_* name, which LMS would save): the warning and its box stay on the page.
		$params->{anywhere_keep_public_url} = $keep->{pref_public_url} if exists $keep->{pref_public_url};
		# Messages can echo submitted text: escape before they reach the page.
		# Notices are saved settings that won't work as they stand: shown after any errors.
		my @lines = (@$errors, @$notices);
		$params->{warning} = join('<br>', map { escape_html($_) } @lines) if @lines;
		# LMS saves the prefs, then calls beforeRender (below: the record), then renders.
		$params->{anywhere_saving} = 1;
		my $result = $class->SUPER::handler($client, $params);
		my $changed = grep { ($before{$_} // '') ne ($prefs->get($_) // '') } keys %before;
		if ($changed) {
			if ($prefs->get('enabled')) { Plugins::LyrionAnywhere::Gateway->restart }
			else                        { Plugins::LyrionAnywhere::Gateway->stop }
		}
		return $result;
	}
	$params->{warning} = escape_html(ucfirst $record_problem) if $record_problem && !$params->{warning};
	return $class->SUPER::handler($client, $params);
}

# LMS calls this after saving the prefs and before rendering the page. This page is the one
# authorised writer: what it saved is what the gateway runs with. A save that cannot be
# recorded says so on the page it renders.
sub beforeRender {
	my ($class, $params) = @_;
	return unless $params->{anywhere_saving};
	return if Plugins::LyrionAnywhere::Gateway::settings_saved();
	$params->{warning} = join '<br>', grep { length } $params->{warning} // '',
		escape_html('The settings could not be recorded for the gateway (see the server log), so it keeps running with the ones saved before.');
}

1;
