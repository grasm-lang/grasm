// Oracle for gonetmail: Go 1.25's net/mail ParseAddress, ParseAddressList,
// Address.String and ParseDate. Run with TZ=UTC.
// Input lines: a <hex> | l <hex> | d <hex> | s <hex name> <hex address>
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"net/mail"
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

func addr(a *mail.Address) string {
	return hx(a.Name) + ":" + hx(a.Address) + ":" + hx(a.String())
}

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<24)
	w := bufio.NewWriter(os.Stdout)
	defer w.Flush()
	for in.Scan() {
		f := strings.Split(in.Text(), " ")
		switch f[0] {
		case "a":
			a, err := mail.ParseAddress(unhex(f[1]))
			if err != nil {
				fmt.Fprintln(w, "E", hx(err.Error()))
			} else {
				fmt.Fprintln(w, "OK", addr(a))
			}
		case "l":
			as, err := mail.ParseAddressList(unhex(f[1]))
			if err != nil {
				fmt.Fprintln(w, "E", hx(err.Error()))
			} else {
				parts := []string{}
				for _, a := range as {
					parts = append(parts, addr(a))
				}
				if len(parts) == 0 {
					parts = append(parts, "~")
				}
				fmt.Fprintln(w, "OK", len(as), strings.Join(parts, ","))
			}
		case "d":
			t, err := mail.ParseDate(unhex(f[1]))
			if err != nil {
				fmt.Fprintln(w, "E", hx(err.Error()))
			} else {
				name, off := t.Zone()
				fmt.Fprintln(w, "OK", t.Unix(), t.Nanosecond(), hx(name), off)
			}
		case "s":
			a := mail.Address{Name: unhex(f[1]), Address: unhex(f[2])}
			fmt.Fprintln(w, hx(a.String()))
		}
	}
}
