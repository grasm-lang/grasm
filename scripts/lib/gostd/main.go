// The reference side of scripts/gostd_diff.py: each stdin line is
// "op hexA hexB" (each hex field prefixed with "x" so it is never empty),
// and each stdout line is what Go's standard library answers, in the same
// form the Grasm harness packages/gostdtest prints.
package main

import (
	"bufio"
	"encoding/json"
	"math"
	"bytes"
	"crypto/cipher"
	"crypto/des"
	"crypto/rc4"
	"encoding/hex"
	"errors"
	"fmt"
	"hash"
	"hash/fnv"
	"index/suffixarray"
	"io"
	"net/textproto"
	"os"
	"sort"
	"strconv"
	"strings"
	"text/tabwriter"
	"time"
)

func unhex(s string) []byte {
	b, err := hex.DecodeString(s[1:])
	if err != nil {
		panic(err)
	}
	return b
}

func h(b []byte) string { return hex.EncodeToString(b) }

func sum(f func() hash.Hash, data []byte) string {
	x := f()
	x.Write(data)
	return h(x.Sum(nil))
}

func desCipher(key []byte) (cipher.Block, error) {
	switch len(key) {
	case 8:
		return des.NewCipher(key)
	case 24:
		return des.NewTripleDESCipher(key)
	}
	return nil, errors.New("key")
}

func ints(xs []int) string {
	parts := make([]string, len(xs))
	for i, x := range xs {
		parts[i] = strconv.Itoa(x)
	}
	return strings.Join(parts, ",")
}

func readerAt(data []byte) (*textproto.Reader, func() int) {
	under := bytes.NewReader(data)
	br := bufio.NewReaderSize(under, 1<<20)
	return textproto.NewReader(br), func() int { return len(data) - br.Buffered() - under.Len() }
}

func tpErr(err error) string {
	var pe textproto.ProtocolError
	var te *textproto.Error
	switch {
	case errors.As(err, &te):
		return fmt.Sprintf("ERR:unexpected:%d:%s", te.Code, h([]byte(te.Msg)))
	case errors.As(err, &pe):
		return "ERR:malformed"
	case errors.Is(err, io.EOF), errors.Is(err, io.ErrUnexpectedEOF):
		return "ERR:trunc"
	}
	return "ERR:other:" + err.Error()
}

// goValue decodes the driver's tagged JSON into the Go value it names, so
// both sides format exactly the same operands.
func goValue(raw any) any {
	t := raw.([]any)
	switch t[0].(string) {
	case "n":
		return nil
	case "t":
		return t[1].(bool)
	case "i":
		v, _ := strconv.ParseInt(t[1].(string), 10, 64)
		return int(v)
	case "u":
		v, _ := strconv.ParseUint(t[1].(string), 10, 64)
		return uint(v)
	case "f":
		v, _ := strconv.ParseUint(t[1].(string), 16, 64)
		return math.Float64frombits(v)
	case "s":
		return t[1].(string)
	case "b":
		v, _ := hex.DecodeString(t[1].(string))
		return v
	case "l":
		out := []any{}
		for _, item := range t[1].([]any) {
			out = append(out, goValue(item))
		}
		return out
	case "m":
		out := map[string]any{}
		for _, pair := range t[1].([]any) {
			kv := pair.([]any)
			out[kv[0].(string)] = goValue(kv[1])
		}
		return out
	}
	panic("unknown tag " + t[0].(string))
}

func goArgs(b []byte) []any {
	var raw []any
	if err := json.Unmarshal(b, &raw); err != nil {
		panic(err)
	}
	out := []any{}
	for _, r := range raw {
		out = append(out, goValue(r))
	}
	return out
}

func answer(op string, a, b []byte) string {
	switch op {
	case "fnv32":
		return sum(func() hash.Hash { return fnv.New32() }, a)
	case "fnv32a":
		return sum(func() hash.Hash { return fnv.New32a() }, a)
	case "fnv64":
		return sum(func() hash.Hash { return fnv.New64() }, a)
	case "fnv64a":
		return sum(func() hash.Hash { return fnv.New64a() }, a)
	case "fnv128":
		return sum(func() hash.Hash { return fnv.New128() }, a)
	case "fnv128a":
		return sum(func() hash.Hash { return fnv.New128a() }, a)
	case "rc4":
		c, err := rc4.NewCipher(a)
		if err != nil {
			return "ERR"
		}
		out := make([]byte, len(b))
		c.XORKeyStream(out, b)
		return h(out)
	case "desecb", "desecbd", "descbc", "descbcd":
		c, err := desCipher(a)
		if err != nil {
			return "ERR"
		}
		data := b
		var iv []byte
		if op == "descbc" || op == "descbcd" {
			if len(b) < 8 {
				return "ERR"
			}
			iv, data = b[:8], b[8:]
		}
		if len(data)%8 != 0 {
			return "ERR"
		}
		out := make([]byte, len(data))
		switch op {
		case "desecb":
			for i := 0; i < len(data); i += 8 {
				c.Encrypt(out[i:], data[i:])
			}
		case "desecbd":
			for i := 0; i < len(data); i += 8 {
				c.Decrypt(out[i:], data[i:])
			}
		case "descbc":
			cipher.NewCBCEncrypter(c, iv).CryptBlocks(out, data)
		case "descbcd":
			cipher.NewCBCDecrypter(c, iv).CryptBlocks(out, data)
		}
		return h(out)
	case "gq":
		return h([]byte(strconv.Quote(string(a))))
	case "gqa":
		return h([]byte(strconv.QuoteToASCII(string(a))))
	case "gqg":
		return h([]byte(strconv.QuoteToGraphic(string(a))))
	case "gqr", "gqra", "gqrg":
		r, _ := strconv.Atoi(string(a))
		switch op {
		case "gqr":
			return h([]byte(strconv.QuoteRune(rune(r))))
		case "gqra":
			return h([]byte(strconv.QuoteRuneToASCII(rune(r))))
		}
		return h([]byte(strconv.QuoteRuneToGraphic(rune(r))))
	case "guq":
		s, err := strconv.Unquote(string(a))
		if err != nil {
			return "ERR"
		}
		return h([]byte(s))
	case "gqp":
		s, err := strconv.QuotedPrefix(string(a))
		if err != nil {
			return "ERR"
		}
		return h([]byte(s))
	case "gcb":
		if strconv.CanBackquote(string(a)) {
			return "yes"
		}
		return "no"
	case "gpr":
		r, _ := strconv.Atoi(string(a))
		p, g := strconv.IsPrint(rune(r)), strconv.IsGraphic(rune(r))
		return fmt.Sprintf("%t,%t", p, g)
	case "gtf":
		f := strings.Split(string(b), ",")
		unix, _ := strconv.ParseInt(f[0], 10, 64)
		nanos, _ := strconv.ParseInt(f[1], 10, 64)
		off, _ := strconv.Atoi(f[2])
		t := time.Unix(unix, nanos).In(time.FixedZone(f[3], off))
		return h([]byte(t.Format(string(a))))
	case "gtp":
		t, err := time.Parse(string(a), string(b))
		if err != nil {
			return "ERR"
		}
		name, off := t.Zone()
		return h([]byte(fmt.Sprintf("%d,%d,%d,%s", t.Unix(), t.Nanosecond(), off, name)))
	case "tw":
		f := strings.Split(string(b), ",")
		n := make([]int, 5)
		for i := range n {
			n[i], _ = strconv.Atoi(f[i])
		}
		var out bytes.Buffer
		w := tabwriter.NewWriter(&out, n[0], n[1], n[2], byte(n[3]), uint(n[4]))
		w.Write(a)
		w.Flush()
		return h(out.Bytes())
	case "tpkey":
		return h([]byte(textproto.CanonicalMIMEHeaderKey(string(a))))
	case "tptrim":
		return h([]byte(textproto.TrimString(string(a))))
	case "tphdr":
		r, used := readerAt(a)
		m, err := r.ReadMIMEHeader()
		if err != nil {
			return tpErr(err)
		}
		keys := make([]string, 0, len(m))
		for k := range m {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		var parts []string
		for _, k := range keys {
			parts = append(parts, k+"="+strings.Join(m[k], "\x01"))
		}
		return h([]byte(strings.Join(parts, "\x02"))) + "|" + strconv.Itoa(used())
	case "tpcont":
		r, used := readerAt(a)
		s, err := r.ReadContinuedLine()
		if err != nil {
			return tpErr(err)
		}
		return h([]byte(s)) + "|" + strconv.Itoa(used())
	case "tpresp", "tpcode":
		expect, _ := strconv.Atoi(string(b))
		r, used := readerAt(a)
		var code int
		var msg string
		var err error
		if op == "tpresp" {
			code, msg, err = r.ReadResponse(expect)
		} else {
			code, msg, err = r.ReadCodeLine(expect)
		}
		if err != nil {
			return tpErr(err)
		}
		return fmt.Sprintf("%d|%s|%d", code, h([]byte(msg)), used())
	case "tpdotd":
		r, used := readerAt(a)
		got, err := r.ReadDotBytes()
		if err != nil {
			return tpErr(err)
		}
		return h(got) + "|" + strconv.Itoa(used())
	case "tpdote":
		var out bytes.Buffer
		bw := bufio.NewWriter(&out)
		w := textproto.NewWriter(bw).DotWriter()
		w.Write(a)
		w.Close()
		return h(out.Bytes())
	case "gpf":
		return h([]byte(fmt.Sprintf(string(a), goArgs(b)...)))
	case "gps":
		return h([]byte(fmt.Sprint(goArgs(b)...)))
	case "gpl":
		return h([]byte(fmt.Sprintln(goArgs(b)...)))
	case "gff":
		f := strings.Split(string(a), ",")
		bits, _ := strconv.ParseUint(f[0], 16, 64)
		prec, _ := strconv.Atoi(f[2])
		return h([]byte(strconv.FormatFloat(math.Float64frombits(bits), f[1][0], prec, 64)))
	case "salook":
		idx := suffixarray.New(a)
		got := idx.Lookup(b, -1)
		sort.Ints(got)
		return ints(got)
	}
	return "UNKNOWN " + op
}

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<26)
	out := bufio.NewWriter(os.Stdout)
	defer out.Flush()
	for in.Scan() {
		f := strings.Split(in.Text(), " ")
		fmt.Fprintln(out, answer(f[0], unhex(f[1]), unhex(f[2])))
	}
}
