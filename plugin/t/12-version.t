# M18: the plugin's version has one source, install.xml (what LMS and the release job read);
# Plugin.pm's VERSION, used in its log, said 0.2.0 long after install.xml said 0.4.0.
use strict; use warnings;
use Test::More;
use File::Temp qw(tempdir);
BEGIN { package main; use constant WEBUI => 0; }
use Plugins::LyrionAnywhere::Plugin;

open my $fh, '<', 'Plugins/LyrionAnywhere/install.xml' or die "install.xml: $!";
my $xml = do { local $/; <$fh> };
my ($want) = $xml =~ m{<version>([^<]+)</version>};
like($want, qr/^\d+(?:\.\d+)+\z/, 'install.xml has a version');
is(Plugins::LyrionAnywhere::Plugin::VERSION(), $want, 'Plugin.pm VERSION equals install.xml\'s');

my $v = \&Plugins::LyrionAnywhere::Plugin::version_from;
my $dir = tempdir(CLEANUP => 1);
my $write = sub { open my $o, '>', "$dir/install.xml" or die; print $o shift; close $o; "$dir/install.xml" };
is($v->($write->("<extension>\n\t<version>1.2.3</version>\n</extension>\n")), '1.2.3', 'read from the file');
is($v->("$dir/missing.xml"), 'unknown', 'a missing file: unknown, not a crash');
is($v->($write->("<extension><version>1.2.3; evil</version></extension>")), 'unknown', 'anything but digits and dots: unknown');
is($v->($write->('')), 'unknown', 'an empty file: unknown');
done_testing;
