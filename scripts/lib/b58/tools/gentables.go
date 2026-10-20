// Prints Go's unicode tables as flat integer lists for embedding in Grasm
// packages: `letter`, `digit` and `space` as range pairs (lo, hi);
// `case` as Go's CaseRanges (lo, hi, upper delta, lower delta, title delta,
// with 1114112 for UpperLower); `orbit` as the SimpleFold pairs (from, to)
// that the case ranges alone do not give (Go's unexported caseOrbit).
package main

import (
	"fmt"
	"os"
	"strings"
	"unicode"
)

func ranges(pred func(rune) bool) []int {
	var out []int
	in := false
	start := 0
	for r := 0; r <= unicode.MaxRune+1; r++ {
		ok := r <= unicode.MaxRune && pred(rune(r))
		if ok && !in {
			start = r
			in = true
		} else if !ok && in {
			out = append(out, start, r-1)
			in = false
		}
	}
	return out
}

func defaultFold(r rune) rune {
	if l := unicode.ToLower(r); l != r {
		return l
	}
	return unicode.ToUpper(r)
}

func main() {
	var t []int
	switch os.Args[1] {
	case "letter":
		t = ranges(unicode.IsLetter)
	case "digit":
		t = ranges(unicode.IsDigit)
	case "space":
		t = ranges(unicode.IsSpace)
	case "case":
		for _, cr := range unicode.CaseRanges {
			t = append(t, int(cr.Lo), int(cr.Hi), int(cr.Delta[0]), int(cr.Delta[1]), int(cr.Delta[2]))
		}
	case "orbit":
		for r := rune(0); r <= unicode.MaxRune; r++ {
			if f := unicode.SimpleFold(r); f != defaultFold(r) {
				t = append(t, int(r), int(f))
			}
		}
	}
	var b strings.Builder
	line := 0
	for i, v := range t {
		s := fmt.Sprintf("%d", v)
		if i > 0 {
			b.WriteString(",")
			line++
			if line+len(s) > 120 {
				b.WriteString("\n        ")
				line = 0
			} else {
				b.WriteString(" ")
				line++
			}
		}
		b.WriteString(s)
		line += len(s)
	}
	fmt.Println("        " + b.String())
}
