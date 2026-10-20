# MIME::Base64 and MIME::QuotedPrint 3.16, one call per line, bytes as hex
# ("-" for empty, "~" for an argument not given):
#   e64 <data> <eol>   d64 <text>   l64 <data> <eol>   dl64 <text>
#   eu <data>          du <text>    eqp <data> <eol> <binary>   dqp <text>
use strict;
use warnings;
use MIME::Base64 qw(encode_base64 decode_base64 encoded_base64_length decoded_base64_length encode_base64url decode_base64url);
use MIME::QuotedPrint qw(encode_qp decode_qp);
binmode STDOUT;
$| = 1;
sub unhex { my $h = shift; return "" if $h eq "-"; return pack("H*", $h) }
sub hx { my $h = unpack("H*", shift); return $h eq "" ? "-" : $h }
while (my $line = <STDIN>) {
    chomp $line;
    my ($op, @f) = split / /, $line;
    my $out;
    if ($op eq "e64") { $out = hx($f[1] eq "~" ? encode_base64(unhex($f[0])) : encode_base64(unhex($f[0]), unhex($f[1]))) }
    elsif ($op eq "d64") { $out = hx(decode_base64(unhex($f[0]))) }
    elsif ($op eq "l64") { $out = $f[1] eq "~" ? encoded_base64_length(unhex($f[0])) : encoded_base64_length(unhex($f[0]), unhex($f[1])) }
    elsif ($op eq "dl64") { $out = decoded_base64_length(unhex($f[0])) }
    elsif ($op eq "eu") { $out = hx(encode_base64url(unhex($f[0]))) }
    elsif ($op eq "du") { $out = hx(decode_base64url(unhex($f[0]))) }
    elsif ($op eq "eqp") {
        my @a = (unhex($f[0]));
        push @a, ($f[1] eq "~" ? undef : unhex($f[1])) if $f[1] ne "~" || $f[2] ne "~";
        push @a, $f[2] if $f[2] ne "~";
        $out = hx(&encode_qp(@a));
    }
    elsif ($op eq "dqp") { $out = hx(decode_qp(unhex($f[0]))) }
    print $out, "\n";
}
