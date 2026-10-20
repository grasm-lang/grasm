# Getopt::Long 2.52: Configure("default", <config>) then
# GetOptionsFromArray(\@args, \%h, <specs>), one call per line:
#   <config> <specs> <args>
# each a comma-joined list of hex strings ("-" for an empty string, "~" for
# no items). Answers D:<hex message> when it dies, or
#   <1|0> <hex of the hash dump> <hex list of what is left> <hex list of warnings>
# The dump is one entry per key in sorted order, joined by newlines:
# key TAB S TAB value, key TAB A (TAB item)..., key TAB H (TAB k TAB v)...
use strict;
use warnings;
use Getopt::Long qw(GetOptionsFromArray);
binmode STDOUT;
$| = 1;
delete $ENV{POSIXLY_CORRECT};
my @w;
$SIG{__WARN__} = sub { my $m = $_[0]; $m =~ s/ at \S+ line \d+(?:, <\w+> line \d+)?\.\n\z//; $m =~ s/\n\z//; push @w, $m };
sub unhex { my $h = shift; return "" if $h eq "-"; my $b = pack("H*", $h); utf8::decode($b); return $b }
sub hx { my $s = shift; utf8::encode($s); my $h = unpack("H*", $s); return $h eq "" ? "-" : $h }
sub list { my $l = shift; return () if $l eq "~"; return map { unhex($_) } split /,/, $l, -1 }
sub hlist { return @_ ? join(",", map { hx($_) } @_) : "~" }
while (my $line = <STDIN>) {
    chomp $line;
    my ($c, $s, $a) = split / /, $line;
    my @cfg = list($c);
    my @specs = list($s);
    my @args = list($a);
    my %h;
    @w = ();
    my $r = eval { Getopt::Long::Configure("default", @cfg); GetOptionsFromArray(\@args, \%h, @specs) };
    if (!defined $r) {
        print "D:", hx($@), "\n";
        next;
    }
    my @dump;
    for my $k (sort keys %h) {
        my $v = $h{$k};
        if (ref $v eq "ARRAY") { push @dump, join("\t", $k, "A", map { defined $_ ? "$_" : "<undef>" } @$v) }
        elsif (ref $v eq "HASH") { push @dump, join("\t", $k, "H", map { ($_, defined $v->{$_} ? "$v->{$_}" : "<undef>") } sort keys %$v) }
        else { push @dump, join("\t", $k, "S", defined $v ? "$v" : "<undef>") }
    }
    print $r ? "1" : "0", " ", hx(join("\n", @dump)), " ", hlist(@args), " ", hlist(@w), "\n";
}
