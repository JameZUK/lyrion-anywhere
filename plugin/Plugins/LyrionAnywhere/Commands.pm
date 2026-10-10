package Plugins::LyrionAnywhere::Commands;

# LMS JSON-RPC/CLI commands used by the settings page: `anywhere status`, `anywhere restart`
# and one command per admin operation (`anywhere users`, `anywhere user_create
# username:… password:… role:…`, …).
#
# They run inside LMS, so they are as reachable as LMS's own settings: anyone who can
# change LMS settings on the LAN can manage remote accounts. The gateway never forwards
# `anywhere` commands from the internet (it is reserved in its allow-list).

use strict;
use warnings;

use Slim::Control::Request;
use Slim::Utils::Log;

use Plugins::LyrionAnywhere::Admin;
use Plugins::LyrionAnywhere::Gateway;

my $log = logger('plugin.lyrionanywhere');

my @ARG_NAMES = qw(username password role disabled player id limit);

sub register {
	# [needsClient, isQuery, hasTags, function]
	Slim::Control::Request::addDispatch(['anywhere', 'status'],  [0, 1, 0, \&status]);
	Slim::Control::Request::addDispatch(['anywhere', 'restart'], [0, 0, 0, \&restart]);
	for my $op (Plugins::LyrionAnywhere::Admin::operations()) {
		Slim::Control::Request::addDispatch(['anywhere', $op], [0, 1, 1, \&admin]);
	}
}

sub status {
	my $request = shift;
	my $state = Plugins::LyrionAnywhere::Gateway->state;
	$request->addResult($_, $state->{$_}) for sort keys %$state;
	$request->setStatusDone;
}

sub restart {
	my $request = shift;
	Plugins::LyrionAnywhere::Gateway->restart;
	$request->setStatusDone;
}

sub admin {
	my $request = shift;
	my $op = $request->getRequest(1);
	my %args;
	for my $k (@ARG_NAMES) {
		my $v = $request->getParam($k);
		$args{$k} = $v if defined $v;
	}
	$args{limit} //= 100 if $op eq 'audit';

	# Validate before anything is sent; a bad request never reaches the gateway.
	eval { Plugins::LyrionAnywhere::Admin::request_for($op, \%args) };
	if (my $e = $@) {
		chomp $e;
		$request->addResult('error', $e);
		$request->setStatusDone;
		return;
	}

	$request->setStatusProcessing;
	Plugins::LyrionAnywhere::Admin::call(
		$op, \%args,
		sub {
			my $data = shift;
			$request->addResult('ok', 1);
			$request->addResult('data', $data);
			$request->setStatusDone;
		},
		sub {
			my $e = shift;
			$request->addResult('error', $e);
			$request->setStatusDone;
		},
	);
}

1;
