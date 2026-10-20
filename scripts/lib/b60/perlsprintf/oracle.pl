# Perl 5.34's sprintf, one call per line: <format-hex> <arg>... where an
# argument is s<hex> (a string), i<decimal> (an integer) or n<16 hex digits>
# (the bits of a double). Answers <T|U|D>:<hex> <warnings>: T and the text,
# U when the text holds a surrogate, a code point past U+10FFFF or broken
# UTF-8, D and the message when sprintf died; then the warnings, each as
# hex, joined by commas, or "-".
use strict;
use warnings;
binmode STDOUT;
$| = 1;
my @w;
$SIG{__WARN__} = sub { my $m = $_[0]; $m =~ s/ at \S+ line \d+(?:, <\w+> line \d+)?\.\n\z//; push @w, $m };
sub unhex { my $h = shift; return "" if $h eq "-"; my $b = pack("H*", $h); utf8::decode($b); return $b }
sub hx { my $s = shift; utf8::encode($s); my $h = unpack("H*", $s); return $h eq "" ? "-" : $h }
while (my $line = <STDIN>) {
    chomp $line;
    my ($f, @a) = split / /, $line;
    my $fmt = unhex($f);
    my @args;
    for my $x (@a) {
        my ($k, $v) = (substr($x, 0, 1), substr($x, 1));
        if ($k eq "s") { push @args, unhex($v) }
        elsif ($k eq "i") { push @args, 0 + $v }
        else { push @args, unpack("d>", pack("H*", $v)) }
    }
    @w = ();
    my $r = eval { sprintf($fmt, @args) };
    my @ws = @w;
    my $out;
    if (!defined $r) {
        my $e = $@;
        $e =~ s/ at \S+ line \d+(?:, <\w+> line \d+)?\.\n\z//;
        $out = "D:" . hx($e);
    } elsif (!utf8::valid($r) || $r =~ /[\x{D800}-\x{DFFF}]/ || $r =~ /[^\x{0}-\x{10FFFF}]/) {
        $out = "U:-";
    } else {
        $out = "T:" . hx($r);
    }
    print $out, " ", (@ws ? join(",", map { hx($_) } @ws) : "-"), "\n";
}
