package main

import (
	"fmt"
	"runtime"
	"strconv"
	"sync"
)

func heavy(w string) string {
	acc := w
	for i := 0; i < 200; i++ {
		acc += strconv.Itoa(i % 7)
	}
	return strconv.Itoa(len([]rune(acc))) + "-" + w
}

func main() {
	const n = 20000
	words := make([]string, n)
	for i := range words {
		words[i] = "w" + strconv.Itoa(i)
	}
	out := make([]string, n)
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
				out[i] = heavy(words[i])
			}
		}(lo, hi)
	}
	wg.Wait()
	var h int64
	for _, s := range out {
		for _, c := range s {
			h = (h*131 + int64(c)) % 1000000007
		}
	}
	fmt.Println(h)
}
