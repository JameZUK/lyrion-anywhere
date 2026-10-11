# `anywhere play_record`: the signed command the gateway sends to count an offline play.
use strict; use warnings;
use Test::More;
use File::Temp qw(tempdir);
use Slim::Control::Request;
use Slim::Schema;
use Slim::Music::Import;
use Slim::Utils::Prefs;
use Plugins::LyrionAnywhere::Commands;
use Plugins::LyrionAnywhere::Plays;

my $P = 'Plugins::LyrionAnywhere::Plays';
my $NOW = 1_790_000_100;
{ no warnings 'redefine'; *Plugins::LyrionAnywhere::Plays::_now = sub { $NOW } }
my $sign = \&Plugins::LyrionAnywhere::Plays::signature;
my $seen = \&Plugins::LyrionAnywhere::Plays::seen;

# The admin secret, where the gateway writes it.
my $secret = '0123456789abcdef' x 4;
my $dir = tempdir(CLEANUP => 1);
$Slim::Utils::Prefs::DIR = $dir;
mkdir "$dir/lyrion-anywhere";
sub write_secret { open my $fh, '>', "$dir/lyrion-anywhere/admin.secret" or die; print $fh shift; close $fh }
write_secret($secret);

Plugins::LyrionAnywhere::Commands::register();
my $spec = $Slim::Control::Request::DISPATCH{'anywhere play_record'};
ok($spec, 'play_record registered');
is_deeply([@$spec[0..2]], [0, 0, 1], 'no player; a command, not a query; key:value arguments');

# The shared test vector: the gateway's tests check the same value (gateway/src/plays.rs).
is($sign->($secret, 42, 1_790_000_000, 'key-0001', 1_790_000_100),
	'de7d809202cfbfb878c1db65e10cbdb75f1885b20e7e4a25e05b5d73d2393806', 'signature matches the gateway');

Slim::Schema->add_track(1, url => 'file:///srv/music/a.flac', playcount => 3, lastplayed => 1_700_000_000);
Slim::Schema->add_track(2, url => 'http://radio.example.com/live', remote => 1);
Slim::Schema->add_track(3, url => 'spotify:track:abc');
Slim::Schema->add_track(4, url => 'file:///srv/music/new.flac');
Slim::Schema->add_track(5, url => 'file:///srv/music/nopersist.flac', persistent => undef);

my $n = 0;
sub params {
	my (%o) = @_;
	my %p = (track_id => 1, played_at => $NOW - 3600, key => sprintf('key-%04d', ++$n), ts => $NOW, %o);
	$p{sig} = $sign->($secret, @p{qw(track_id played_at key ts)}) unless exists $o{sig};
	delete $p{$_} for grep { !defined $p{$_} } keys %p;
	return \%p;
}
sub run_cmd {
	my $p = shift;
	my $r = Slim::Control::Request->new(['anywhere', 'play_record'], $p);
	$spec->[3]->($r);
	is($r->{status}, 'done', 'answered at once');
	return $r->{results};
}
my $t1 = $Slim::Schema::TRACKS{1}{persistent};

# Recorded: playcount + 1, lastplayed moves to when the play started.
my $p = params();
is_deeply(run_cmd($p), { ok => 1 }, 'a signed play is recorded');
is($t1->{playcount}, 4, 'play count raised');
is($t1->{lastplayed}, $NOW - 3600, 'last played set to when the play started');
ok($seen->($p->{key}), 'key remembered');

# The same key again: duplicate, nothing counted.
is_deeply(run_cmd($p), { error => 'duplicate' }, 'a resend is a duplicate');
is($t1->{playcount}, 4, 'not counted twice');

# An older play counts, but doesn't move last-played back.
is_deeply(run_cmd(params(played_at => $NOW - 86400 * 30)), { ok => 1 });
is($t1->{playcount}, 5);
is($t1->{lastplayed}, $NOW - 3600, 'last played never goes back in time');

# A track never played before.
is_deeply(run_cmd(params(track_id => 4)), { ok => 1 });
is($Slim::Schema::TRACKS{4}{persistent}{playcount}, 1, 'first play counted from nothing');

# --- refusals: nothing is counted --------------------------------------------------------
my $before = $t1->{playcount};
is(run_cmd(params(track_id => 2))->{error}, 'not_library', 'radio stream refused');
is(run_cmd(params(track_id => 3))->{error}, 'not_library', 'non-file URL refused');
is(run_cmd(params(track_id => 5))->{error}, 'not_library', 'no persistent row: refused');
is(run_cmd(params(track_id => 999))->{error}, 'not_found', 'unknown track refused');

# Signature and freshness.
for my $case (
	['no sig',              params(sig => undef)],
	['empty sig',           params(sig => '')],
	['wrong sig',           params(sig => 'f' x 64)],
	['upper-case sig',      do { my $x = params(); $x->{sig} = uc $x->{sig}; $x }],
	['sig for other track', do { my $x = params(); $x->{track_id} = 4; $x }],
	['sig for other time',  do { my $x = params(); $x->{played_at}--; $x }],
	['sig for other key',   do { my $x = params(); $x->{key} .= 'x'; $x }],
	['sig as a list',       do { my $x = params(); $x->{sig} = [$x->{sig}]; $x }],
	['stale',               params(ts => $NOW - 121)],
	['from the future',     params(ts => $NOW + 121)],
	['play in the future',  params(played_at => $NOW + 301)],
	['play too old',        params(played_at => $NOW - 92 * 86400)],
) {
	my ($name, $q) = @$case;
	like(run_cmd($q)->{error} // '', qr/^refused/, "$name: refused");
}

# Hostile values never reach the library.
my @found;
{
	no warnings 'redefine';
	local *Slim::Schema::find = sub { push @found, $_[2]; undef };
	for my $case (
		[track_id => '0'], [track_id => '-1'], [track_id => '1.5'], [track_id => '01'],
		[track_id => "1\n"], [track_id => '1 OR 1=1'], [track_id => '1' x 13], [track_id => ''],
		[played_at => 'now'], [played_at => "$NOW\n"], [played_at => '-5'],
		[key => 'short'], [key => 'x' x 65], [key => "key-0001\n"], [key => 'key 0001'], [key => '../../etc'],
		[ts => '0'], [ts => '1e10'], [ts => [1]],
	) {
		my ($k, $v) = @$case;
		my $q = params();
		$q->{$k} = $v;
		like(run_cmd($q)->{error} // '', qr/^refused: bad/, "bad $k: refused");
	}
}
is(scalar @found, 0, 'the library was never looked at');

# Without the admin secret (gateway never started), nothing can be checked.
unlink "$dir/lyrion-anywhere/admin.secret";
is(run_cmd(params())->{error}, 'unavailable', 'no secret: unavailable');
write_secret('not hex at all');
is(run_cmd(params())->{error}, 'unavailable', 'a malformed secret is not used');
write_secret($secret);

# LMS states: scanning, no library, statistics off, database errors.
{
	local $Slim::Music::Import::SCANNING = 1;
	is(run_cmd(params())->{error}, 'busy', 'busy while the scanner has the database');
}
{
	local $Slim::Schema::HAS_LIBRARY = 0;
	is(run_cmd(params())->{error}, 'busy', 'busy without a library');
}
{
	no warnings 'once';
	local $Slim::Schema::TrackPersistent::FAIL = 1;
	my $q = params();
	is(run_cmd($q)->{error}, 'failed', 'a database error is reported');
	ok(!$seen->($q->{key}), 'and the key is not remembered, so a retry can count');
}
{
	local $Slim::Schema::DIE = "boom\n";
	is(run_cmd(params())->{error}, 'failed', 'a failing lookup is reported');
}
{
	no strict 'refs'; no warnings 'redefine', 'once';
	local *main::STATISTICS = sub { 0 };
	is(run_cmd(params())->{error}, 'unsupported', 'statistics off: unsupported');
}
is($t1->{playcount}, $before, 'no refusal counted anything');

# The key memory is bounded.
Plugins::LyrionAnywhere::Plays::forget_all();
my $max = Plugins::LyrionAnywhere::Plays::REMEMBER();
Plugins::LyrionAnywhere::Plays::_remember("bulk-key-$_") for 1 .. $max + 5;
ok(!$seen->('bulk-key-1'), 'oldest keys forgotten');
ok($seen->('bulk-key-' . ($max + 5)), 'newest kept');

# Robustness: random parameters never die and never count anything.
{
	my $count = $t1->{playcount};
	srand(42);
	my @chars = map { chr } 0 .. 127;
	my $ok = 1;
	for (1 .. 2000) {
		my %q;
		for my $k (qw(track_id played_at key ts sig)) {
			next if rand() < 0.1;
			$q{$k} = join '', map { $chars[rand @chars] } 1 .. int rand 70;
		}
		my $r = Slim::Control::Request->new(['anywhere', 'play_record'], \%q);
		$ok = 0 unless eval { $spec->[3]->($r); 1 };
		$ok = 0 unless $r->{status} eq 'done' && defined $r->{results}{error};
	}
	ok($ok, 'every random request answered with an error, none died');
	is($t1->{playcount}, $count, 'random requests counted nothing');
}

done_testing;
