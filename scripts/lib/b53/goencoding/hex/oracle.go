// The oracle for packages/goencoding: Go's encoding/hex. Data is hex, '-'
// when empty.
package main

import (
	"bufio"
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
		switch f[0] {
		case "en":
			fmt.Fprintln(out, hx([]byte(hex.EncodeToString(data))))
		case "de":
			b, err := hex.DecodeString(string(data))
			e := "-"
			if err != nil {
				e = hx([]byte(err.Error()))
			}
			fmt.Fprintln(out, hx(b)+"|"+e)
		default:
			fmt.Fprintln(out, hx([]byte(hex.Dump(data))))
		}
	}
}
