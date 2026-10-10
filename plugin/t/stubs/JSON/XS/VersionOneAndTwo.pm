package JSON::XS::VersionOneAndTwo;
# Test stub: LMS bundles JSON::XS; core JSON::PP behaves the same for these calls.
use strict; use warnings;
use JSON::PP ();
use Exporter 'import';
our @EXPORT = qw(encode_json decode_json);
sub encode_json { JSON::PP->new->canonical->encode($_[0]) }
sub decode_json { JSON::PP::decode_json($_[0]) }
1;
