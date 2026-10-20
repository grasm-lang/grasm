# Text::Abbrev's abbrev, one call per line: the words as hex ("-" for an
# empty one), space-separated, or "~" for no words. Answers key=value pairs
# as hex, sorted by the key's hex, joined by commas, or "-".
use strict;
use warnings;
use Text::Abbrev;
binmode STDOUT;
$| = 1;
sub unhex { my $h = shift; return "" if $h eq "-"; my $b = pack("H*", $h); utf8::decode($b); return $b }
sub hx { my $s = shift; utf8::encode($s); my $h = unpack("H*", $s); return $h eq "" ? "-" : $h }
while (my $line = <STDIN>) {
    chomp $line;
    my @words = $line eq "~" ? () : map { unhex($_) } split / /, $line;
    my %h = abbrev(@words);
    my @pairs = sort map { hx($_) . "=" . hx($h{$_}) } keys %h;
    print @pairs ? join(",", @pairs) : "-", "\n";
}
