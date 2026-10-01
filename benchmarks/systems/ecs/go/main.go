// ECS update loop, array-of-structs: []Entity updated in place.
// Usage: ecs [N] [S]   (defaults N=1000000, S=100)
package main

import (
	"fmt"
	"math"
	"os"
	"strconv"
)

type Entity struct {
	X, Y, Z    float64
	VX, VY, VZ float64
}

func next(s int64) int64 {
	return (s*1103515245 + 12345) % 2147483648
}

func argOr(i int, def int) int {
	if len(os.Args) > i {
		if v, err := strconv.Atoi(os.Args[i]); err == nil {
			return v
		}
	}
	return def
}

func reflect(p, v *float64) {
	if *p > 1000 {
		*p = 2000 - *p
		*v = -*v
	} else if *p < -1000 {
		*p = -2000 - *p
		*v = -*v
	}
}

func main() {
	n := argOr(1, 1000000)
	steps := argOr(2, 100)
	const dt = 0.01

	var s int64 = 42
	gen := func(m, off int64, div float64) float64 {
		s = next(s)
		return float64(s%m-off) / div
	}
	es := make([]Entity, n)
	for i := range es {
		e := &es[i]
		e.X = gen(2000001, 1000000, 1000)
		e.Y = gen(2000001, 1000000, 1000)
		e.Z = gen(2000001, 1000000, 1000)
		e.VX = gen(2001, 1000, 10)
		e.VY = gen(2001, 1000, 10)
		e.VZ = gen(2001, 1000, 10)
	}

	for step := 0; step < steps; step++ {
		for i := range es {
			e := &es[i]
			// The explicit float64() conversions stop the Go compiler from
			// fusing x + v*dt into one FMA instruction (allowed by the Go
			// spec, done on arm64), which would round differently from the
			// Rust and Grasm versions.
			e.X += float64(e.VX * dt)
			e.Y += float64(e.VY * dt)
			e.Z += float64(e.VZ * dt)
			reflect(&e.X, &e.VX)
			reflect(&e.Y, &e.VY)
			reflect(&e.Z, &e.VZ)
		}
	}

	sum := 0.0
	var isum int64
	for i := range es {
		e := &es[i]
		sum += e.X + e.Y + e.Z
		isum += int64(math.Floor(float64(e.X*1000))) + int64(math.Floor(float64(e.Y*1000))) + int64(math.Floor(float64(e.Z*1000)))
	}
	fmt.Printf("checksum %.6f %d\n", sum, isum)
}
