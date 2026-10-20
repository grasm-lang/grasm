#!/usr/bin/perl
# The oracle for packages/perldumper: Data::Dumper 2.179 (XS) under Perl
# 5.34. Each line describes SVs as nodes; this builds them with the flags
# the description names, dumps the roots with the options given and
# Sortkeys on, and prints the output in hex, or E: and the croak.
use strict;
use warnings;
use Data::Dumper;
use Encode ();

sub unhex { my $h = shift; return $h eq '-' ? '' : pack('H*', $h) }

sub line {
    my ($opts, $nodes, $roots) = @_;
    my @n = map { [split /:/, $_, 2] } split /;/, $nodes;
    my (%A, %S, %inner);
    for my $i (0 .. $#n) {
        my ($k) = @{$n[$i]};
        $A{$i} = [] if $k eq 'a';
        $A{$i} = {} if $k eq 'h';
    }
    my $resolve;
    $resolve = sub { my $t = shift; return $n[$t][0] eq 'B' ? $resolve->((split m{/}, $n[$t][1])[1]) : $t };
    # Elements are SVs in their containers; a ref to one is a ref to the slot.
    for my $i (0 .. $#n) {
        my ($k, $a) = @{$n[$i]};
        if ($k eq 'a' && $a ne '-') {
            my @items = split /\./, $a;
            for my $j (0 .. $#items) { $A{$i}[$j] = undef; $S{$items[$j]} = \$A{$i}[$j]; }
        }
        if ($k eq 'h' && $a ne '-') {
            for my $e (split /\./, $a) {
                my ($kh, $f, $id) = split m{/}, $e;
                my $key = unhex($kh);
                $key = Encode::decode('UTF-8', $key) if $f;
                $A{$i}{$key} = undef;
                $S{$id} = \$A{$i}{$key};
            }
        }
    }
    for my $i (0 .. $#n) {
        my ($k) = @{$n[$i]};
        next if $k eq 'a' || $k eq 'h' || $k eq 'B' || exists $S{$i};
        if ($k eq 'U') { $S{$i} = \undef; next }
        my $fresh;
        $S{$i} = \$fresh;
        undef $fresh;
    }
    my $refto = sub { my $t = $resolve->(shift); return exists $A{$t} ? $A{$t} : $S{$t} };
    for my $i (0 .. $#n) {
        my ($k, $a) = @{$n[$i]};
        next if $k eq 'U' || $k eq 'a' || $k eq 'h' || $k eq 'B';
        my $s = $S{$i};
        if ($k eq 'u') { $$s = undef }
        elsif ($k eq 'y') { $$s = !!1 }
        elsif ($k eq 'n') { $$s = !!0 }
        elsif ($k eq 'i') { $$s = unpack('q>', pack('H16', $a)) }
        elsif ($k eq 'd') { $$s = unpack('d>', pack('H16', $a)) }
        elsif ($k eq 'b') { $$s = unhex($a) }
        elsif ($k eq 's') { $$s = Encode::decode('UTF-8', unhex($a)) }
        elsif ($k eq 'r') { $$s = $refto->($a) }
    }
    for my $i (0 .. $#n) {
        my ($k, $a) = @{$n[$i]};
        next unless $k eq 'B';
        my ($ch, $t) = split m{/}, $a;
        bless $refto->($t), unhex($ch);
    }
    my @vals;
    for my $r (split /\./, $roots) {
        my $k = $n[$r][0];
        push @vals, ($k eq 'a' || $k eq 'h' || $k eq 'B') ? $refto->($r) : ${$S{$r}};
    }
    my @o = split /,/, $opts;
    my @names = $o[12] eq '-' ? () : map { unhex($_) } split /\./, $o[12], -1;
    my $d = Data::Dumper->new(\@vals, \@names);
    $d->Indent($o[0])->Pad(unhex($o[1]))->Varname(unhex($o[2]))->Terse($o[3])->Useqq($o[4])->Quotekeys($o[5])
      ->Pair(unhex($o[6]))->Trailingcomma($o[7])->Purity($o[8])->Deepcopy($o[9])->Maxrecurse($o[10])->Bless(unhex($o[11]))
      ->Sortkeys(1);
    my $out = eval { $d->Dump };
    if (!defined $out) {
        my $e = $@;
        $e =~ s/ at \S+ line \d+(?:, <\w+> line \d+)?\.?\n?\z//;
        return 'E:' . unpack('H*', $e);
    }
    if (utf8::is_utf8($out)) { utf8::downgrade($out) }
    return length($out) ? unpack('H*', $out) : '-';
}

$| = 0;
while (my $l = <STDIN>) {
    chomp $l;
    next if $l eq '';
    my @f = split / /, $l;
    print line(@f[1 .. 3]), "\n";
}
