# Text::Balanced 2.04, one call per line, in list context:
#   d <text> <delims> <prefix> <escapes>    extract_delimited
#   b <text> <delims> <prefix>              extract_bracketed
# Strings are hex ("-" for empty, "~" for an argument not given). Answers
#   <extracted> <remainder> <prefix> <error>
# with "~" for undef and the error as <hex message>:<pos>.
use strict;
use warnings;
use Text::Balanced qw(extract_delimited extract_bracketed);
binmode STDOUT;
$| = 1;
sub unhex { my $h = shift; return undef if $h eq "~"; return "" if $h eq "-"; my $b = pack("H*", $h); utf8::decode($b); return $b }
sub hx { my $s = shift; return "~" unless defined $s; utf8::encode($s); my $h = unpack("H*", $s); return $h eq "" ? "-" : $h }
while (my $line = <STDIN>) {
    chomp $line;
    my ($op, @f) = split / /, $line;
    my @a = map { unhex($_) } @f;
    my @r;
    if ($op eq "d") { @r = extract_delimited($a[0], $a[1], $a[2], $a[3]) }
    else { @r = extract_bracketed($a[0], $a[1], $a[2]) }
    my $e = $@;
    my $err = ref $e ? hx($e->{error}) . ":" . $e->{pos} : "~";
    print join(" ", hx($r[0]), hx($r[1]), hx($r[2]), $err), "\n";
}
