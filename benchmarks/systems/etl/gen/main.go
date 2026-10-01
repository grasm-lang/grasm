// Command gen writes the ETL benchmark's input: JSON Lines, one nested
// customer record per line, deterministic from a seed.
//
// About 3% of the records are invalid on purpose, in one of six ways: a
// required field missing, an email without '@', an order quantity <= 0, an id
// written as a string, a price written as a string, and tags that are a string
// instead of a list. Money is integer cents throughout, so no language has to
// agree with another about a float.
//
//	go run . -size 1073741824 -seed 42 -out /path/to/etl.jsonl
package main

import (
	"bufio"
	"flag"
	"fmt"
	"os"
	"strconv"
)

// splitmix64: tiny, fast, and the same sequence on every platform.
type rng struct{ s uint64 }

func (r *rng) next() uint64 {
	r.s += 0x9e3779b97f4a7c15
	z := r.s
	z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9
	z = (z ^ (z >> 27)) * 0x94d049bb133111eb
	return z ^ (z >> 31)
}

func (r *rng) intn(n int) int { return int(r.next() % uint64(n)) }

var firstNames = []string{
	"Ada", "Grace", "Alan", "Linus", "Margaret", "Dennis", "Barbara", "Ken",
	"Frances", "Edsger", "Radia", "Donald", "Hedy", "John", "Katherine", "Niklaus",
	"José", "Zoë", "Björn", "Søren", "Chloé", "Mátyás", "Łukasz", "Renée",
	"Yuki", "Wei", "Priya", "Olu", "Fatima", "Mateo", "Ingrid", "Kofi",
}

var lastNames = []string{
	"Lovelace", "Hopper", "Turing", "Torvalds", "Hamilton", "Ritchie", "Liskov",
	"Thompson", "Allen", "Dijkstra", "Perlman", "Knuth", "Lamarr", "Backus",
	"Johnson", "Wirth", "García", "Müller", "Ångström", "Nørgaard", "O'Brien",
	"Tanaka", "Chen", "Patel", "Okafor", "Haddad", "Rossi", "Kowalski",
}

var cityBases = []string{
	"Springfield", "Riverton", "Lakeside", "Fairview", "Greenville", "Madison",
	"Georgetown", "Salem", "Franklin", "Clinton", "Arlington", "Ashland",
	"São Paulo", "Zürich", "Kraków", "Malmö", "Reykjavík", "Düsseldorf",
	"Québec", "Tromsø", "Córdoba", "Łódź", "Besançon", "Nürnberg",
}

var tagWords = []string{"vip", "new", "returning", "wholesale", "promo", "beta", "eu", "us", "apac", "mobile"}

var words = []string{
	"the", "order", "arrived", "late", "but", "customer", "was", "happy", "with",
	"quality", "please", "ship", "next", "batch", "to", "warehouse", "north",
	"invoice", "attached", "refund", "requested", "for", "damaged", "item",
	"lorem", "ipsum", "dolor", "sit", "amet", "consectetur", "adipiscing", "elit",
	"café", "naïve", "façade", "résumé", "über", "piñata",
}

// Escaped fragments, written as JSON source, so every parser has to decode
// them: quotes, backslashes, control escapes and \u escapes in the BMP.
var escapes = []string{`\"quoted\"`, `line\nbreak`, `tab\there`, `back\\slash`, "caf" + bs + "u00e9", bs + "u4e2d" + bs + "u6587", `a\/b`}

const bs = `\`

func quote(s string) string { return strconv.Quote(s) }

func main() {
	size := flag.Int64("size", 1<<30, "stop once at least this many bytes are written")
	seed := flag.Uint64("seed", 42, "generator seed")
	out := flag.String("out", "etl.jsonl", "output path")
	flag.Parse()

	f, err := os.Create(*out)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	w := bufio.NewWriterSize(f, 1<<20)
	r := &rng{s: *seed}

	var written int64
	var lines, invalid int64
	buf := make([]byte, 0, 4096)
	for id := int64(1); written < *size; id++ {
		buf = buf[:0]
		bad := -1
		if r.intn(1000) < 30 {
			bad = r.intn(6)
			invalid++
		}
		first := firstNames[r.intn(len(firstNames))]
		last := lastNames[r.intn(len(lastNames))]
		name := first + " " + last
		email := fmt.Sprintf("%s.%s%d@example.com", first, last, r.intn(1000))
		if bad == 1 {
			email = fmt.Sprintf("%s.%s%d.example.com", first, last, r.intn(1000))
		}
		city := fmt.Sprintf("%s-%d", cityBases[r.intn(len(cityBases))], r.intn(20))
		zip := fmt.Sprintf("%05d", r.intn(100000))
		// Which field a "missing field" record loses.
		missing := ""
		if bad == 0 {
			missing = []string{"id", "email", "address", "city", "orders", "qty", "note"}[r.intn(7)]
		}

		buf = append(buf, '{')
		if missing != "id" {
			buf = append(buf, `"id":`...)
			if bad == 3 {
				buf = append(buf, '"')
				buf = strconv.AppendInt(buf, id, 10)
				buf = append(buf, '"')
			} else {
				buf = strconv.AppendInt(buf, id, 10)
			}
			buf = append(buf, ',')
		}
		buf = append(buf, `"user":{"name":`...)
		buf = append(buf, quote(name)...)
		if missing != "email" {
			buf = append(buf, `,"email":`...)
			buf = append(buf, quote(email)...)
		}
		buf = append(buf, `,"tags":`...)
		ntags := r.intn(5)
		if bad == 5 {
			buf = append(buf, quote(tagWords[r.intn(len(tagWords))])...)
		} else {
			buf = append(buf, '[')
			for t := 0; t < ntags; t++ {
				if t > 0 {
					buf = append(buf, ',')
				}
				buf = append(buf, quote(tagWords[r.intn(len(tagWords))])...)
			}
			buf = append(buf, ']')
		}
		if missing != "address" {
			buf = append(buf, `,"address":{`...)
			if missing != "city" {
				buf = append(buf, `"city":`...)
				buf = append(buf, quote(city)...)
				buf = append(buf, ',')
			}
			buf = append(buf, `"zip":`...)
			buf = append(buf, quote(zip)...)
			buf = append(buf, '}')
		}
		buf = append(buf, '}')

		if missing != "orders" {
			buf = append(buf, `,"orders":[`...)
			norders := r.intn(6) // 0..5; an empty list is valid
			if bad == 2 || bad == 4 || missing == "qty" {
				norders = 1 + r.intn(5)
			}
			spoil := r.intn(norders + 1)
			if spoil == norders && norders > 0 {
				spoil = norders - 1
			}
			for o := 0; o < norders; o++ {
				if o > 0 {
					buf = append(buf, ',')
				}
				sku := fmt.Sprintf("SKU-%06d", r.intn(1000000))
				qty := 1 + r.intn(9)
				price := 99 + r.intn(50000)
				buf = append(buf, `{"sku":`...)
				buf = append(buf, quote(sku)...)
				if !(missing == "qty" && o == spoil) {
					buf = append(buf, `,"qty":`...)
					if bad == 2 && o == spoil {
						qty = -r.intn(3) // 0, -1 or -2
					}
					buf = strconv.AppendInt(buf, int64(qty), 10)
				}
				buf = append(buf, `,"price":`...)
				if bad == 4 && o == spoil {
					buf = append(buf, '"')
					buf = strconv.AppendInt(buf, int64(price), 10)
					buf = append(buf, '"')
				} else {
					buf = strconv.AppendInt(buf, int64(price), 10)
				}
				buf = append(buf, '}')
			}
			buf = append(buf, ']')
		}

		if missing != "note" {
			buf = append(buf, `,"note":"`...)
			target := 150 + r.intn(350)
			n := 0
			for n < target {
				var piece string
				if r.intn(25) == 0 {
					piece = escapes[r.intn(len(escapes))]
				} else {
					piece = words[r.intn(len(words))]
				}
				if n > 0 {
					buf = append(buf, ' ')
					n++
				}
				buf = append(buf, piece...)
				n += len(piece)
			}
			buf = append(buf, '"')
		}
		// An unknown field now and then: every decoder must ignore it.
		if r.intn(20) == 0 {
			buf = append(buf, `,"source":{"channel":"web","v":2}`...)
		}
		buf = append(buf, '}', '\n')

		if _, err := w.Write(buf); err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		written += int64(len(buf))
		lines++
	}
	if err := w.Flush(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if err := f.Close(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	fmt.Fprintf(os.Stderr, "gen: %d lines, %d bytes, %d invalid by construction\n", lines, written, invalid)
}
