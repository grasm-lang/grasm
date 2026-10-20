// The oracle for packages/gonetip: Go's own net/netip.
package main

import (
	"bufio"
	"encoding/hex"
	"fmt"
	"net/netip"
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

func desc(a netip.Addr) string {
	f := []string{a.String(), a.StringExpanded(), a.Zone(), strconv.Itoa(a.BitLen()), fmt.Sprint(a.Is4(), a.Is6(), a.Is4In6()), a.Unmap().String(),
		fmt.Sprint(a.IsLoopback(), a.IsMulticast(), a.IsLinkLocalUnicast(), a.IsLinkLocalMulticast(), a.IsInterfaceLocalMulticast(),
			a.IsGlobalUnicast(), a.IsPrivate(), a.IsUnspecified()),
		a.Next().String(), a.Prev().String(), hex.EncodeToString(a.AsSlice())}
	out := []string{}
	for _, s := range f {
		out = append(out, hx(s))
	}
	return strings.Join(out, ",")
}

func errLine(err error) string { return "E " + hx(err.Error()) }

func run(f []string) string {
	switch f[0] {
	case "pa":
		a, err := netip.ParseAddr(unhex(f[1]))
		if err != nil {
			return errLine(err)
		}
		return desc(a)
	case "pp":
		p, err := netip.ParsePrefix(unhex(f[1]))
		if err != nil {
			return errLine(err)
		}
		return hx(p.String()) + " " + hx(p.Masked().String()) + " " + fmt.Sprint(p.IsSingleIP(), p.Bits()) + " " + hx(p.Addr().String())
	case "pap":
		p, err := netip.ParseAddrPort(unhex(f[1]))
		if err != nil {
			return errLine(err)
		}
		return hx(p.String()) + " " + strconv.Itoa(int(p.Port()))
	case "cmp":
		a, _ := netip.ParseAddr(unhex(f[1]))
		b, _ := netip.ParseAddr(unhex(f[2]))
		return fmt.Sprint(a.Compare(b), a.Less(b))
	case "con":
		p, _ := netip.ParsePrefix(unhex(f[1]))
		a, _ := netip.ParseAddr(unhex(f[2]))
		return fmt.Sprint(p.Contains(a))
	case "ov":
		p, _ := netip.ParsePrefix(unhex(f[1]))
		o, _ := netip.ParsePrefix(unhex(f[2]))
		return fmt.Sprint(p.Overlaps(o))
	default:
		a, _ := netip.ParseAddr(unhex(f[1]))
		bits, _ := strconv.Atoi(f[2])
		p, err := a.Prefix(bits)
		if err != nil {
			return errLine(err)
		}
		return hx(p.String())
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
