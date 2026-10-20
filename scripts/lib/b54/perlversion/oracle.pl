# The oracle for packages/perlversion: Perl 5.34's version objects. A line
# is "p <hex>" (version->parse), "d <hex>" (version->declare) or
# "c <hexA> <modeA> <hexB> <modeB>" (<=>). Text is hex, '-' when empty.
use strict;
use version;

sub unhex { my $h = shift; return '' if $h eq '-'; return pack('H*', $h); }
sub hx { my $s = shift; return '-' if $s eq ''; return unpack('H*', $s); }
sub make { my ($mode, $t) = @_; return $mode eq 'd' ? version->declare($t) : version->parse($t); }
sub err { my $e = shift; $e =~ s/ at \S+ line \d+(?:, <\w+> line \d+)?\.?\n?\z//; return 'E:' . hx($e); }

local $| = 0;
while (my $l = <STDIN>) {
    chomp $l;
    next if $l eq '';
    my @f = split / /, $l;
    my $out;
    if ($f[0] eq 'c') {
        my $r = eval { make($f[2], unhex($f[1])) <=> make($f[4], unhex($f[3])) };
        $out = $@ ? err($@) : "$r";
    } else {
        my $v = eval { make($f[0], unhex($f[1])) };
        if ($@) { $out = err($@); }
        else { $out = join('|', hx($v->stringify), hx($v->numify), hx($v->normal), ($v->is_alpha ? 1 : 0), ($v->is_qv ? 1 : 0)); }
    }
    print "$out\n";
}
