# Contract test: every admin request the plugin builds is accepted by the real gateway.
# Runs the gateway binary (target/debug or target/release); skipped if neither is built.
use strict; use warnings;
use Test::More;
use File::Temp qw(tempdir);
use HTTP::Tiny;
use IO::Socket::INET;
use JSON::PP qw(encode_json decode_json);
use Plugins::LyrionAnywhere::Admin;
use Plugins::LyrionAnywhere::Gateway;

my ($bin) = grep { -x } map { "../target/$_/anywhere-gateway" } qw(debug release);
unless ($bin) {
	# CI sets ANYWHERE_REQUIRE_GATEWAY so this contract test can never be skipped silently there.
	BAIL_OUT('gateway binary not built (cargo build -p anywhere-gateway)') if $ENV{ANYWHERE_REQUIRE_GATEWAY};
	plan skip_all => 'gateway binary not built (cargo build -p anywhere-gateway)';
}

sub free_port { my $s = IO::Socket::INET->new(Listen => 1, LocalAddr => '127.0.0.1', LocalPort => 0) or die; my $p = $s->sockport; close $s; $p }
my ($public, $admin, $tunnel) = (free_port(), free_port(), free_port());
my $dir = tempdir(CLEANUP => 1);
# Trusted networks as the plugin normalises them, written awkwardly: the gateway must accept
# each and name the same networks (Rust's Display), so the two checks agree.
my @trusted_in = ('10.8.0.5/24', 'FD12:3456:789A::1/48', 'fd00:0:1:0:0:1:0:0', 'fd01::10.0.0.1/128', '100.101.102.103', 'fe80:0:0:0::/64');
my @trusted = map { (Plugins::LyrionAnywhere::Gateway::valid_trusted_network($_))[0] } @trusted_in;
my $pid = fork // die;
if ($pid == 0) {
	open STDOUT, '>', "$dir/gw.log"; open STDERR, '>&', \*STDOUT;
	exec $bin, 'serve', '--data-dir', "$dir/data", '--listen', "127.0.0.1:$public",
		'--admin-listen', "127.0.0.1:$admin", '--lms', '127.0.0.1', '--lms-http-port', 1, '--lms-cli-port', 1,
		'--no-transcode', '--download-cache-mb', 64, '--client-ip-header', 'CF-Connecting-IP',
		'--no-analysis', '--tunnel-listen', "127.0.0.1:$tunnel",
		map { ('--trusted-network', $_) } @trusted;
	exit 127;
}
END { kill 'TERM', $pid if $pid }

my $secret;
for (1 .. 100) { last if -s "$dir/data/admin.secret"; select undef, undef, undef, 0.1 }
open my $fh, '<', "$dir/data/admin.secret" or BAIL_OUT('gateway did not start: ' . `cat $dir/gw.log`);
chomp($secret = <$fh>);
my $http = HTTP::Tiny->new(timeout => 10);

sub send_op {
	my ($op, $args) = @_;
	my ($method, $path, $body) = Plugins::LyrionAnywhere::Admin::request_for($op, $args);
	my $res = $http->request($method, "http://127.0.0.1:$admin$path", {
		headers => { Authorization => "Bearer $secret", 'Content-Type' => 'application/json' },
		($body ? (content => JSON::PP->new->encode($body)) : ()),
	});
	return ($res->{status}, length($res->{content} // '') ? eval { decode_json($res->{content}) } : undef);
}

my @steps = (
	[user_create   => { username => 'alice', password => 'correct horse battery', role => 'listener' }, 201],
	[users         => {}, 200],
	[user_password => { username => 'alice', password => 'another long password' }, 204],
	[pairing       => { username => 'alice' }, 200],
	[grant         => { username => 'alice', player => 'AA:BB:CC:DD:EE:01' }, 204],
	[grants        => { username => 'alice' }, 200],
	[ungrant       => { username => 'alice', player => 'aa:bb:cc:dd:ee:01' }, 204],
	[user_disable  => { username => 'alice', disabled => 1 }, 204],
	[user_disable  => { username => 'alice', disabled => 0 }, 204],
	[totp_reset    => { username => 'alice' }, 204],
	[user_downloads => { username => 'alice', enabled => 1, formats => 'original,opus' }, 204],
	[users         => {}, 200, 'downloads'],
	[user_downloads => { username => 'alice', enabled => 0 }, 204],
	[users         => {}, 200, 'downloads off'],
	[devices       => {}, 200],
	# No such device: the route and body are right (a wrong body would be a 400).
	[device_rename => { id => 999999, name => 'Kitchen' }, 404],
	[audit         => { limit => 20 }, 200],
	[settings      => {}, 200],
	[session_days  => { days => 90 }, 204],
	[session_days  => { days => '7' }, 204],
	[plugins       => {}, 200],
	[plugins_admin_only => { commands => 'audiomuseai,custombrowse' }, 204],
	[plugins_admin_only => {}, 204],
	[user_delete   => { username => 'alice' }, 204],
);
for my $s (@steps) {
	my ($op, $args, $want, $check) = @$s;
	my ($status, $data) = send_op($op, $args);
	is($status, $want, "$op accepted by the gateway");
	if ($op eq 'pairing') { like($data->{code}, qr/^[A-Z2-9]{10}$/, 'pairing code returned') }
	if ($op eq 'grants')  { is_deeply($data->{players}, ['aa:bb:cc:dd:ee:01'], 'grant visible') }
	if ($op eq 'users')   { is($data->{users}[0]{username}, 'alice') }
	if ($op eq 'users' && !$check) {
		is($data->{users}[0]{downloads}, JSON::PP::true, 'downloads on by default');
		is_deeply($data->{users}[0]{download_formats}, [qw(original mp3 opus)], 'every format by default');
	}
	if (($check // '') eq 'downloads') {
		is_deeply($data->{users}[0]{download_formats}, [qw(original opus)], 'download formats saved');
	}
	if (($check // '') eq 'downloads off') {
		is($data->{users}[0]{downloads}, JSON::PP::false, 'downloads turned off');
		is_deeply($data->{users}[0]{download_formats}, [qw(original opus)], 'formats kept when turned off');
	}
	if ($op eq 'settings') {
		is($data->{session_days}, 90, 'default sign-in lifetime');
		is($data->{session_days_default}, 90, 'the gateway reports the default');
		is_deeply($data->{download_formats}, ['original'], 'started with --no-transcode: originals only');
		is_deeply($data->{transcoding}, { state => 'off' }, 'and the settings page is told why');
	}
}
is((send_op('settings', {}))[1]{session_days}, 7, 'sign-in lifetime saved');
my ($status) = send_op('users', {});
my ($users_after) = (send_op('users', {}))[1];
is_deeply($users_after->{users}, [], 'user deleted');

# Bad download settings are refused by the gateway too (not only by the plugin).
{
	my $res = $http->request('POST', "http://127.0.0.1:$admin/admin/v1/users/nobody/downloads", {
		headers => { Authorization => "Bearer $secret", 'Content-Type' => 'application/json' },
		content => '{"enabled":true,"formats":["flac"]}',
	});
	is($res->{status}, 400, 'unknown download format refused by the gateway');
}

# A refusal explains itself as {"error": "..."}, which the plugin shows on the settings page.
{
	my ($status, $data) = send_op(user_create => { username => 'bob', password => 'x' x 10, role => 'listener' });
	is($status, 201, 'user created');
	($status, $data) = send_op(user_create => { username => 'bob', password => 'x' x 10, role => 'listener' });
	cmp_ok($status, '>=', 400, 'a second user of that name is refused');
	my $res = bless { content => JSON::PP->new->encode($data // {}) }, 'ContractRes';
	sub ContractRes::content { $_[0]{content} }
	my $m = Plugins::LyrionAnywhere::Admin::gateway_message($res);
	ok(defined $m && length $m, 'with a message the plugin can show: ' . ($m // 'none'));
	send_op(user_delete => { username => 'bob' });
}

# The tunnel's status names the tunnel's own listener, which the settings page tells the
# admin to set as the service in Cloudflare.
{
	my ($status, $data) = send_op(tunnel_status => {});
	is($status, 200, 'tunnel_status accepted by the gateway');
	is($data->{tunnel_listen}, "127.0.0.1:$tunnel", 'the status names the tunnel listener');
}

# --- the flags the plugin passes are the gateway's own -------------------------------------
{
	# Every flag the plugin can build, with every option set (and with the switches off).
	my ($argv, $err) = Plugins::LyrionAnywhere::Gateway::command_line(
		binary => $bin, listen => '10.8.0.1:9443', admin_listen => '127.0.0.1:9445', data_dir => "$dir/data",
		http_port => 9000, cli_port => 9090, log_file => "$dir/gw.log", static_dir => "$dir/web",
		trusted_proxies => ['10.0.0.10'], trusted_networks => ['10.8.0.0/24'], allowed_origins => ['https://music.example.com'],
		client_ip_header => 'CF-Connecting-IP', ffmpeg => '/usr/bin/ffmpeg', download_cache_mb => 64,
	);
	ok($argv, 'every option: a command line') or diag $err;
	my ($no) = Plugins::LyrionAnywhere::Gateway::command_line(
		binary => $bin, listen => '127.0.0.1:9443', admin_listen => '127.0.0.1:9445', data_dir => "$dir/data",
		http_port => 9000, cli_port => 9090, transcode => 0, analysis => 0,
	);
	my $help = `"$bin" serve --help 2>&1`;
	my %known = map { $_ => 1 } $help =~ /(--[a-z][a-z0-9-]*)/g;
	my %flags = map { $_ => 1 } grep { /^--/ } @$argv, @$no;
	ok($flags{'--trusted-network'}, 'the plugin can pass --trusted-network');
	ok($known{$_}, "the gateway knows $_") for sort keys %flags;
	like($help, qr/--trusted-network <[A-Z_]+>/, 'and --trusted-network takes a value');
}

# The gateway took the trusted networks and names them as the plugin normalised them.
{
	(my $log = do { open my $fh, '<', "$dir/gw.log" or die; local $/; <$fh> }) =~ s/\e\[[0-9;]*m//g;
	my ($named) = $log =~ /trusted networks networks=(.*)$/m;
	ok(defined $named, 'the gateway warns that it accepts plain HTTP from the trusted networks') or diag $log;
	is($named, join(', ', @trusted), 'the same networks, written the same way, as the plugin\'s normalisation');
}

# A network the plugin refuses, the gateway refuses too, with the same reason.
for my $bad ('8.8.8.0/24', '0.0.0.0/0', '::ffff:10.0.0.0/104', '127.0.0.1', '10.8.0.0/33', '10.8.0/24') {
	my (undef, $why) = Plugins::LyrionAnywhere::Gateway::valid_trusted_network($bad);
	my $out = `"$bin" serve --data-dir "$dir/refused" --listen 127.0.0.1:1 --admin-listen 127.0.0.1:1 --lms 127.0.0.1 --trusted-network '$bad' 2>&1`;
	isnt($? >> 8, 0, "the gateway will not start with --trusted-network $bad");
	ok(index($out, $why) >= 0, "and says the same as the plugin: $why") or diag $out;
}

# Without the secret the gateway refuses, whatever the plugin would have sent.
my $res = $http->get("http://127.0.0.1:$admin/admin/v1/users");
is($res->{status}, 403, 'admin API refuses requests without the secret');

done_testing;
