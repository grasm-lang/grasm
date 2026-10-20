// The oracle for packages/goflag: Go's flag package. A line is "p" or "d",
// the flags as kind:name:default:usage (hex, '-' empty, '~' no default),
// and for "p" the arguments (hex, comma separated, '~' for none).
package main

import (
	"bufio"
	"encoding/hex"
	"flag"
	"fmt"
	"io"
	"os"
	"sort"
	"strings"
	"time"
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

func define(fs *flag.FlagSet, spec string) error {
	p := strings.Split(spec, ":")
	name, usage := unhex(p[1]), unhex(p[3])
	def := p[2]
	tmp := flag.NewFlagSet("t", flag.ContinueOnError)
	tmp.SetOutput(io.Discard)
	var err error
	set := func(v flag.Value) {
		if def != "~" {
			err = v.Set(unhex(def))
		}
	}
	switch p[0] {
	case "b":
		v := tmp.Bool("x", false, "")
		set(tmp.Lookup("x").Value)
		fs.Bool(name, *v, usage)
	case "i":
		v := tmp.Int("x", 0, "")
		set(tmp.Lookup("x").Value)
		fs.Int(name, *v, usage)
	case "I":
		v := tmp.Int64("x", 0, "")
		set(tmp.Lookup("x").Value)
		fs.Int64(name, *v, usage)
	case "u":
		v := tmp.Uint("x", 0, "")
		set(tmp.Lookup("x").Value)
		fs.Uint(name, *v, usage)
	case "U":
		v := tmp.Uint64("x", 0, "")
		set(tmp.Lookup("x").Value)
		fs.Uint64(name, *v, usage)
	case "s":
		v := tmp.String("x", "", "")
		set(tmp.Lookup("x").Value)
		fs.String(name, *v, usage)
	case "f":
		v := tmp.Float64("x", 0, "")
		set(tmp.Lookup("x").Value)
		fs.Float64(name, *v, usage)
	default:
		v := tmp.Duration("x", time.Duration(0), "")
		set(tmp.Lookup("x").Value)
		fs.Duration(name, *v, usage)
	}
	return err
}

func line(f []string) string {
	fs := flag.NewFlagSet("t", flag.ContinueOnError)
	var buf strings.Builder
	fs.SetOutput(&buf)
	if f[1] != "~" {
		for _, spec := range strings.Split(f[1], ",") {
			if err := define(fs, spec); err != nil {
				return "BADDEF"
			}
		}
	}
	if f[0] == "d" {
		buf.Reset()
		fs.PrintDefaults()
		return hx(buf.String())
	}
	var args []string
	if f[2] != "~" {
		for _, a := range strings.Split(f[2], ",") {
			args = append(args, unhex(a))
		}
	}
	if err := fs.Parse(args); err != nil {
		return "E:" + hx(err.Error())
	}
	var vals, set, rest []string
	fs.VisitAll(func(fl *flag.Flag) { vals = append(vals, hx(fl.Name)+"="+hx(fl.Value.String())) })
	fs.Visit(func(fl *flag.Flag) { set = append(set, hx(fl.Name)) })
	for _, a := range fs.Args() {
		rest = append(rest, hx(a))
	}
	sort.Strings(set)
	j := func(x []string) string {
		if len(x) == 0 {
			return "~"
		}
		return strings.Join(x, ",")
	}
	return j(vals) + " " + j(set) + " " + j(rest)
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
