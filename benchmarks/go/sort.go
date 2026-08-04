package main

import (
	"fmt"
	"sort"
)

func main() {
	n := 400000
	xs := make([]int64, 0, n)
	var seed int64 = 12345
	for i := 0; i < n; i++ {
		seed = (seed*1103515245 + 12345) % 2147483648
		xs = append(xs, seed%1000000)
	}
	sort.Slice(xs, func(i, j int) bool { return xs[i] < xs[j] })
	var checksum int64
	for k, v := range xs {
		checksum = (checksum + v*(int64(k)%7+1)) % 1000000007
	}
	fmt.Println(n, checksum)
}
