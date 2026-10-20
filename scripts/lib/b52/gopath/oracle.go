// The oracle for packages/gopath: Go's path and path/filepath on Unix.
// Text is hex, '-' when empty; lists are joined by ','.
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"os"
	"path"
	"path/filepath"
	"strings"
)

func unhex(h string) string {
	if h == "-" {
		return ""
	}
	b, _ := hex.DecodeString(h)
	return string(b)
}

func hx(s string) string {
	if s == "" {
		return "-"
	}
	return hex.EncodeToString([]byte(s))
}

func list(h string) []string {
	if h == "~" {
		return nil
	}
	var out []string
	for _, x := range strings.Split(h, ",") {
		out = append(out, unhex(x))
	}
	return out
}

func line(f []string) string {
	switch f[0] {
	case "cl":
		return hx(path.Clean(unhex(f[1])))
	case "jo":
		return hx(path.Join(list(f[1])...))
	case "fj":
		return hx(filepath.Join(list(f[1])...))
	case "sp":
		d, b := path.Split(unhex(f[1]))
		return hx(d) + " " + hx(b)
	case "ex":
		return hx(path.Ext(unhex(f[1])))
	case "ba":
		return hx(path.Base(unhex(f[1])))
	case "di":
		return hx(path.Dir(unhex(f[1])))
	case "ia":
		return fmt.Sprint(path.IsAbs(unhex(f[1])))
	case "ma":
		ok, err := path.Match(unhex(f[1]), unhex(f[2]))
		if err != nil {
			return "E" + hx(err.Error())
		}
		return fmt.Sprint(ok)
	case "re":
		r, err := filepath.Rel(unhex(f[1]), unhex(f[2]))
		if err != nil {
			return "E" + hx(err.Error())
		}
		return hx(r)
	}
	parts := filepath.SplitList(unhex(f[1]))
	out := make([]string, len(parts))
	for i, p := range parts {
		out[i] = hx(p)
	}
	return "[" + strings.Join(out, ",") + "]"
}

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<26)
	out := bufio.NewWriter(os.Stdout)
	defer out.Flush()
	for in.Scan() {
		if t := in.Text(); t != "" {
			fmt.Fprintln(out, line(strings.Split(t, " ")))
		}
	}
}
