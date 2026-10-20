// The oracle for packages/goencoding: Go's encoding/base64 and
// encoding/base32. Data is hex, '-' when empty.
package main

import (
	"bufio"
	"encoding/base32"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"os"
	"strings"
)

func unhex(h string) []byte {
	if h == "-" {
		return nil
	}
	b, _ := hex.DecodeString(h)
	return b
}

func hx(b []byte) string {
	if len(b) == 0 {
		return "-"
	}
	return hex.EncodeToString(b)
}

func enc64(name string) *base64.Encoding {
	var e *base64.Encoding
	switch {
	case strings.HasPrefix(name, "url"):
		e = base64.URLEncoding
	default:
		e = base64.StdEncoding
	}
	if strings.Contains(name, "raw") {
		e = e.WithPadding(base64.NoPadding)
	}
	if strings.Contains(name, "star") {
		e = e.WithPadding('*')
	}
	if strings.HasSuffix(name, "S") {
		e = e.Strict()
	}
	return e
}

func enc32(name string) *base32.Encoding {
	e := base32.StdEncoding
	if strings.HasPrefix(name, "hex") {
		e = base32.HexEncoding
	}
	if strings.Contains(name, "raw") {
		e = e.WithPadding(base32.NoPadding)
	}
	if strings.Contains(name, "star") {
		e = e.WithPadding('*')
	}
	return e
}

func line(f []string) string {
	switch f[0] {
	case "e64":
		return hx([]byte(enc64(f[1]).EncodeToString(unhex(f[2]))))
	case "d64":
		b, err := enc64(f[1]).DecodeString(string(unhex(f[2])))
		if err != nil {
			return hx(b) + "|" + hx([]byte(err.Error()))
		}
		return hx(b) + "|-"
	case "e32":
		return hx([]byte(enc32(f[1]).EncodeToString(unhex(f[2]))))
	}
	b, err := enc32(f[1]).DecodeString(string(unhex(f[2])))
	if err != nil {
		return hx(b) + "|" + hx([]byte(err.Error()))
	}
	return hx(b) + "|-"
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
