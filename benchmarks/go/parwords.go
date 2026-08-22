package main

import (
	"fmt"
	"runtime"
	"strconv"
	"strings"
	"sync"
)

// strings.Builder, because `acc += piece` in a loop copies the accumulator
// every step in Go and every Go programmer knows to reach for this instead.
func heavy(w string) string {
	var acc strings.Builder
	acc.WriteString(w)
	for i := 0; i < 200; i++ {
		acc.WriteByte(byte('0' + i%7))
	}
	return strconv.Itoa(acc.Len()) + "-" + w
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
		for i := 0; i < len(s); i++ {
			h = (h*131 + int64(s[i])) % 1000000007
		}
	}
	fmt.Println(h)
}
