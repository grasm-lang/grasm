// Oracle for gostrings: Go 1.25's strings package and unicode case mapping.
// Each input line is an operation and hex arguments; each output one line.
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"os"
	"strconv"
	"strings"
	"unicode"
)

func unhex(s string) string {
	if s == "-" {
		return ""
	}
	b, err := hex.DecodeString(s)
	if err != nil {
		panic(err)
	}
	return string(b)
}

func hx(s string) string {
	if s == "" {
		return "-"
	}
	return hex.EncodeToString([]byte(s))
}

func list(xs []string) string {
	if len(xs) == 0 {
		return "~"
	}
	out := make([]string, len(xs))
	for i, x := range xs {
		out[i] = hx(x)
	}
	return strings.Join(out, ",")
}

func b(v bool) string {
	if v {
		return "1"
	}
	return "0"
}

func atoi(s string) int {
	n, err := strconv.Atoi(s)
	if err != nil {
		panic(err)
	}
	return n
}

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<24)
	w := bufio.NewWriter(os.Stdout)
	defer w.Flush()
	for in.Scan() {
		f := strings.Split(in.Text(), " ")
		a := func(i int) string { return unhex(f[i]) }
		switch f[0] {
		case "F":
			fmt.Fprintln(w, list(strings.Fields(a(1))))
		case "E":
			fmt.Fprintln(w, b(strings.EqualFold(a(1), a(2))))
		case "U":
			s := a(1)
			fmt.Fprintln(w, hx(strings.ToUpper(s)), hx(strings.ToLower(s)), hx(strings.ToTitle(s)), hx(strings.Title(s)))
		case "V":
			fmt.Fprintln(w, hx(strings.ToValidUTF8(a(1), a(2))))
		case "S":
			s, sep, n := a(1), a(2), atoi(f[3])
			fmt.Fprintln(w, list(strings.SplitN(s, sep, n)), list(strings.SplitAfterN(s, sep, n)), list(strings.Split(s, sep)), list(strings.SplitAfter(s, sep)))
		case "C":
			s, sep := a(1), a(2)
			x, y, ok := strings.Cut(s, sep)
			p, okp := strings.CutPrefix(s, sep)
			q, oks := strings.CutSuffix(s, sep)
			fmt.Fprintln(w, hx(x), hx(y), b(ok), hx(p), b(okp), hx(q), b(oks), strings.Count(s, sep), strings.Index(s, sep), strings.LastIndex(s, sep))
		case "A":
			s, chars := a(1), a(2)
			fmt.Fprintln(w, strings.IndexAny(s, chars), strings.LastIndexAny(s, chars), b(strings.ContainsAny(s, chars)))
		case "I":
			r := rune(atoi(f[2]))
			fmt.Fprintln(w, strings.IndexRune(a(1), r), b(strings.ContainsRune(a(1), r)))
		case "R":
			fmt.Fprintln(w, hx(strings.Replace(a(1), a(2), a(3), atoi(f[4]))), hx(strings.ReplaceAll(a(1), a(2), a(3))))
		case "P":
			var args []string
			for i := 2; i < len(f); i++ {
				args = append(args, a(i))
			}
			fmt.Fprintln(w, hx(strings.NewReplacer(args...).Replace(a(1))))
		case "T":
			s, c := a(1), a(2)
			fmt.Fprintln(w, hx(strings.Trim(s, c)), hx(strings.TrimLeft(s, c)), hx(strings.TrimRight(s, c)), hx(strings.TrimPrefix(s, c)), hx(strings.TrimSuffix(s, c)), hx(strings.TrimSpace(s)))
		case "K":
			r := rune(atoi(f[1]))
			fmt.Fprintln(w, unicode.ToUpper(r), unicode.ToLower(r), unicode.ToTitle(r), unicode.SimpleFold(r))
		}
	}
}
