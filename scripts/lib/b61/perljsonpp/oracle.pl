# JSON::PP (4.06, as in Perl 5.34), one call per line:
#   enc <options> <value tokens...>   $json->encode(value)
#   dec <options> <text hex>          $json->decode(text)
# Options are comma-separated names (ascii, latin1, utf8, indent, pretty,
# space_before, space_after, canonical, escape_slash, relaxed, loose,
# allow_singlequote, allow_barekey), nonref=0, indent_length=N,
# max_depth=N, or "-". Value tokens: u t f i:<decimal> x:<64-bit hex of a
# double> s:<UTF-8 hex> [ ] { k:<UTF-8 hex> value ... }.
# Answers: R:<hex of the UTF-8 of the result> for encode, the value
# tokens for decode (n:<Perl's text> for a double, keys sorted), or
# E:<hex of the croak message, without " at FILE line N.">.
use strict;
use warnings;
no warnings 'numeric';
use JSON::PP;
use B;

# JSON::PP's number test adds 0 to a copy of the value, and which way
# Perl's add goes for an integral double from 2**53 to 2**62 depends on
# whether that op has already done a floating-point add in this process.
# Doing one now puts it in the state every later call sees.
JSON::PP->new->encode([0.5]);

sub unhex { my $h = shift; return '' if $h eq '-'; return pack('H*', $h); }

sub build {
    my ($toks) = @_;
    my $t = shift @$toks;
    return undef if $t eq 'u';
    return JSON::PP::true if $t eq 't';
    return JSON::PP::false if $t eq 'f';
    if ($t =~ /^i:(-?\d+)$/) { return 0 + "$1"; }
    if ($t =~ /^x:([0-9a-f]{16})$/) { return unpack('d>', pack('H16', $1)); }
    if ($t =~ /^s:(.*)$/) { my $s = unhex($1); utf8::decode($s); return $s; }
    if ($t eq '[') {
        my @a;
        while ($toks->[0] ne ']') { push @a, build($toks); }
        shift @$toks;
        return \@a;
    }
    if ($t eq '{') {
        my %h;
        while ($toks->[0] ne '}') {
            my $k = shift @$toks;
            $k =~ s/^k://;
            $k = unhex($k);
            utf8::decode($k);
            $h{$k} = build($toks);
        }
        shift @$toks;
        return \%h;
    }
    die "bad token $t";
}

sub hexs { my $s = shift; utf8::upgrade($s); utf8::encode($s); return length $s ? unpack('H*', $s) : '-'; }

sub dump_value {
    my ($v) = @_;
    return 'u' unless defined $v;
    if (ref $v eq 'JSON::PP::Boolean') { return $v ? 't' : 'f'; }
    if (ref $v eq 'ARRAY') { return join(' ', '[', (map { dump_value($_) } @$v), ']'); }
    if (ref $v eq 'HASH') {
        return join(' ', '{', (map { ('k:' . hexs($_), dump_value($v->{$_})) } sort keys %$v), '}');
    }
    my $f = B::svref_2object(\$v)->FLAGS;
    return 's:' . hexs($v) if $f & B::SVf_POK;
    return 'i:' . "$v" if $f & B::SVf_IOK;
    return 'n:' . "$v";
}

sub configure {
    my ($opts) = @_;
    my $j = JSON::PP->new;
    return $j if $opts eq '-';
    for my $o (split /,/, $opts) {
        if ($o eq 'nonref=0') { $j->allow_nonref(0); }
        elsif ($o =~ /^indent_length=(\d+)$/) { $j->indent_length($1); }
        elsif ($o =~ /^max_depth=(\d+)$/) { $j->max_depth($1); }
        else { $j->$o(1); }
    }
    return $j;
}

sub message {
    my $m = shift;
    $m =~ s/ at \S+ line \d+\.\n\z//;
    return 'E:' . hexs($m);
}

while (my $line = <STDIN>) {
    chomp $line;
    my @f = split / /, $line;
    my $op = shift @f;
    my $j = configure(shift @f);
    if ($op eq 'enc') {
        my $v = build(\@f);
        my $r = eval { $j->encode($v) };
        if (!defined $r) { print message($@), "\n"; next; }
        if ($j->get_utf8) { print 'R:', (length $r ? unpack('H*', $r) : '-'), "\n"; }
        else { print 'R:', hexs($r), "\n"; }
    } else {
        my $text = unhex($f[0]);
        utf8::decode($text) unless $j->get_utf8;
        my $ok = eval { my $v = $j->decode($text); print 'V:', dump_value($v), "\n"; 1 };
        print message($@), "\n" unless $ok;
    }
}
