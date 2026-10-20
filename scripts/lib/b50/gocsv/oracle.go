// The oracle for packages/gocsv: Go's encoding/csv. Each input line is an
// operation and its arguments; each answer is one line.
package main

import (
	"bufio"
	"bytes"
	"encoding/csv"
	"encoding/hex"
	"fmt"
	"io"
	"os"
	"strconv"
	"strings"
)

func unhex(h string) []byte {
	if h == "-" {
		return nil
	}
	b, _ := hex.DecodeString(h)
	return b
}

func hx(b []byte) string {
	if len(b) == 0 {
		return "~"
	}
	return hex.EncodeToString(b)
}

func num(s string) int {
	n, _ := strconv.Atoi(s)
	return n
}

func newReader(f []string) *csv.Reader {
	r := csv.NewReader(bytes.NewReader(unhex(f[1])))
	r.Comma = rune(num(f[2]))
	r.Comment = rune(num(f[3]))
	r.FieldsPerRecord = num(f[4])
	r.LazyQuotes = f[5] == "1"
	r.TrimLeadingSpace = f[6] == "1"
	return r
}

func fields(rec []string) string {
	parts := make([]string, len(rec))
	for i, x := range rec {
		parts[i] = hx([]byte(x))
	}
	return strings.Join(parts, ",")
}

func errText(err error) string {
	if err == nil {
		return "-"
	}
	return hex.EncodeToString([]byte(err.Error()))
}

func reads(f []string) string {
	r := newReader(f)
	var out []string
	for k := 0; k < 300; k++ {
		rec, err := r.Read()
		if err == io.EOF {
			out = append(out, "EOF")
			break
		}
		pos := make([]string, len(rec))
		for i := range rec {
			l, c := r.FieldPos(i)
			pos[i] = fmt.Sprintf("%d:%d", l, c)
		}
		out = append(out, fields(rec)+"/"+strings.Join(pos, ",")+"/"+errText(err))
		if err != nil && strings.HasPrefix(err.Error(), "csv: invalid") {
			break
		}
	}
	return strings.Join(out, ";") + " @" + strconv.FormatInt(r.InputOffset(), 10)
}

func readAll(f []string) string {
	recs, err := newReader(f).ReadAll()
	if err != nil {
		return "E" + errText(err)
	}
	parts := make([]string, len(recs))
	for i, rec := range recs {
		parts[i] = "r" + fields(rec)
	}
	return strings.Join(parts, "/")
}

func write(f []string) string {
	var buf bytes.Buffer
	w := csv.NewWriter(&buf)
	w.Comma = rune(num(f[2]))
	w.UseCRLF = f[3] == "1"
	var recs [][]string
	if f[1] != "-" {
		for _, r := range strings.Split(f[1], "/") {
			rec := []string{}
			if r != "r" {
				for _, x := range strings.Split(r[1:], ",") {
					if x == "~" {
						rec = append(rec, "")
					} else {
						rec = append(rec, string(unhex(x)))
					}
				}
			}
			recs = append(recs, rec)
		}
	}
	if err := w.WriteAll(recs); err != nil {
		return "E" + errText(err)
	}
	return hx(buf.Bytes())
}

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<26)
	out := bufio.NewWriter(os.Stdout)
	defer out.Flush()
	for in.Scan() {
		t := in.Text()
		if t == "" {
			continue
		}
		f := strings.Split(t, " ")
		switch f[0] {
		case "rd":
			fmt.Fprintln(out, reads(f))
		case "ra":
			fmt.Fprintln(out, readAll(f))
		default:
			fmt.Fprintln(out, write(f))
		}
	}
}
