# Oracle for scripts/b40_diff.py: each file of TAP read by TAP::Parser,
# printed as b40test prints its own reading.
use strict;
use warnings;
use TAP::Parser;

sub hs { my $s = shift; utf8::encode($s) if utf8::is_utf8($s); return unpack("H*", $s); }

for my $f (@ARGV) {
    open(my $fh, '<:raw', $f) or die;
    local $/;
    my $text = <$fh>;
    my $p = TAP::Parser->new({ tap => $text });
    my @toks;
    while (my $r = $p->next) {
        my $t = $r->type;
        if ($t eq 'version') { push @toks, "V" . $r->version; }
        elsif ($t eq 'plan') { push @toks, "P" . $r->tests_planned . "," . ($r->directive // '') . "," . hs($r->explanation // '') . "," . join(",", @{ $r->todo_list }); }
        elsif ($t eq 'test') { push @toks, "T" . ($r->ok eq 'ok' ? 'true' : 'false') . "," . (defined $r->{test_num} ? $r->{test_num} : -1) . "," . hs($r->description // '') . "," . ($r->{directive} // '') . "," . hs($r->explanation // ''); }
        elsif ($t eq 'comment') { push @toks, "C" . hs($r->comment); }
        elsif ($t eq 'bailout') { push @toks, "B" . hs($r->explanation); }
        elsif ($t eq 'yaml') { push @toks, "Y" . hs($r->raw); }
        elsif ($t eq 'pragma') { push @toks, "G" . join(",", $r->pragmas); }
        else { push @toks, "U" . hs($r->raw); }
    }
    my @errs = map { hs($_) } $p->parse_errors;
    my $list = sub { my @v = @_; return join(",", @v); };
    my $sk = defined $p->skip_all ? hs($p->skip_all) : '-';
    print join(";", @toks), "|", $p->version, ",", ($p->tests_planned // -1), ",", $p->tests_run, ",",
        $list->($p->passed), "/", $list->($p->failed), "/", $list->($p->actual_passed), "/", $list->($p->actual_failed), "/",
        $list->($p->todo), "/", $list->($p->todo_passed), "/", $list->($p->skipped), "|", $sk, "|-|",
        ($p->is_good_plan ? "true" : "false"), "|", join(",", @errs), "\n";
}
