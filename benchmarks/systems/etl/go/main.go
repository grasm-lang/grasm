// Command etl is a streaming JSON-to-database ETL.
//
// It reads a JSON Lines file line by line, decodes each line into typed
// structs with encoding/json, validates, transforms, and inserts batches of
// 1000 into an in-memory "database" that keeps one aggregate per city and a
// running checksum. Each batch is dropped after its insert.
//
//	etl <path.jsonl>
package main

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"sort"
	"strings"
)

const (
	batchSize = 1000
	modulus   = 1_000_000_007
)

// encoding/json leaves a missing field at its zero value, so required fields
// are pointers and decode checks them.
type Address struct {
	City *string `json:"city"`
	Zip  *string `json:"zip"`
}

type User struct {
	Name    *string  `json:"name"`
	Email   *string  `json:"email"`
	Tags    []string `json:"tags"`
	Address *Address `json:"address"`
}

type Order struct {
	SKU   *string `json:"sku"`
	Qty   *int64  `json:"qty"`
	Price *int64  `json:"price"`
}

type Customer struct {
	ID     *int64  `json:"id"`
	User   *User   `json:"user"`
	Orders []Order `json:"orders"`
	Note   *string `json:"note"`
}

var errMissing = errors.New("missing field")

func decode(line []byte) (*Customer, error) {
	var c Customer
	if err := json.Unmarshal(line, &c); err != nil {
		return nil, err
	}
	if c.ID == nil || c.User == nil || c.Orders == nil || c.Note == nil {
		return nil, errMissing
	}
	u := c.User
	if u.Name == nil || u.Email == nil || u.Tags == nil || u.Address == nil {
		return nil, errMissing
	}
	if u.Address.City == nil || u.Address.Zip == nil {
		return nil, errMissing
	}
	for _, o := range c.Orders {
		if o.SKU == nil || o.Qty == nil || o.Price == nil {
			return nil, errMissing
		}
	}
	return &c, nil
}

// Row is what the pipeline loads: one per valid customer.
type Row struct {
	ID         int64
	Name       string
	City       string
	OrderCount int64
	TotalCents int64
	TagCount   int64
}

func validate(c *Customer) bool {
	if !strings.Contains(*c.User.Email, "@") {
		return false
	}
	for _, o := range c.Orders {
		if *o.Qty <= 0 || *o.Price < 0 {
			return false
		}
	}
	return true
}

func transform(c *Customer) Row {
	var total int64
	for _, o := range c.Orders {
		total += *o.Qty * *o.Price
	}
	return Row{
		ID:         *c.ID,
		Name:       *c.User.Name,
		City:       *c.User.Address.City,
		OrderCount: int64(len(c.Orders)),
		TotalCents: total,
		TagCount:   int64(len(c.User.Tags)),
	}
}

type cityAgg struct {
	count, total int64
}

// DB is the mock database.
type DB struct {
	cities     map[string]*cityAgg
	rows       int64
	totalCents int64
	checksum   int64
}

func (db *DB) Insert(batch []Row) {
	for _, r := range batch {
		db.rows++
		db.totalCents += r.TotalCents
		mix := r.ID*7919 + r.TotalCents + r.OrderCount*31 + r.TagCount*17 + int64(len(r.Name))
		db.checksum = (db.checksum*1_000_003 + mix) % modulus
		agg, ok := db.cities[r.City]
		if !ok {
			agg = &cityAgg{}
			db.cities[r.City] = agg
		}
		agg.count++
		agg.total += r.TotalCents
	}
}

func (db *DB) CityDigest() int64 {
	names := make([]string, 0, len(db.cities))
	for c := range db.cities {
		names = append(names, c)
	}
	sort.Strings(names)
	var d int64
	for _, c := range names {
		agg := db.cities[c]
		d = (d*131 + agg.count*7 + agg.total + int64(len(c))) % modulus
	}
	return d
}

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: etl <path.jsonl>")
		os.Exit(2)
	}
	f, err := os.Open(os.Args[1])
	if err != nil {
		fmt.Fprintln(os.Stderr, "etl:", err)
		os.Exit(1)
	}
	defer f.Close()

	db := &DB{cities: make(map[string]*cityAgg)}
	batch := make([]Row, 0, batchSize)
	var rejected int64

	sc := bufio.NewScanner(f)
	sc.Buffer(make([]byte, 1<<20), 64<<20)
	for sc.Scan() {
		line := sc.Bytes()
		if len(line) == 0 {
			continue
		}
		c, err := decode(line)
		if err != nil || !validate(c) {
			rejected++
			continue
		}
		batch = append(batch, transform(c))
		if len(batch) == batchSize {
			db.Insert(batch)
			batch = batch[:0]
		}
	}
	if err := sc.Err(); err != nil {
		fmt.Fprintln(os.Stderr, "etl: read error:", err)
		os.Exit(1)
	}
	if len(batch) > 0 {
		db.Insert(batch)
	}

	fmt.Printf("valid=%d rejected=%d cities=%d total_cents=%d checksum=%d city_digest=%d\n",
		db.rows, rejected, len(db.cities), db.totalCents, db.checksum, db.CityDigest())
}
