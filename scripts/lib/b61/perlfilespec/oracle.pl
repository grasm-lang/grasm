# File::Spec::Unix (PathTools 3.80, Perl 5.34), one call per line; strings
# as hex ("-" empty), lists as comma-joined hex ("~" for none):
#   cp <p>  cd <list>  cf <list>  sd <p>  sp <p> <0|1>  ct <v> <d> <f>
#   ab <p>  nu <list>  ra <path> <base> <cwd>  ar <path> <base> <cwd>
# Cwd::getcwd is replaced by the <cwd> given.
use strict;
use warnings;
use File::Spec::Unix;
binmode STDOUT;
$| = 1;
my $CWD = "/";
{ no warnings 'redefine'; *Cwd::getcwd = sub { $CWD }; }
my $F = 'File::Spec::Unix';
sub unhex { my $h = shift; return "" if $h eq "-"; my $b = pack("H*", $h); utf8::decode($b); return $b }
sub hx { my $s = shift; return "~" unless defined $s; utf8::encode($s); my $h = unpack("H*", $s); return $h eq "" ? "-" : $h }
sub list { my $l = shift; return () if $l eq "~"; return map { unhex($_) } split /,/, $l, -1 }
sub hlist { return @_ ? join(",", map { hx($_) } @_) : "~" }
while (my $line = <STDIN>) {
    chomp $line;
    my ($op, @f) = split / /, $line;
    my $out;
    if ($op eq "cp") { $out = hx($F->canonpath(unhex($f[0]))) }
    elsif ($op eq "cd") { $out = hx($F->catdir(list($f[0]))) }
    elsif ($op eq "cf") { $out = hx($F->catfile(list($f[0]))) }
    elsif ($op eq "sd") { $out = hlist($F->splitdir(unhex($f[0]))) }
    elsif ($op eq "sp") { $out = join(" ", map { hx($_) } $F->splitpath(unhex($f[0]), $f[1])) }
    elsif ($op eq "ct") { $out = hx($F->catpath(unhex($f[0]), unhex($f[1]), unhex($f[2]))) }
    elsif ($op eq "ab") { $out = $F->file_name_is_absolute(unhex($f[0])) ? 1 : 0 }
    elsif ($op eq "nu") { $out = hlist($F->no_upwards(list($f[0]))) }
    elsif ($op eq "ra") { $CWD = unhex($f[2]); $out = hx($F->rel2abs(unhex($f[0]), unhex($f[1]))) }
    else { $CWD = unhex($f[2]); $out = hx($F->abs2rel(unhex($f[0]), unhex($f[1]))) }
    print $out, "\n";
}
