// The reference side of the gates that check Grasm packages against Go
// modules already in the module cache (built offline: GOFLAGS=-mod=mod
// GOPROXY=off GOSUMDB=off). It reads a JSON list of requests on stdin and
// writes a JSON list of answers.
package main

import (
	"archive/zip"
	"bufio"
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/url"
	"io"
	"os"
	"sort"
	"strings"

	"github.com/cespare/xxhash/v2"
	rendezvous "github.com/dgryski/go-rendezvous"
	"github.com/paulmach/orb"
	"github.com/paulmach/orb/maptile"
	proxyproto "github.com/pires/go-proxyproto"
	"golang.org/x/mod/sumdb/dirhash"
	"golang.org/x/net/bpf"
	"golang.org/x/net/http/httpproxy"
	"golang.org/x/tools/txtar"
)

type request struct {
	Op    string    `json:"op"`
	Nodes []string  `json:"nodes"`
	Keys  []string  `json:"keys"`
	Hex   string    `json:"hex"`
	Args  []string  `json:"args"`
	Nums  []float64 `json:"nums"`
	Hexes []string  `json:"hexes"`
}

// score is go-rendezvous's xorshiftMult64 over the two hashes, which the
// package does not export; Lookup below is the package's own.
func score(k, n uint64) uint64 {
	x := k ^ n
	x ^= x >> 12
	x ^= x << 25
	x ^= x >> 27
	return x * 2685821657736338717
}

func answer(r request) string {
	switch r.Op {
	case "rdv":
		ring := rendezvous.New(r.Nodes, xxhash.Sum64String)
		var out []string
		for _, k := range r.Keys {
			order := make([]int, len(r.Nodes))
			for i := range order {
				order[i] = i
			}
			kh := xxhash.Sum64String(k)
			sort.SliceStable(order, func(a, b int) bool {
				return score(kh, xxhash.Sum64String(r.Nodes[order[a]])) > score(kh, xxhash.Sum64String(r.Nodes[order[b]]))
			})
			names := make([]string, len(order))
			for i, o := range order {
				names[i] = r.Nodes[o]
			}
			out = append(out, ring.Lookup(k)+"/"+strings.Join(names, ","))
		}
		return strings.Join(out, " ")
	case "txtar":
		data, _ := hex.DecodeString(r.Hex)
		a := txtar.Parse(data)
		var parts []string
		parts = append(parts, hex.EncodeToString(a.Comment))
		for _, f := range a.Files {
			parts = append(parts, hex.EncodeToString([]byte(f.Name))+":"+hex.EncodeToString(f.Data))
		}
		return strings.Join(parts, "|") + "/" + hex.EncodeToString(txtar.Format(a))
	case "proxy":
		cfg := httpproxy.Config{HTTPProxy: r.Args[0], HTTPSProxy: r.Args[1], NoProxy: r.Args[2], CGI: r.Args[3] == "1"}
		f := cfg.ProxyFunc()
		var out []string
		for _, k := range r.Keys {
			u, err := url.Parse(k)
			if err != nil {
				out = append(out, "BADURL")
				continue
			}
			p, err := f(u)
			if err != nil {
				out = append(out, "ERR")
			} else if p == nil {
				out = append(out, "-")
			} else {
				out = append(out, p.String())
			}
		}
		return strings.Join(out, " ")
	case "bpf":
		return bpfAnswer(r)
	case "pp":
		return proxyAnswer(r.Hex)
	case "tile":
		return tileAnswer(r)
	case "h1":
		files := map[string][]byte{}
		var names []string
		for i := 0; i+1 < len(r.Hexes); i += 2 {
			n, _ := hex.DecodeString(r.Hexes[i])
			d, _ := hex.DecodeString(r.Hexes[i+1])
			names = append(names, string(n))
			files[string(n)] = d
		}
		h, err := dirhash.Hash1(names, func(n string) (io.ReadCloser, error) {
			return io.NopCloser(bytes.NewReader(files[n])), nil
		})
		if err != nil {
			return "ERR"
		}
		return h
	case "h1zip":
		data, _ := hex.DecodeString(r.Hex)
		f, err := os.CreateTemp("", "h1zip*.zip")
		if err != nil {
			return "ERR"
		}
		defer os.Remove(f.Name())
		f.Write(data)
		f.Close()
		if _, err := zip.OpenReader(f.Name()); err != nil {
			return "ERR"
		}
		h, err := dirhash.HashZip(f.Name(), dirhash.Hash1)
		if err != nil {
			return "ERR"
		}
		return h
	}
	return "UNKNOWN"
}

func bpfAnswer(r request) string {
	prog, _ := hex.DecodeString(r.Hex)
	var insns []bpf.Instruction
	var texts []string
	for i := 0; i+8 <= len(prog); i += 8 {
		raw := bpf.RawInstruction{Op: uint16(prog[i]) | uint16(prog[i+1])<<8, Jt: prog[i+2], Jf: prog[i+3],
			K: uint32(prog[i+4]) | uint32(prog[i+5])<<8 | uint32(prog[i+6])<<16 | uint32(prog[i+7])<<24}
		ins := raw.Disassemble()
		insns = append(insns, ins)
		text := "?"
		if _, isRaw := ins.(bpf.RawInstruction); !isRaw {
			text = fmt.Sprint(ins)
			if strings.HasPrefix(text, "unknown") {
				text = "?"
			}
		}
		texts = append(texts, text)
	}
	vm, err := bpf.NewVM(insns)
	var runs []string
	for _, h := range r.Hexes {
		if err != nil {
			runs = append(runs, "INVALID")
			continue
		}
		pkt, _ := hex.DecodeString(h)
		n, rerr := vm.Run(pkt)
		if rerr != nil {
			runs = append(runs, "ERR")
		} else {
			runs = append(runs, fmt.Sprint(n))
		}
	}
	return strings.Join(texts, ";") + "/" + strings.Join(runs, ",")
}

func proxyAnswer(h string) string {
	data, _ := hex.DecodeString(h)
	rd := bufio.NewReader(bytes.NewReader(data))
	hd, err := proxyproto.Read(rd)
	if err != nil {
		return "ERR"
	}
	used := len(data) - rd.Buffered()
	src, dst, sp, dp, sn, dn := "", "", 0, 0, "", ""
	if s, d, ok := hd.IPs(); ok {
		if hd.TransportProtocol.IsIPv4() {
			src, dst = hex.EncodeToString(s.To4()), hex.EncodeToString(d.To4())
		} else {
			src, dst = hex.EncodeToString(s.To16()), hex.EncodeToString(d.To16())
		}
	}
	if a, b, ok := hd.Ports(); ok {
		sp, dp = a, b
	}
	if a, b, ok := hd.UnixAddrs(); ok {
		sn, dn = a.Name, b.Name
	}
	tl := "TLVERR"
	if tlvs, terr := hd.TLVs(); terr == nil {
		var parts []string
		for _, t := range tlvs {
			parts = append(parts, fmt.Sprintf("%d:%s", t.Type, hex.EncodeToString(t.Value)))
		}
		tl = strings.Join(parts, ",")
	}
	formatted := "FMTERR"
	if f, ferr := hd.Format(); ferr == nil {
		formatted = hex.EncodeToString(f)
	}
	return fmt.Sprintf("v%d c%d t%d %s %s %d %d %s %s %s %d %s", hd.Version, byte(hd.Command), byte(hd.TransportProtocol),
		src, dst, sp, dp, hex.EncodeToString([]byte(sn)), hex.EncodeToString([]byte(dn)), tl, used, formatted)
}

func tileAnswer(r request) string {
	var out []string
	for i := 0; i+2 < len(r.Nums); i += 3 {
		lon, lat, z := r.Nums[i], r.Nums[i+1], maptile.Zoom(r.Nums[i+2])
		t := maptile.At(orb.Point{lon, lat}, z)
		b := t.Bound()
		c := t.Center()
		p := t.Parent()
		lo, hi := t.Range(z + 2)
		up, _ := t.Range(z / 2)
		other := maptile.At(orb.Point{-lon / 3, lat / 2}, z)
		sp := t.SharedParent(other)
		qk := t.Quadkey()
		back := maptile.FromQuadkey(qk, z)
		var digits []byte
		for l := int(z); l > 0; l-- {
			digits = append(digits, byte('0'+((t.X>>(l-1))&1)+2*((t.Y>>(l-1))&1)))
		}
		var kids []string
		for _, k := range t.Children() {
			kids = append(kids, fmt.Sprintf("%d.%d.%d", k.X, k.Y, k.Z))
		}
		out = append(out, fmt.Sprintf("%d.%d.%d %.9f,%.9f,%.9f,%.9f %.9f,%.9f %d.%d.%d %s %d.%d.%d-%d.%d.%d %d.%d.%d %d.%d.%d %d %d.%d.%d q%s %v",
			t.X, t.Y, t.Z, b.Min[0], b.Min[1], b.Max[0], b.Max[1], c[0], c[1], p.X, p.Y, p.Z, strings.Join(kids, ","),
			lo.X, lo.Y, lo.Z, hi.X, hi.Y, hi.Z, up.X, up.Y, up.Z, sp.X, sp.Y, sp.Z, qk, back.X, back.Y, back.Z, string(digits),
			t.Contains(other)))
	}
	return strings.Join(out, " ")
}

func main() {
	var reqs []request
	if err := json.NewDecoder(os.Stdin).Decode(&reqs); err != nil {
		panic(err)
	}
	out := make([]string, len(reqs))
	for i, r := range reqs {
		out[i] = answer(r)
	}
	json.NewEncoder(os.Stdout).Encode(out)
}
