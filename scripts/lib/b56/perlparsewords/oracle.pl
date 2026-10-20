#!/usr/bin/perl
# The oracle for packages/perlparsewords: Text::ParseWords 3.30 under Perl
# 5.34, on character strings. Text travels as hex of UTF-8 ("-" when
# empty); a word answers as s<hex> or u (undef), joined by ","; nested
# lists are joined by ";"; old_shellwords' unmatched quote answers W.
use strict;
use warnings;
use Encode ();
use Text::ParseWords qw(shellwords quotewords nested_quotewords parse_line old_shellwords);

sub unhex { my $h = shift; return $h eq '-' ? '' : Encode::decode('UTF-8', pack('H*', $h)) }
sub w { my $x = shift; return 'u' unless defined $x; my $h = unpack('H*', Encode::encode('UTF-8', $x)); return 's' . ($h eq '' ? '-' : $h) }
sub keep { my $k = shift; return $k eq 'd' ? 'delimiters' : $k }

my @out;
while (my $l = <STDIN>) {
    chomp $l;
    next if $l eq '';
    my @f = split / /, $l;
    shift @f;
    my $op = shift @f;
    my $r;
    if ($op eq 'pl') {
        $r = join ',', map { w($_) } parse_line(unhex($f[0]), keep($f[1]), unhex($f[2]));
    } elsif ($op eq 'qw') {
        $r = join ',', map { w($_) } quotewords(unhex($f[0]), keep($f[1]), map { unhex($_) } @f[2 .. $#f]);
    } elsif ($op eq 'nq') {
        $r = join ';', map { join ',', map { w($_) } @$_ } nested_quotewords(unhex($f[0]), keep($f[1]), map { unhex($_) } @f[2 .. $#f]);
    } elsif ($op eq 'sw') {
        $r = join ',', map { w($_) } shellwords(map { unhex($_) } @f);
    } else {
        my $warned = 0;
        local $SIG{__WARN__} = sub { $warned = 1 };
        my @words = old_shellwords(map { unhex($_) } @f);
        $r = $warned ? 'W' : join ',', map { w($_) } @words;
    }
    push @out, $r;
}
print map { "$_\n" } @out;
