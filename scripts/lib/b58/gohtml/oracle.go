// Oracle for gohtml: Go 1.25's html.EscapeString and html.UnescapeString.
// Input lines: e <hex> | u <hex>; output: the result as hex.
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"html"
	"os"
	"strings"
)

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<24)
	w := bufio.NewWriter(os.Stdout)
	defer w.Flush()
	for in.Scan() {
		f := strings.Split(in.Text(), " ")
		s := ""
		if f[1] != "-" {
			b, err := hex.DecodeString(f[1])
			if err != nil {
				panic(err)
			}
			s = string(b)
		}
		out := html.UnescapeString(s)
		if f[0] == "e" {
			out = html.EscapeString(s)
		}
		if out == "" {
			fmt.Fprintln(w, "-")
		} else {
			fmt.Fprintln(w, hex.EncodeToString([]byte(out)))
		}
	}
}
