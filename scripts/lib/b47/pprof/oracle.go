// The oracle for packages/pprof: the profile package Go itself vendors
// (cmd/vendor/github.com/google/pprof/profile), copied next to this file at
// gate time. Reads paths on stdin and prints, for each, the hex of
// Profile.String(), or `error`.
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"os"

	"pporacle/profile"
)

func main() {
	sc := bufio.NewScanner(os.Stdin)
	for sc.Scan() {
		data, err := os.ReadFile(sc.Text())
		if err != nil {
			fmt.Println("unreadable")
			continue
		}
		p, err := profile.ParseData(data)
		if err != nil {
			fmt.Println("error")
			continue
		}
		fmt.Println(hex.EncodeToString([]byte(p.String())))
	}
}
