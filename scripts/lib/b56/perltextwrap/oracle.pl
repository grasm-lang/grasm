#!/usr/bin/perl
# The oracle for packages/perltextwrap: Text::Wrap 2013.0523 and
# Text::Tabs under Perl 5.34, on character strings. Text travels as hex of
# UTF-8 ("-" when empty, "u" for undef); the answer is the hex of the
# result, or D when Text::Wrap dies.
use strict;
use warnings;
use Encode ();
use Text::Wrap ();
use Text::Tabs ();

sub unhex { my $h = shift; return undef if $h eq 'u'; my $s = $h eq '-' ? '' : Encode::decode('UTF-8', pack('H*', $h)); utf8::upgrade($s); return $s }
sub hx { my $h = unpack('H*', Encode::encode('UTF-8', shift)); return $h eq '' ? '-' : $h }

local $SIG{__WARN__} = sub { };
my @out;
while (my $l = <STDIN>) {
    chomp $l;
    next if $l eq '';
    my @f = split / /, $l;
    shift @f;
    my $op = shift @f;
    if ($op eq 'e' || $op eq 'x') {
        local $Text::Tabs::tabstop = $f[0];
        my $t = unhex($f[1]);
        push @out, hx($op eq 'e' ? Text::Tabs::expand($t) : Text::Tabs::unexpand($t));
        next;
    }
    my ($columns, $huge, $unexpand, $tabstop, $sep, $sep2, $brk, $ip, $xp, @t) = @f;
    local $Text::Wrap::columns = $columns;
    local $Text::Wrap::huge = $huge;
    local $Text::Wrap::unexpand = $unexpand;
    local $Text::Wrap::tabstop = $tabstop;
    local $Text::Wrap::separator = unhex($sep);
    local $Text::Wrap::separator2 = unhex($sep2);
    local $Text::Wrap::break = defined(unhex($brk)) ? unhex($brk) : '(?=\s)\X';
    my @texts = map { unhex($_) } @t;
    my $r = eval { $op eq 'w' ? Text::Wrap::wrap(unhex($ip), unhex($xp), @texts) : Text::Wrap::fill(unhex($ip), unhex($xp), @texts) };
    push @out, defined($r) ? hx($r) : 'D';
}
print map { "$_\n" } @out;
