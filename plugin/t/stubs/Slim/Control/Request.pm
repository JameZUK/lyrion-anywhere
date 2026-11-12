package Slim::Control::Request;
# Test stub: records dispatch registrations; minimal request object for handlers.
use strict; use warnings;
our %DISPATCH;
sub addDispatch { my ($names, $spec) = @_; $DISPATCH{ join(' ', @$names) } = $spec }
sub new { my ($class, $words, $params) = @_; bless { words => $words, params => $params || {}, results => {}, status => 'new' }, $class }
sub getRequest { $_[0]{words}[ $_[1] ] }
sub getParam   { $_[0]{params}{ $_[1] } }
sub addResult  { $_[0]{results}{ $_[1] } = $_[2] }
sub setStatusDone        { $_[0]{status} = 'done' }
sub setStatusProcessing  { $_[0]{status} = 'processing' }
sub setStatusBadParams   { $_[0]{status} = 'badparams' }
# Where the request came from, as LMS's front ends set it: 'JSONRPC' (with the HTTP
# connection, a key into %Slim::Web::JSONRPC::contexts), 'CLI', a Cometd string, or none.
sub source       { $_[0]{source} }
sub connectionID { $_[0]{connection} }
1;
