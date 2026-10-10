package Plugins::LyrionAnywhere::Settings;

# Settings page. Gateway options are ordinary LMS prefs, cleaned before they are saved;
# accounts, devices and the audit log are managed live through `anywhere …` commands
# (see Commands.pm and the page's script).

use strict;
use warnings;
use base qw(Slim::Web::Settings);

use File::Spec::Functions qw(file_name_is_absolute);

use Slim::Utils::Log;
use Slim::Utils::Prefs;

use Plugins::LyrionAnywhere::Gateway;

my $log   = logger('plugin.lyrionanywhere');
my $prefs = preferences('plugin.lyrionanywhere');

sub name { 'PLUGIN_LYRIONANYWHERE' }
sub page { 'plugins/LyrionAnywhere/settings/basic.html' }

sub prefs {
	return ($prefs, qw(enabled listen public_url trusted_proxies gateway_path));
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

# Clean submitted values. Returns (\%clean, \@errors); invalid values keep the old pref.
sub clean {
	my ($params) = @_;
	my (%out, @errors);
	for my $k (qw(listen public_url trusted_proxies gateway_path)) {
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
		elsif ($k eq 'gateway_path') {
			if (length $v && !(file_name_is_absolute($v) && -f $v && -x $v)) {
				push @errors, 'Gateway path must be an absolute path to an executable file (or empty)';
				next;
			}
		}
		$out{$name} = $v;
	}
	if (exists $params->{pref_enabled}) {
		$out{pref_enabled} = $params->{pref_enabled} ? 1 : 0;
	}
	return (\%out, \@errors);
}

sub handler {
	my ($class, $client, $params) = @_;
	if ($params->{saveSettings}) {
		my %before = map { $_ => $prefs->get($_) } qw(enabled listen public_url trusted_proxies gateway_path);
		my ($clean, $errors) = clean($params);
		# Invalid values are not saved; the others are.
		for my $k (keys %$params) {
			delete $params->{$k} if $k =~ /^pref_/ && !exists $clean->{$k};
		}
		$params->{$_} = $clean->{$_} for keys %$clean;
		$params->{pref_enabled} //= 0;   # an unticked checkbox is not submitted
		# Messages can echo submitted text: escape before they reach the page.
		$params->{warning} = join('<br>', map { escape_html($_) } @$errors) if @$errors;
		my $result = $class->SUPER::handler($client, $params);
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
