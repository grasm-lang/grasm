// The oracle for packages/goencoding: Go's encoding/ascii85. Data is hex,
// '-' when empty.
package main

import (
	"bufio"
	"encoding/ascii85"
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

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<26)
	out := bufio.NewWriter(os.Stdout)
	defer out.Flush()
	for in.Scan() {
		t := in.Text()
		if t == "" {
			continue
		}
		f := strings.Split(t, " ")
		data := unhex(f[1])
		if f[0] == "en" {
			dst := make([]byte, ascii85.MaxEncodedLen(len(data)))
			n := ascii85.Encode(dst, data)
			fmt.Fprintln(out, hx(dst[:n]))
			continue
		}
		dst := make([]byte, 4*len(data)+8)
		ndst, nsrc, err := ascii85.Decode(dst, data, f[2] == "1")
		e := "-"
		if err != nil {
			e = hx([]byte(err.Error()))
		}
		fmt.Fprintf(out, "%s %d %s\n", hx(dst[:ndst]), nsrc, e)
	}
}
