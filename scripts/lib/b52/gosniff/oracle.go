// The oracle for packages/gosniff: Go's http.DetectContentType.
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"net/http"
	"os"
	"strings"
)

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
		var data []byte
		if f[1] != "-" {
			data, _ = hex.DecodeString(f[1])
		}
		fmt.Fprintln(out, http.DetectContentType(data))
	}
}
