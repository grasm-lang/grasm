package main

import "fmt"

func main() {
	n := 200
	a := make([]int64, n*n)
	b := make([]int64, n*n)
	c := make([]int64, n*n)
	for i := 0; i < n*n; i++ {
		a[i] = int64(i % 7)
		b[i] = int64(i % 5)
	}
	for i := 0; i < n; i++ {
		for k := 0; k < n; k++ {
			aik := a[i*n+k]
			if aik != 0 {
				for j := 0; j < n; j++ {
					c[i*n+j] += aik * b[k*n+j]
				}
			}
		}
	}
	var total int64
	for _, v := range c {
		total += v
	}
	fmt.Println(total)
}
