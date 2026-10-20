// The oracle for packages/goduration: Go's time.Duration text and methods.
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"math"
	"os"
	"strconv"
	"strings"
	"time"
)

func bits(x float64) string {
	return strconv.FormatUint(math.Float64bits(x), 16)
}

func line(f []string) string {
	switch f[0] {
	case "pd":
		s := ""
		if f[1] != "-" {
			b, _ := hex.DecodeString(f[1])
			s = string(b)
		}
		d, err := time.ParseDuration(s)
		if err != nil {
			return "E" + hex.EncodeToString([]byte(err.Error()))
		}
		return strconv.FormatInt(int64(d), 10)
	case "ds":
		n, _ := strconv.ParseInt(f[1], 10, 64)
		d := time.Duration(n)
		return hex.EncodeToString([]byte(d.String())) + " " + bits(d.Hours()) + " " + bits(d.Minutes()) + " " + bits(d.Seconds()) + " " +
			strconv.FormatInt(int64(d.Abs()), 10)
	}
	n, _ := strconv.ParseInt(f[1], 10, 64)
	m, _ := strconv.ParseInt(f[2], 10, 64)
	d := time.Duration(n)
	return strconv.FormatInt(int64(d.Round(time.Duration(m))), 10) + " " + strconv.FormatInt(int64(d.Truncate(time.Duration(m))), 10)
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
