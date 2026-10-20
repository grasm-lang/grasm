// Oracle for goversion: Go 1.25's go/version.
// Input lines: <hex x> <hex y>; output: <Compare(x,y)> <IsValid(x)> <hex Lang(x)>
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"go/version"
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

func main() {
	in := bufio.NewScanner(os.Stdin)
	w := bufio.NewWriter(os.Stdout)
	defer w.Flush()
	for in.Scan() {
		f := strings.Split(in.Text(), " ")
		x, y := unhex(f[0]), unhex(f[1])
		v := 0
		if version.IsValid(x) {
			v = 1
		}
		fmt.Fprintf(w, "%d %d %s\n", version.Compare(x, y), v, hx(version.Lang(x)))
	}
}
