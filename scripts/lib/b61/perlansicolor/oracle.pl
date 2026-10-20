# Term::ANSIColor (5.01, as in Perl 5.34), one call per line; arguments
# are hex of UTF-8 ("-" for empty), lists are comma-separated:
#   color <codes>            color(@codes)
#   colored <eachline> <string> <codes>       colored($string, @codes)
#   coloredl <eachline> <codes> <strings>     colored(\@codes, @strings)
#   uncolor <escapes>        uncolor(@escapes)
#   strip <strings>          colorstrip(@strings), in list context
#   valid <codes>            colorvalid(@codes)
# <eachline> is "~" for undef. Answers: the result's hex (a list as hex
# joined with ",", "~" for an empty list, "U" for undef, "1"/"0" for
# colorvalid), or E:<hex of the croak message without " at FILE line N.">.
use strict;
use warnings;
no warnings 'uninitialized';
use Term::ANSIColor qw(color colored uncolor colorstrip colorvalid);

delete $ENV{ANSI_COLORS_DISABLED};
delete $ENV{NO_COLOR};
delete $ENV{ANSI_COLORS_ALIASES};

sub unhex { my $h = shift; return '' if $h eq '-'; my $s = pack('H*', $h); utf8::decode($s); return $s; }
sub hexs { my $s = shift; return 'U' unless defined $s; utf8::upgrade($s); utf8::encode($s); return length $s ? unpack('H*', $s) : '-'; }
sub list { my $l = shift; return () if $l eq '~'; return map { unhex($_) } split /,/, $l; }
sub hexlist { return @_ ? join(',', map { hexs($_) } @_) : '~'; }

while (my $line = <STDIN>) {
    chomp $line;
    my @f = split / /, $line;
    my $op = shift @f;
    my $out = eval {
        if ($op eq 'color') { return hexs(color(list($f[0]))); }
        if ($op eq 'colored' or $op eq 'coloredl') {
            local $Term::ANSIColor::EACHLINE = $f[0] eq '~' ? undef : unhex($f[0]);
            return hexs($op eq 'colored' ? colored(unhex($f[1]), list($f[2])) : colored([list($f[1])], list($f[2])));
        }
        if ($op eq 'uncolor') { return hexlist(uncolor(list($f[0]))); }
        if ($op eq 'strip') { my @r = colorstrip(list($f[0])); return hexlist(@r); }
        return colorvalid(list($f[0])) ? '1' : '0';
    };
    if (!defined $out) {
        my $m = $@;
        $m =~ s/ at \S+ line \d+\.\n\z//;
        $out = 'E:' . hexs($m);
    }
    print "$out\n";
}
