package main

import (
	"fmt"
	"runtime"
	"sync"
)

func mix(seed int64) int64 {
	acc := seed
	for i := 0; i < 800; i++ {
		acc = (acc*1103515245 + 12345) % 2147483648
	}
	return acc % 1000
}

func main() {
	const n = 200000
	xs := make([]int64, n)
	for i := range xs {
		xs[i] = int64(i)
	}
	out := make([]int64, n)
	workers := runtime.NumCPU()
	chunk := (n + workers - 1) / workers
	var wg sync.WaitGroup
	for w := 0; w < workers; w++ {
		lo := w * chunk
		hi := lo + chunk
		if lo >= n {
			break
		}
		if hi > n {
			hi = n
		}
		wg.Add(1)
		go func(lo, hi int) {
			defer wg.Done()
			for i := lo; i < hi; i++ {
				out[i] = mix(xs[i])
			}
		}(lo, hi)
	}
	wg.Wait()
	var total int64
	for _, v := range out {
		total += v
	}
	fmt.Println(total)
}
