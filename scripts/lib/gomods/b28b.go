package main

// The second half of the oracle: HTML charset sniffing (x/net), SVCB and
// DNSSEC (miekg/dns), and WKB (orb).

import (
	"crypto"
	"crypto/ecdsa"
	"crypto/ed25519"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/rsa"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"regexp"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/miekg/dns"
	"github.com/paulmach/orb"
	"github.com/paulmach/orb/encoding/ewkb"
	"github.com/paulmach/orb/encoding/wkb"
	"golang.org/x/net/html/charset"
	"golang.org/x/text/encoding/htmlindex"
)

func answer2(r request) string {
	switch r.Op {
	case "charset":
		data, _ := hex.DecodeString(r.Hex)
		_, name, certain := charset.DetermineEncoding(data, r.Args[0])
		return fmt.Sprintf("%s|%v", name, certain)
	case "labels":
		return labels(r.Args[0])
	case "label":
		var out []string
		for _, l := range r.Args {
			e, err := htmlindex.Get(l)
			if err != nil {
				out = append(out, "ERR")
				continue
			}
			n, _ := htmlindex.Name(e)
			out = append(out, n)
		}
		return strings.Join(out, ",")
	case "svcb.text":
		rr, err := dns.NewRR(r.Args[0])
		if err != nil || rr == nil {
			return "ERR"
		}
		return rdataOf(rr) + "|" + textOf(rr)
	case "svcb.wire":
		rdata, _ := hex.DecodeString(r.Hex)
		msg := []byte{0, 0, 65, 0, 1, 0, 0, 14, 16, byte(len(rdata) >> 8), byte(len(rdata))}
		msg = append(msg, rdata...)
		rr, _, err := dns.UnpackRR(msg, 0)
		if err != nil {
			return "ERR"
		}
		return rdataOf(rr) + "|" + textOf(rr)
	case "keytag":
		k := dnskey(r)
		return fmt.Sprint(k.KeyTag())
	case "ds":
		k := dnskey(r)
		var out []string
		for _, h := range []uint8{1, 2, 4} {
			ds := k.ToDS(h)
			if ds == nil {
				out = append(out, "ERR")
			} else {
				out = append(out, fmt.Sprintf("%d:%s", ds.KeyTag, strings.ToLower(ds.Digest)))
			}
		}
		return strings.Join(out, " ")
	case "nsec3":
		iter, _ := strconv.Atoi(r.Args[2])
		return dns.HashName(r.Args[0], dns.SHA1, uint16(iter), r.Args[1])
	case "bitmap":
		var types []uint16
		for _, n := range r.Nums {
			types = append(types, uint16(n))
		}
		rr := &dns.NSEC{Hdr: dns.RR_Header{Name: "a.", Rrtype: dns.TypeNSEC, Class: 1}, NextDomain: ".", TypeBitMap: types}
		return rdataOf(rr)
	case "sign":
		return signCase(r)
	case "validity":
		incep, _ := strconv.ParseUint(r.Args[0], 10, 32)
		expir, _ := strconv.ParseUint(r.Args[1], 10, 32)
		now, _ := strconv.ParseInt(r.Args[2], 10, 64)
		sig := &dns.RRSIG{Inception: uint32(incep), Expiration: uint32(expir)}
		return fmt.Sprint(sig.ValidityPeriod(time.Unix(now, 0)))
	case "verify":
		return verifyCase(r)
	case "wkb.enc":
		g, err := geometry(r.Args[0])
		if err != nil {
			return "ERR"
		}
		b, err := wkb.Marshal(g, byteOrder(r.Args[1]))
		if err != nil {
			return "ERR"
		}
		return hex.EncodeToString(b)
	case "ewkb.enc":
		g, err := geometry(r.Args[0])
		if err != nil {
			return "ERR"
		}
		srid, _ := strconv.Atoi(r.Args[2])
		b, err := ewkb.Marshal(g, srid, byteOrder(r.Args[1]))
		if err != nil {
			return "ERR"
		}
		return hex.EncodeToString(b)
	case "wkb.dec":
		b, _ := hex.DecodeString(r.Hex)
		g, err := wkb.Unmarshal(b)
		if err != nil {
			return "ERR"
		}
		return canonical(g)
	case "ewkb.dec":
		b, _ := hex.DecodeString(r.Hex)
		g, srid, err := ewkb.Unmarshal(b)
		if err != nil {
			return "ERR"
		}
		return fmt.Sprintf("%d|%s", srid, canonical(g))
	}
	return "UNKNOWN"
}

// labels lists every label htmlindex knows with its canonical name, from
// the table x/text generates from the WHATWG Encoding Standard.
func labels(path string) string {
	src, err := os.ReadFile(path)
	if err != nil {
		return "ERR"
	}
	re := regexp.MustCompile(`"([^"]+)":\s+[a-zA-Z0-9_]+,`)
	var out []string
	for _, m := range re.FindAllStringSubmatch(string(src), -1) {
		e, err := htmlindex.Get(m[1])
		if err != nil {
			continue
		}
		n, _ := htmlindex.Name(e)
		out = append(out, m[1]+"="+n)
	}
	sort.Strings(out)
	return strings.Join(out, " ")
}

func byteOrder(name string) binary.ByteOrder {
	if name == "big" {
		return binary.BigEndian
	}
	return binary.LittleEndian
}

func rdataOf(rr dns.RR) string {
	buf := make([]byte, 65536)
	off, err := dns.PackRR(rr, buf, 0, nil, false)
	if err != nil {
		return "ERR"
	}
	start := nameLen(buf) + 10
	return hex.EncodeToString(buf[start:off])
}

func nameLen(buf []byte) int {
	i := 0
	for buf[i] != 0 {
		i += int(buf[i]) + 1
	}
	return i + 1
}

// textOf is the presentation form after the type.
func textOf(rr dns.RR) string {
	s := rr.String()
	parts := strings.SplitN(s, "\t", 5)
	if len(parts) < 5 {
		return ""
	}
	return parts[4]
}

func dnskey(r request) *dns.DNSKEY {
	flags, _ := strconv.Atoi(r.Args[0])
	proto, _ := strconv.Atoi(r.Args[1])
	alg, _ := strconv.Atoi(r.Args[2])
	key, _ := hex.DecodeString(r.Hex)
	owner := "example.com."
	if len(r.Args) > 3 {
		owner = r.Args[3]
	}
	return &dns.DNSKEY{Hdr: dns.RR_Header{Name: owner, Rrtype: dns.TypeDNSKEY, Class: 1, Ttl: 3600},
		Flags: uint16(flags), Protocol: uint8(proto), Algorithm: uint8(alg), PublicKey: base64.StdEncoding.EncodeToString(key)}
}

// signCase makes a key for the algorithm, signs the RRset given in
// presentation form, and returns what a verifier needs: the DNSKEY's
// public key in hex, the key tag, and the signature in hex.
func signCase(r request) string {
	alg, _ := strconv.Atoi(r.Args[0])
	var signer crypto.Signer
	var err error
	switch alg {
	case 8, 10:
		signer, err = rsa.GenerateKey(rand.Reader, 2048)
	case 13:
		signer, err = ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	case 14:
		signer, err = ecdsa.GenerateKey(elliptic.P384(), rand.Reader)
	case 15:
		_, signer, err = ed25519.GenerateKey(rand.Reader)
	}
	if err != nil {
		return "ERR"
	}
	key := &dns.DNSKEY{Hdr: dns.RR_Header{Name: r.Args[1], Rrtype: dns.TypeDNSKEY, Class: 1, Ttl: 3600},
		Flags: 257, Protocol: 3, Algorithm: uint8(alg)}
	switch k := signer.(type) {
	case *rsa.PrivateKey:
		key.PublicKey = rsaPublic(&k.PublicKey)
	case *ecdsa.PrivateKey:
		key.PublicKey = ecdsaPublic(&k.PublicKey, alg)
	case ed25519.PrivateKey:
		key.PublicKey = base64.StdEncoding.EncodeToString(k.Public().(ed25519.PublicKey))
	}
	var rrset []dns.RR
	for _, line := range r.Args[5:] {
		rr, err := dns.NewRR(line)
		if err != nil {
			return "ERR " + err.Error()
		}
		rrset = append(rrset, rr)
	}
	incep, _ := strconv.ParseUint(r.Args[2], 10, 32)
	expir, _ := strconv.ParseUint(r.Args[3], 10, 32)
	ttl, _ := strconv.ParseUint(r.Args[4], 10, 32)
	sig := &dns.RRSIG{Hdr: dns.RR_Header{Name: rrset[0].Header().Name, Rrtype: dns.TypeRRSIG, Class: 1, Ttl: uint32(ttl)},
		Inception: uint32(incep), Expiration: uint32(expir), KeyTag: key.KeyTag(), SignerName: r.Args[1], Algorithm: uint8(alg)}
	if err := sig.Sign(signer, rrset); err != nil {
		return "ERR " + err.Error()
	}
	pub, _ := base64.StdEncoding.DecodeString(key.PublicKey)
	raw, _ := base64.StdEncoding.DecodeString(sig.Signature)
	return fmt.Sprintf("%s|%d|%d|%d|%s", hex.EncodeToString(pub), key.KeyTag(), sig.Labels, sig.OrigTtl, hex.EncodeToString(raw))
}

// verifyCase checks a signature made elsewhere: Args are algorithm,
// signer, inception, expiration, labels, original TTL, key tag, then the
// RRset; Hex is public key then signature, separated by "|".
func verifyCase(r request) string {
	alg, _ := strconv.Atoi(r.Args[0])
	parts := strings.Split(r.Hex, "|")
	pub, _ := hex.DecodeString(parts[0])
	raw, _ := hex.DecodeString(parts[1])
	key := &dns.DNSKEY{Hdr: dns.RR_Header{Name: r.Args[1], Rrtype: dns.TypeDNSKEY, Class: 1, Ttl: 3600},
		Flags: 257, Protocol: 3, Algorithm: uint8(alg), PublicKey: base64.StdEncoding.EncodeToString(pub)}
	var rrset []dns.RR
	for _, line := range r.Args[7:] {
		rr, err := dns.NewRR(line)
		if err != nil {
			return "ERR"
		}
		rrset = append(rrset, rr)
	}
	num := func(i int) uint32 { v, _ := strconv.ParseUint(r.Args[i], 10, 32); return uint32(v) }
	sig := &dns.RRSIG{Hdr: dns.RR_Header{Name: rrset[0].Header().Name, Rrtype: dns.TypeRRSIG, Class: 1, Ttl: num(5)},
		TypeCovered: rrset[0].Header().Rrtype, Algorithm: uint8(alg), Labels: uint8(num(4)), OrigTtl: num(5),
		Expiration: num(3), Inception: num(2), KeyTag: uint16(num(6)), SignerName: r.Args[1],
		Signature: base64.StdEncoding.EncodeToString(raw)}
	if err := sig.Verify(key, rrset); err != nil {
		return "false"
	}
	return "true"
}

func rsaPublic(k *rsa.PublicKey) string {
	e := big(k.E)
	var out []byte
	if len(e) < 256 {
		out = append([]byte{byte(len(e))}, e...)
	} else {
		out = append([]byte{0, byte(len(e) >> 8), byte(len(e))}, e...)
	}
	return base64.StdEncoding.EncodeToString(append(out, k.N.Bytes()...))
}

func big(e int) []byte {
	var b []byte
	for e > 0 {
		b = append([]byte{byte(e)}, b...)
		e >>= 8
	}
	return b
}

func ecdsaPublic(k *ecdsa.PublicKey, alg int) string {
	size := 32
	if alg == 14 {
		size = 48
	}
	out := make([]byte, 2*size)
	k.X.FillBytes(out[:size])
	k.Y.FillBytes(out[size:])
	return base64.StdEncoding.EncodeToString(out)
}

// --- WKB -----------------------------------------------------------------

type geom struct {
	T string            `json:"t"`
	C json.RawMessage   `json:"c"`
	G []json.RawMessage `json:"g"`
}

func geometry(text string) (orb.Geometry, error) {
	var g geom
	if err := json.Unmarshal([]byte(text), &g); err != nil {
		return nil, err
	}
	switch g.T {
	case "Point":
		var c [2]float64
		err := json.Unmarshal(g.C, &c)
		return orb.Point(c), err
	case "LineString":
		var c []orb.Point
		err := json.Unmarshal(g.C, &c)
		return orb.LineString(c), err
	case "Polygon":
		var c [][]orb.Point
		err := json.Unmarshal(g.C, &c)
		p := orb.Polygon{}
		for _, r := range c {
			p = append(p, orb.Ring(r))
		}
		return p, err
	case "MultiPoint":
		var c []orb.Point
		err := json.Unmarshal(g.C, &c)
		return orb.MultiPoint(c), err
	case "MultiLineString":
		var c [][]orb.Point
		err := json.Unmarshal(g.C, &c)
		m := orb.MultiLineString{}
		for _, l := range c {
			m = append(m, orb.LineString(l))
		}
		return m, err
	case "MultiPolygon":
		var c [][][]orb.Point
		err := json.Unmarshal(g.C, &c)
		m := orb.MultiPolygon{}
		for _, p := range c {
			poly := orb.Polygon{}
			for _, r := range p {
				poly = append(poly, orb.Ring(r))
			}
			m = append(m, poly)
		}
		return m, err
	case "GeometryCollection":
		c := orb.Collection{}
		for _, raw := range g.G {
			sub, err := geometry(string(raw))
			if err != nil {
				return nil, err
			}
			c = append(c, sub)
		}
		return c, nil
	}
	return nil, fmt.Errorf("unknown %s", g.T)
}

func bitsOf(p orb.Point) string {
	return fmt.Sprintf("%d,%d", int64(math.Float64bits(p[0])), int64(math.Float64bits(p[1])))
}

func points(ps []orb.Point) string {
	var out []string
	for _, p := range ps {
		out = append(out, bitsOf(p))
	}
	return strings.Join(out, ";")
}

// canonical writes a geometry the way the Grasm harness does: coordinates
// as the bits of their doubles.
func canonical(g orb.Geometry) string {
	switch v := g.(type) {
	case orb.Point:
		return "Point(" + bitsOf(v) + ")"
	case orb.LineString:
		return "LineString(" + points(v) + ")"
	case orb.Ring:
		return "LineString(" + points(v) + ")"
	case orb.Polygon:
		var rings []string
		for _, r := range v {
			rings = append(rings, "("+points(r)+")")
		}
		return "Polygon(" + strings.Join(rings, "") + ")"
	case orb.MultiPoint:
		return "MultiPoint(" + points(v) + ")"
	case orb.MultiLineString:
		var ls []string
		for _, l := range v {
			ls = append(ls, "("+points(l)+")")
		}
		return "MultiLineString(" + strings.Join(ls, "") + ")"
	case orb.MultiPolygon:
		var ps []string
		for _, p := range v {
			ps = append(ps, canonical(p)[len("Polygon"):])
		}
		return "MultiPolygon(" + strings.Join(ps, "") + ")"
	case orb.Collection:
		var gs []string
		for _, s := range v {
			gs = append(gs, canonical(s))
		}
		return "GeometryCollection(" + strings.Join(gs, ",") + ")"
	}
	return "?"
}
