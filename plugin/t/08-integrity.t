# The bundled gateway runs only if it matches Bin/SHA256SUMS.
use strict; use warnings;
use Test::More;
use File::Path qw(make_path);
use File::Temp qw(tempdir);
use Digest::SHA qw(sha256_hex);
use Plugins::LyrionAnywhere::Gateway;
use Proc::Background;
use Slim::Utils::Prefs;

my $base = tempdir(CLEANUP => 1);
make_path("$base/Bin/x86_64-linux");
my $bin = "$base/Bin/x86_64-linux/anywhere-gateway";
my $content = "#!/bin/sh\nexit 0\n";
sub write_file { my ($p, $c) = @_; open my $fh, '>', $p or die $!; binmode $fh; print $fh $c; close $fh }
write_file($bin, $content);
my $sums = "$base/Bin/SHA256SUMS";
my $v = \&Plugins::LyrionAnywhere::Gateway::verify_binary;

like($v->($bin, $base), qr/checksum list missing/, 'no manifest: refused');

write_file($sums, sha256_hex($content) . "  x86_64-linux/anywhere-gateway\n");
is($v->($bin, $base), undef, 'matching binary accepted');

write_file($bin, $content . "# tampered\n");
like($v->($bin, $base), qr/failed its checksum/, 'modified binary refused');
write_file($bin, substr($content, 0, 5));
like($v->($bin, $base), qr/failed its checksum/, 'truncated binary refused');
write_file($bin, $content);

write_file($sums, sha256_hex($content) . "  aarch64-linux/anywhere-gateway\n");
like($v->($bin, $base), qr/no checksum for x86_64-linux/, 'entry for another platform does not count');
write_file($sums, "not a checksum line\n" . ('0' x 64) . "  x86_64-linux/anywhere-gateway\n");
like($v->($bin, $base), qr/failed its checksum/, 'wrong checksum refused, junk lines ignored');

is($v->("$base/../elsewhere/gw", $base), 'no gateway binary', 'missing file refused');
{
	my $dev = tempdir(CLEANUP => 1);
	write_file("$dev/gw", 'anything');
	is($v->("$dev/gw", $base), undef, 'development binary outside Bin/ is not checked');
}

# Supervision refuses to launch a bundled binary that fails, and fixes a lost executable bit.
$Slim::Utils::Prefs::DIR = tempdir(CLEANUP => 1);
my $prefs = preferences('plugin.lyrionanywhere');
$prefs->set(gateway_path => '');
$prefs->set(listen => '127.0.0.1:9443');
$prefs->set(admin_port => 9445);
preferences('server')->set(httpport => 9000);
Plugins::LyrionAnywhere::Gateway->set_basedir($base);
my $plat = Plugins::LyrionAnywhere::Gateway::platform();
SKIP: {
	skip 'test binary layout is for x86_64 Linux', 4 unless ($plat // '') eq 'x86_64-linux';

	write_file($sums, ('0' x 64) . "  x86_64-linux/anywhere-gateway\n");
	@Proc::Background::STARTED = ();
	Plugins::LyrionAnywhere::Gateway->start;
	is(scalar @Proc::Background::STARTED, 0, 'tampered bundled binary is not started');
	like(Plugins::LyrionAnywhere::Gateway->state->{error}, qr/failed its checksum/, 'reason shown in status');
	Plugins::LyrionAnywhere::Gateway->stop;

	write_file($sums, sha256_hex($content) . "  x86_64-linux/anywhere-gateway\n");
	chmod 0644, $bin;
	Plugins::LyrionAnywhere::Gateway->start;
	is(scalar @Proc::Background::STARTED, 1, 'verified binary is started');
	ok(-x $bin, 'executable bit restored');
	Plugins::LyrionAnywhere::Gateway->stop;
}
done_testing;
