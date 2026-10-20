package main

import (
	"os"
	"runtime/pprof"
)

func busy(n int) int {
	s := 0
	for i := 0; i < n; i++ {
		s += i * i % 7
	}
	return s
}

func main() {
	f, _ := os.Create(os.Args[1])
	pprof.StartCPUProfile(f)
	t := 0
	for k := 0; k < 300; k++ {
		t += busy(200000)
	}
	pprof.StopCPUProfile()
	f.Close()
	h, _ := os.Create(os.Args[2])
	pprof.WriteHeapProfile(h)
	h.Close()
	println(t)
}
