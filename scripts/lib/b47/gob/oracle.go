// The oracle for packages/gobwire: Go's own encoding/gob. Writes streams of
// random values, decodes each with gob.Decoder into the same Go types, and
// prints `path<TAB>hex(rendering)` -- the canonical form packages/gobwire's
// `render` gives -- or `path<TAB>error` when Go refuses the stream.
//
//	go run . <seed> <count> <dir>
package main

import (
	"bytes"
	"encoding/gob"
	"encoding/hex"
	"fmt"
	"math"
	"math/rand"
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"strings"
	"time"
)

type Leaf struct {
	A int
	B string
	C float64
	D bool
	E []byte
	F uint16
	G int8
	H float32
	I complex128
	J [2]uint8
	k int
}

type Tree struct {
	Name   string
	Kids   []*Tree
	Weight *int
	Tags   map[string]int32
	Any    interface{}
	When   time.Time
	Leaf   Leaf
	Opt    *Leaf
	Grid   [2][2]int16
	U64    uint64
	I64    int64
}

type Pair struct {
	K string
	V int
}

var r *rand.Rand

func pick[T any](xs ...T) T { return xs[r.Intn(len(xs))] }

func str() string {
	return pick("", "a", "hello", "é漢", "x\x00y", "\xff\xfe", strings.Repeat("z", 200))
}

func leaf() Leaf {
	l := Leaf{}
	if r.Intn(2) == 0 {
		l.A = pick(0, 1, -1, 63, 64, -64, -65, 255, 256, math.MaxInt64, math.MinInt64, r.Int())
	}
	if r.Intn(2) == 0 {
		l.B = str()
	}
	if r.Intn(2) == 0 {
		l.C = pick(0.0, 1.5, -2.25, math.Inf(1), math.NaN(), 1e300, 5e-324, math.Copysign(0, -1), r.NormFloat64())
	}
	l.D = r.Intn(2) == 0
	if r.Intn(2) == 0 {
		l.E = pick([]byte{}, []byte{0}, []byte("bytes"), nil)
	}
	l.F = uint16(pick(0, 1, 127, 128, 65535))
	l.G = int8(pick(0, -128, 127, 5))
	l.H = pick(float32(0), float32(0.1), float32(-3.5), float32(math.MaxFloat32))
	if r.Intn(3) == 0 {
		l.I = complex(r.NormFloat64(), pick(0.0, 2.0))
	}
	l.J = [2]uint8{uint8(r.Intn(256)), pick(uint8(0), uint8(255))}
	l.k = 9
	return l
}

func tree(depth int) Tree {
	t := Tree{Name: str()}
	if depth < 2 {
		for i := r.Intn(3); i > 0; i-- {
			k := tree(depth + 1)
			t.Kids = append(t.Kids, &k)
		}
	}
	if r.Intn(2) == 0 {
		w := pick(0, 7, -3)
		t.Weight = &w
	}
	switch r.Intn(3) {
	case 0:
		t.Tags = map[string]int32{}
	case 1:
		t.Tags = map[string]int32{"a": 1, "b": -2, str(): int32(r.Intn(1000))}
	}
	switch r.Intn(7) {
	case 0:
		t.Any = leaf()
	case 1:
		l := leaf()
		t.Any = &l
	case 2:
		t.Any = Pair{K: str(), V: r.Intn(5)}
	case 3:
		t.Any = pick(0, 42, -1)
	case 4:
		t.Any = str()
	case 5:
		t.Any = []int{1, 2, 3}
	}
	if r.Intn(2) == 0 {
		t.When = time.Unix(int64(r.Intn(2000000000)), int64(r.Intn(1000000000))).UTC()
	}
	t.Leaf = leaf()
	if r.Intn(2) == 0 {
		o := leaf()
		t.Opt = &o
	}
	t.Grid = [2][2]int16{{int16(r.Intn(100)), 0}, {-1, int16(r.Intn(3))}}
	t.U64 = pick(uint64(0), uint64(1), math.MaxUint64, uint64(1)<<63, uint64(r.Int63()))
	t.I64 = pick(int64(0), math.MinInt64, int64(r.Int63()))
	return t
}

// ---- the canonical rendering

func hx(b []byte) string { return hex.EncodeToString(b) }

func fbits(f float64) string { return fmt.Sprintf("%016x", math.Float64bits(f)) }

func render(v reflect.Value) string {
	if v.Kind() == reflect.Ptr {
		if v.IsNil() {
			if v.Type().Elem().Kind() == reflect.Struct && v.Type().Elem() != reflect.TypeOf(time.Time{}) {
				return "nil"
			}
			return render(reflect.Zero(v.Type().Elem()))
		}
		return render(v.Elem())
	}
	if v.Type() == reflect.TypeOf(time.Time{}) {
		if v.Interface().(time.Time).IsZero() {
			return "G-"
		}
		b, _ := v.Interface().(time.Time).GobEncode()
		return "G:" + hx(b)
	}
	switch v.Kind() {
	case reflect.Bool:
		if v.Bool() {
			return "t"
		}
		return "f"
	case reflect.Int, reflect.Int8, reflect.Int16, reflect.Int32, reflect.Int64:
		return fmt.Sprintf("i:%d", v.Int())
	case reflect.Uint, reflect.Uint8, reflect.Uint16, reflect.Uint32, reflect.Uint64, reflect.Uintptr:
		return fmt.Sprintf("u:%d", v.Uint())
	case reflect.Float32, reflect.Float64:
		return "f:" + fbits(v.Float())
	case reflect.Complex64, reflect.Complex128:
		c := v.Complex()
		return "c:" + fbits(real(c)) + "," + fbits(imag(c))
	case reflect.String:
		return "s:" + hx([]byte(v.String()))
	case reflect.Slice:
		if v.Type().Elem().Kind() == reflect.Uint8 {
			return "b:" + hx(v.Bytes())
		}
		fallthrough
	case reflect.Array:
		parts := []string{}
		for i := 0; i < v.Len(); i++ {
			parts = append(parts, render(v.Index(i)))
		}
		return "[" + strings.Join(parts, ",") + "]"
	case reflect.Map:
		parts := []string{}
		for _, k := range v.MapKeys() {
			parts = append(parts, render(k)+":"+render(v.MapIndex(k)))
		}
		sort.Strings(parts)
		return "{" + strings.Join(parts, ",") + "}"
	case reflect.Struct:
		parts := []string{}
		for i := 0; i < v.NumField(); i++ {
			f := v.Type().Field(i)
			if !f.IsExported() {
				continue
			}
			parts = append(parts, f.Name+"="+render(v.Field(i)))
		}
		return "{" + strings.Join(parts, ",") + "}"
	case reflect.Interface:
		if v.IsNil() {
			return "nil"
		}
		c := v.Elem()
		return "I(" + c.Type().String() + ":" + render(c) + ")"
	}
	return "?"
}

type item struct {
	val  any
	into func() any
}

func value() item {
	switch r.Intn(9) {
	case 0, 1, 2:
		t := tree(0)
		return item{t, func() any { return new(Tree) }}
	case 3:
		return item{leaf(), func() any { return new(Leaf) }}
	case 4:
		return item{pick(0, 5, -9, math.MaxInt64), func() any { return new(int) }}
	case 5:
		return item{str(), func() any { return new(string) }}
	case 6:
		ps := []Pair{}
		for i := r.Intn(4); i > 0; i-- {
			ps = append(ps, Pair{str(), r.Intn(9)})
		}
		return item{ps, func() any { return new([]Pair) }}
	case 7:
		m := map[Pair]Leaf{}
		for i := r.Intn(3); i > 0; i-- {
			m[Pair{str(), i}] = leaf()
		}
		return item{m, func() any { return new(map[Pair]Leaf) }}
	}
	var iv interface{} = pick[interface{}](leaf(), Pair{"p", 1}, 7, "s", []int{4})
	return item{&iv, func() any { return new(interface{}) }}
}

func main() {
	var seed, count int
	fmt.Sscan(os.Args[1], &seed)
	fmt.Sscan(os.Args[2], &count)
	dir := os.Args[3]
	r = rand.New(rand.NewSource(int64(seed)))
	gob.Register(Leaf{})
	gob.Register(Pair{})
	for c := 0; c < count; c++ {
		var buf bytes.Buffer
		e := gob.NewEncoder(&buf)
		items := []item{}
		for i := 1 + r.Intn(3); i > 0; i-- {
			it := value()
			if err := e.Encode(it.val); err != nil {
				panic(err)
			}
			items = append(items, it)
		}
		data := buf.Bytes()
		if r.Intn(8) == 0 && len(data) > 1 {
			data = data[:r.Intn(len(data))]
		}
		path := filepath.Join(dir, fmt.Sprintf("g%d_%d.gob", seed, c))
		os.WriteFile(path, data, 0o644)
		d := gob.NewDecoder(bytes.NewReader(data))
		outs := []string{}
		failed := false
		for _, it := range items {
			p := it.into()
			if err := d.Decode(p); err != nil {
				failed = true
				break
			}
			outs = append(outs, render(reflect.ValueOf(p)))
		}
		if failed {
			fmt.Printf("%s\t%d\terror\n", path, len(items))
		} else {
			fmt.Printf("%s\t%d\t%s\n", path, len(items), hx([]byte(strings.Join(outs, "\n"))))
		}
	}
}
