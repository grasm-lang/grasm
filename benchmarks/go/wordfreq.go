package main

import (
	"fmt"
	"strconv"
)

func main() {
	words := []string{"alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"}
	counts := make(map[string]int64)
	for i := int64(0); i < 400000; i++ {
		w := words[i%8] + strconv.FormatInt(i%97, 10)
		counts[w]++
	}
	var total int64
	for _, v := range counts {
		total += v
	}
	fmt.Println(len(counts), total)
}
