// The oracle for packages/gostrconv: Go's own strconv. Each input line is
// an operation with hex-encoded text arguments; each answer is one line.
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"math"
	"os"
	"strconv"
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

func errText(err error) string {
	if err == nil {
		return "-"
	}
	return hx(err.Error())
}

func run(f []string) string {
	switch f[0] {
	case "ff":
		bits, _ := strconv.ParseUint(f[1], 16, 64)
		prec, _ := strconv.Atoi(f[3])
		size, _ := strconv.Atoi(f[4])
		return hx(strconv.FormatFloat(math.Float64frombits(bits), f[2][0], prec, size))
	case "pf":
		size, _ := strconv.Atoi(f[2])
		v, err := strconv.ParseFloat(unhex(f[1]), size)
		return fmt.Sprintf("%x %s", math.Float64bits(v), errText(err))
	case "pi":
		base, _ := strconv.Atoi(f[2])
		size, _ := strconv.Atoi(f[3])
		v, err := strconv.ParseInt(unhex(f[1]), base, size)
		return fmt.Sprintf("%d %s", v, errText(err))
	case "pu":
		base, _ := strconv.Atoi(f[2])
		size, _ := strconv.Atoi(f[3])
		v, err := strconv.ParseUint(unhex(f[1]), base, size)
		return fmt.Sprintf("%d %s", v, errText(err))
	case "ai":
		v, err := strconv.Atoi(unhex(f[1]))
		return fmt.Sprintf("%d %s", v, errText(err))
	case "pb":
		v, err := strconv.ParseBool(unhex(f[1]))
		return fmt.Sprintf("%t %s", v, errText(err))
	case "fi":
		v, _ := strconv.ParseInt(f[1], 10, 64)
		base, _ := strconv.Atoi(f[2])
		return strconv.FormatInt(v, base)
	default:
		v, _ := strconv.ParseUint(f[1], 10, 64)
		base, _ := strconv.Atoi(f[2])
		return strconv.FormatUint(v, base)
	}
}

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<20)
	out := bufio.NewWriter(os.Stdout)
	for in.Scan() {
		line := in.Text()
		if line == "" {
			continue
		}
		fmt.Fprintln(out, run(strings.Split(line, " ")))
	}
	out.Flush()
}
