package Slim::Schema;
# Test stub: a library of fake tracks with persistent rows (playcount, lastplayed).
use strict; use warnings;
our %TRACKS;          # id => Slim::Schema::Track stub
our $HAS_LIBRARY = 1;
our $DIE;             # find() dies with this, when set
sub hasLibrary { $HAS_LIBRARY }
sub find {
	my ($class, $rs, $id) = @_;
	die $DIE if $DIE;
	die "stub only knows Track\n" unless $rs eq 'Track';
	return $TRACKS{$id};
}
sub add_track {
	my ($class, $id, %o) = @_;
	my $p = exists $o{persistent} ? $o{persistent}
		: bless { playcount => $o{playcount}, lastplayed => $o{lastplayed}, updates => 0 }, 'Slim::Schema::TrackPersistent';
	$TRACKS{$id} = bless { id => $id, url => $o{url}, remote => $o{remote} // 0, persistent => $p }, 'Slim::Schema::Track';
}

package Slim::Schema::Track;
sub url    { $_[0]{url} }
sub remote { $_[0]{remote} }
sub retrievePersistent { $_[0]{persistent} }

package Slim::Schema::TrackPersistent;
our $FAIL;            # update() dies when set
sub playcount  { $_[0]{playcount} }
sub lastplayed { $_[0]{lastplayed} }
sub set { my ($s, %kv) = @_; $s->{pending}{$_} = $kv{$_} for keys %kv }
sub update {
	my $s = shift;
	die "database is locked\n" if $FAIL;
	$s->{$_} = delete $s->{pending}{$_} for keys %{ $s->{pending} || {} };
	$s->{updates}++;
}
1;
