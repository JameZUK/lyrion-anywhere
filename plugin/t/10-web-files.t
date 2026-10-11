# The settings page's files: everything it references exists where LMS serves it, and
# nothing in them can be mis-decoded.
#
# LMS serves a plugin's HTML/EN tree, but images only from paths with an `html/` segment
# (Slim::Web::HTTP treats other image paths as templates, which 404); so the logo lives in
# HTML/EN/plugins/LyrionAnywhere/html/images/. Templates are read as bytes and the page is
# sent as UTF-8, so a raw "\xe2\x80\xba" in a template shows as mojibake; the templates, script
# and style sheet are therefore ASCII only (HTML entities in templates, \u escapes in JS).
use strict; use warnings;
use Test::More;
use File::Find;

my $root = 'Plugins/LyrionAnywhere';
my $html = "$root/HTML/EN";

sub slurp {
	my $f = shift;
	open my $fh, '<:raw', $f or die "$f: $!";
	local $/;
	return scalar <$fh>;
}

# --- every text file served to the browser, and strings.txt, is ASCII --------------------
my @text;
find(sub { push @text, $File::Find::name if -f && /\.(?:html|js|css|txt)\z/ }, "$root/HTML");
push @text, "$root/strings.txt", "$root/install.xml";
ok(@text >= 5, 'found the page files') or diag "@text";
for my $f (sort @text) {
	my $bytes = slurp($f);
	my @bad;
	my $line = 0;
	for my $l (split /\n/, $bytes) {
		$line++;
		push @bad, "line $line" if $l =~ /[^\x00-\x7f]/;
	}
	ok(!@bad, "$f is ASCII only") or diag "non-ASCII at @bad";
	unlike($bytes, qr/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/, "$f has no control characters");
}

# The check itself catches the real failure ("Settings > Advanced" with U+203A).
like("Settings \xe2\x80\xba Advanced", qr/[^\x00-\x7f]/, 'the check spots raw UTF-8');

# --- the template's entities stand for the characters intended ---------------------------
my $page = slurp("$html/plugins/LyrionAnywhere/settings/basic.html");
like($page, qr/Settings &rsaquo; Advanced &rsaquo; Security/, 'menu path uses &rsaquo;');
like($page, qr/Loading&hellip;/, 'ellipsis is an entity');
my %known = map { $_ => 1 } qw(rsaquo lsaquo hellip ndash mdash middot rsquo lsquo ldquo rdquo nbsp amp lt gt quot times);
for my $e ($page =~ /&([a-z]+);/g) {
	ok($known{$e}, "entity &$e; is a standard one");
}

# --- every file the page references exists, and images are under html/ -----------------
my @refs = map { s/\?.*\z//sr } $page =~ /(?:src|href)="(\/plugins\/LyrionAnywhere\/[^"]+)"/g;   # minus any ?v= stamp
ok(scalar(grep { /\.png\z/ } @refs), 'the page shows the logo');
for my $ref (@refs) {
	my $file = $html . $ref;
	ok(-f $file, "$ref exists in the plugin") or diag "missing $file";
	if ($ref =~ /\.(?:png|gif|jpe?g|svg|ico)\z/) {
		like($ref, qr{/html/}, "$ref is under an html/ path, where LMS serves images");
	}
}
{
	my $logo = slurp("$html/plugins/LyrionAnywhere/html/images/logo.png");
	is(substr($logo, 0, 8), "\x89PNG\r\n\x1a\n", 'the logo is a PNG');
	ok(!-e "$html/plugins/LyrionAnywhere/settings/logo.png", 'no stale copy where LMS cannot serve it');
}

# --- the script's helpers use the same defaults and limits as the gateway ---------------
my $js = slurp("$html/plugins/LyrionAnywhere/settings/anywhere.js");
like($js, qr/const DEFAULT_SESSION_DAYS = 90;/, 'script default lifetime is 90 days');
like($page, qr/id="aw-session-days"[^>]*value="90"/, 'page shows 90 until the gateway answers');

done_testing;
