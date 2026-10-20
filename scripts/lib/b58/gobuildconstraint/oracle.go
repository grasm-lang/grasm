// Oracle for gobuildconstraint: Go 1.25's go/build/constraint.
// Input lines: p <hex line> <comma-separated hex tags or ~>
package main

import (
	"bufio"
	"encoding/hex"
	"errors"
	"fmt"
	"go/build/constraint"
	"os"
	"strings"
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

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<24)
	w := bufio.NewWriter(os.Stdout)
	defer w.Flush()
	for in.Scan() {
		f := strings.Split(in.Text(), " ")
		line := unhex(f[1])
		var tags []string
		if f[2] != "~" {
			for _, t := range strings.Split(f[2], ",") {
				tags = append(tags, unhex(t))
			}
		}
		kinds := b(constraint.IsGoBuild(line)) + b(constraint.IsPlusBuild(line))
		x, err := constraint.Parse(line)
		if err != nil {
			off := -1
			var se *constraint.SyntaxError
			if errors.As(err, &se) {
				off = se.Offset
			}
			fmt.Fprintf(w, "E %s %d %s\n", kinds, off, hx(err.Error()))
			continue
		}
		var asked []string
		v := x.Eval(func(t string) bool {
			asked = append(asked, t)
			for _, k := range tags {
				if k == t {
					return true
				}
			}
			return false
		})
		pbl := ""
		lines, perr := constraint.PlusBuildLines(x)
		if perr != nil {
			pbl = "E:" + hx(perr.Error())
		} else {
			pbl = list(lines)
		}
		fmt.Fprintf(w, "OK %s %s %s %s %s %s\n", kinds, hx(x.String()), b(v), list(asked), hx(constraint.GoVersion(x)), pbl)
	}
}
