package Slim::Networking::SimpleAsyncHTTP;
# Test stub: records each request; tests complete it with respond()/fail().
use strict; use warnings;
our @REQUESTS;
sub new { my ($class, $ok, $err, $opts) = @_; bless { ok => $ok, err => $err, opts => $opts }, $class }
sub get  { my ($s, $url, @h) = @_; push @REQUESTS, { %$s, method => 'GET', url => $url, headers => {@h} }; $s }
sub post { my ($s, $url, @rest) = @_; my $body = @rest % 2 ? pop @rest : undef;
           push @REQUESTS, { %$s, method => 'POST', url => $url, headers => {@rest}, body => $body }; $s }
sub respond { my ($req, $content) = @_; $req->{ok}->(bless { content => $content }, 'Slim::Networking::SimpleAsyncHTTP::Response') }
sub fail    { my ($req, $error) = @_; $req->{err}->(undef, $error) }
package Slim::Networking::SimpleAsyncHTTP::Response;
sub content { $_[0]{content} }
1;
