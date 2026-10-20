// The oracle for packages/gourl: Go's own net/url.
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"net/url"
	"os"
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

func desc(u *url.URL) string {
	user, name, pass := "~", "~", "~"
	if u.User != nil {
		user = hx(u.User.String())
		name = hx(u.User.Username())
		p, set := u.User.Password()
		pass = hx(p) + fmt.Sprint(set)
	}
	f := []string{hx(u.Scheme), hx(u.Opaque), user, name, pass, hx(u.Host), hx(u.Path), hx(u.RawPath), fmt.Sprint(u.OmitHost, u.ForceQuery),
		hx(u.RawQuery), hx(u.Fragment), hx(u.RawFragment), hx(u.String()), hx(u.Redacted()), hx(u.EscapedPath()), hx(u.EscapedFragment()),
		hx(u.Hostname()), hx(u.Port()), hx(u.RequestURI()), fmt.Sprint(u.IsAbs()), hx(u.Query().Encode())}
	return strings.Join(f, ",")
}

func run(f []string) string {
	switch f[0] {
	case "p", "r":
		parse := url.Parse
		if f[0] == "r" {
			parse = url.ParseRequestURI
		}
		u, err := parse(unhex(f[1]))
		if err != nil {
			return "E " + hx(err.Error())
		}
		return desc(u)
	case "ref":
		b, err := url.Parse(unhex(f[1]))
		if err != nil {
			return "E " + hx(err.Error())
		}
		u, err := b.Parse(unhex(f[2]))
		if err != nil {
			return "E " + hx(err.Error())
		}
		return desc(u)
	case "jp":
		elems := []string{}
		for _, e := range f[2:] {
			elems = append(elems, unhex(e))
		}
		s, err := url.JoinPath(unhex(f[1]), elems...)
		if err != nil {
			return "E " + hx(err.Error())
		}
		return hx(s)
	case "qe":
		return hx(url.QueryEscape(unhex(f[1])))
	case "pe":
		return hx(url.PathEscape(unhex(f[1])))
	case "qu", "pu":
		un := url.QueryUnescape
		if f[0] == "pu" {
			un = url.PathUnescape
		}
		s, err := un(unhex(f[1]))
		if err != nil {
			return "E " + hx(err.Error())
		}
		return hx(s)
	default:
		v, err := url.ParseQuery(unhex(f[1]))
		e := "-"
		if err != nil {
			e = hx(err.Error())
		}
		return hx(v.Encode()) + " " + e
	}
}

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<20)
	out := bufio.NewWriter(os.Stdout)
	for in.Scan() {
		if in.Text() != "" {
			fmt.Fprintln(out, run(strings.Split(in.Text(), " ")))
		}
	}
	out.Flush()
}
