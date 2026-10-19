# Oracle for scripts/b40_diff.py: random Perl data stored with Storable
# (canonical), each file's thawed value printed as b40test prints its own.
use strict;
use warnings;
use Storable qw(nfreeze freeze nstore store thaw retrieve);
use Scalar::Util qw(refaddr blessed reftype weaken);

$Storable::canonical = 1;
my ($seed, $dir, $n) = @ARGV;
srand($seed);

sub cps { my $s = shift; return join(".", map { sprintf("%x", ord) } split //, $s); }
my %seen; my $count;
sub dumpv {
    my $x = shift;
    return "u" unless defined $x;
    return "r" . dumpref($x) if ref $x;
    return "s" . cps("$x");
}
sub dumpref {
    my $r = shift;
    my $a = refaddr($r);
    return "^" . $seen{$a} if exists $seen{$a};
    my $me = $count++;
    $seen{$a} = $me;
    my $cls = blessed($r);
    my $pre = defined $cls ? "B" . unpack("H*", $cls) . ":" : "";
    my $t = reftype($r);
    if ($t eq 'ARRAY') { return $pre . "A$me(" . join(",", map { dumpv($_) } @$r) . ")"; }
    if ($t eq 'HASH') { return $pre . "H$me(" . join(",", map { cps($_) . "=" . dumpv($r->{$_}) } sort keys %$r) . ")"; }
    return $pre . "S$me:" . dumpv($$r);
}

sub str {
    my $len = int(rand(10));
    my $s = '';
    for (1 .. $len) {
        my $k = int(rand(5));
        $s .= $k == 0 ? chr(0xe9) : $k == 1 ? chr(0x263a) : $k == 2 ? chr(int(rand(32))) : chr(97 + int(rand(26)));
    }
    $s .= "y" x 300 if rand() < 0.05;
    return $s;
}

sub value {
    my $d = shift;
    my $k = int(rand($d > 3 ? 7 : 12));
    return undef if $k == 0;
    return int(rand(256)) - 128 if $k == 1;
    return int(rand(2 ** 31)) * (rand() < 0.5 ? -1 : 1) if $k == 2;
    return int(rand(2 ** 50)) - 2 ** 49 if $k == 3;
    return (rand() - 0.5) * 10 ** int(rand(20)) if $k == 4;
    return str() if $k == 5;
    return \ str() if $k == 6;
    return [ map { value($d + 1) } 1 .. int(rand(4)) ] if $k == 7;
    return { map { (str() => value($d + 1)) } 1 .. int(rand(4)) } if $k == 8;
    if ($k == 9) { my $x = [1]; return [ $x, $x, \$x ]; }
    if ($k == 10) { return bless({ name => str(), list => value($d + 1) }, rand() < 0.5 ? "Foo::Bar" : "Baz"); }
    my $h = { me => undef }; $h->{me} = $h; return $h;
}

for my $i (0 .. $n - 1) {
    my $v = value(0);
    my $root = ref $v ? $v : \$v;
    my $how = $i % 4;
    my $f = "$dir/s$i.st";
    if ($how == 0) { open(my $o, '>:raw', $f); print $o nfreeze($root); close $o; }
    elsif ($how == 1) { open(my $o, '>:raw', $f); print $o freeze($root); close $o; }
    elsif ($how == 2) { nstore($root, $f); }
    else { store($root, $f); }
    my $back = retrieve($f) if $how >= 2;
    if ($how < 2) { open(my $in, '<:raw', $f); local $/; my $d = <$in>; $back = thaw($d); }
    %seen = (); $count = 0;
    print "$f ", dumpref($back), "\n";
}
