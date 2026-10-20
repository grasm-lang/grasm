// Oracle for gohttpcookie: Go 1.25's net/http ParseCookie, ParseSetCookie,
// Cookie.String and Cookie.Valid. Run with TZ=UTC; Go's log output (the
// sanitizer's warnings) goes to stderr and is ignored.
// Input lines: c <hex> | s <hex> | w <hex name> <hex value> <quoted> <hex path>
// <hex domain> <expires: - or secs.nanos> <maxage> <secure> <httponly> <samesite> <partitioned>
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"strconv"
	"strings"
	"time"
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

func b(v bool) string {
	if v {
		return "1"
	}
	return "0"
}

func valid(c *http.Cookie) string {
	if err := c.Valid(); err != nil {
		return hx(err.Error())
	}
	return "ok"
}

func expires(t time.Time) string {
	if t.IsZero() {
		return "-"
	}
	return fmt.Sprintf("%d.%d", t.Unix(), t.Nanosecond())
}

func list(xs []string) string {
	if len(xs) == 0 {
		return "~"
	}
	out := make([]string, len(xs))
	for i, x := range xs {
		out[i] = hx(x)
	}
	return strings.Join(out, ",")
}

func main() {
	log.SetOutput(io.Discard)
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<24)
	w := bufio.NewWriter(os.Stdout)
	defer w.Flush()
	for in.Scan() {
		f := strings.Split(in.Text(), " ")
		switch f[0] {
		case "c":
			cs, err := http.ParseCookie(unhex(f[1]))
			if err != nil {
				fmt.Fprintln(w, "E", hx(err.Error()))
				continue
			}
			parts := []string{}
			for _, c := range cs {
				parts = append(parts, hx(c.Name)+":"+hx(c.Value)+":"+b(c.Quoted)+":"+hx(c.String())+":"+valid(c))
			}
			fmt.Fprintln(w, "OK", len(cs), strings.Join(parts, ","))
		case "s":
			c, err := http.ParseSetCookie(unhex(f[1]))
			if err != nil {
				fmt.Fprintln(w, "E", hx(err.Error()))
				continue
			}
			fmt.Fprintln(w, "OK", hx(c.Name), hx(c.Value), b(c.Quoted), hx(c.Path), hx(c.Domain), expires(c.Expires), hx(c.RawExpires), c.MaxAge,
				b(c.Secure), b(c.HttpOnly), int(c.SameSite), b(c.Partitioned), hx(c.Raw), list(c.Unparsed), hx(c.String()), valid(c))
		case "w":
			c := &http.Cookie{Name: unhex(f[1]), Value: unhex(f[2]), Quoted: f[3] == "1", Path: unhex(f[4]), Domain: unhex(f[5])}
			if f[6] != "-" {
				p := strings.Split(f[6], ".")
				s, _ := strconv.ParseInt(p[0], 10, 64)
				n, _ := strconv.ParseInt(p[1], 10, 64)
				c.Expires = time.Unix(s, n).UTC()
			}
			c.MaxAge, _ = strconv.Atoi(f[7])
			c.Secure = f[8] == "1"
			c.HttpOnly = f[9] == "1"
			ss, _ := strconv.Atoi(f[10])
			c.SameSite = http.SameSite(ss)
			c.Partitioned = f[11] == "1"
			fmt.Fprintln(w, hx(c.String()), valid(c))
		}
	}
}
