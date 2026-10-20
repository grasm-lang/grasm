// The oracle for packages/gomime: Go's own mime package.
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"mime"
	"os"
	"slices"
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

func run(f []string) string {
	switch f[0] {
	case "pm":
		t, params, err := mime.ParseMediaType(unhex(f[1]))
		keys := []string{}
		for k := range params {
			keys = append(keys, k)
		}
		slices.Sort(keys)
		parts := []string{hx(t)}
		for _, k := range keys {
			parts = append(parts, hx(k)+"="+hx(params[k]))
		}
		e := "-"
		if err != nil {
			e = hx(err.Error())
		}
		return strings.Join(parts, ",") + " " + e
	case "fm":
		params := map[string]string{}
		for i := 2; i+1 < len(f); i += 2 {
			params[unhex(f[i])] = unhex(f[i+1])
		}
		return hx(mime.FormatMediaType(unhex(f[1]), params))
	case "we":
		enc := mime.QEncoding
		if f[1] == "b" {
			enc = mime.BEncoding
		}
		return hx(enc.Encode(unhex(f[2]), unhex(f[3])))
	case "wd":
		s, err := new(mime.WordDecoder).Decode(unhex(f[1]))
		if err != nil {
			return "E " + hx(err.Error())
		}
		return hx(s)
	default:
		s, err := new(mime.WordDecoder).DecodeHeader(unhex(f[1]))
		if err != nil {
			return "E " + hx(err.Error())
		}
		return hx(s)
	}
}

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<20)
	out := bufio.NewWriter(os.Stdout)
	for in.Scan() {
		if in.Text() != "" {
			fmt.Fprintln(out, run(strings.Split(in.Text(), " ")))
		}
	}
	out.Flush()
}
