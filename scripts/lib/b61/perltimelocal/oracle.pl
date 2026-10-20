# Time::Local 1.30 (Perl 5.34), one call per line:
#   <timegm|timegm_nocheck|timegm_modern|timegm_posix> <this year> <sec> <min> <hour> <mday> <mon> <year>
# Numbers are written as Perl reads them. "This year" fixes the
# two-digit-year window: the module is loaded afresh for every call with
# localtime() reporting that year. Answers the result as Perl prints it,
# or E:<hex of the croak message>.
use strict;
use warnings;
binmode STDOUT;
$| = 1;
our $FAKE_YEAR = 2026;
BEGIN { *CORE::GLOBAL::localtime = sub { my @t = CORE::localtime(@_ ? $_[0] : 0); $t[5] = $FAKE_YEAR - 1900; return wantarray ? @t : CORE::localtime(@_ ? $_[0] : 0) }; }
require Time::Local;
my $path = $INC{"Time/Local.pm"};
sub hx { my $s = shift; utf8::encode($s); my $h = unpack("H*", $s); return $h eq "" ? "-" : $h }
while (my $line = <STDIN>) {
    chomp $line;
    my ($fn, $year_now, @a) = split / /, $line;
    $FAKE_YEAR = $year_now;
    { no warnings; do $path or die "reload: $@"; }
    my @args = map { 0 + $_ } @a;
    my $r = eval { no strict 'refs'; &{"Time::Local::$fn"}(@args) };
    if (defined $r) { print "$r\n" }
    else { my $e = $@; $e =~ s/ at \S+ line \d+(?:, <\w+> line \d+)?\.\n\z//; print "E:", hx($e), "\n" }
}
