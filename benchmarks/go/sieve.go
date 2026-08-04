package main

import "fmt"

func main() {
	limit := 2000000
	flags := make([]bool, limit+1)
	for i := range flags {
		flags[i] = true
	}
	count := 0
	for i := 2; i <= limit; i++ {
		if flags[i] {
			count++
			for j := i * i; j <= limit; j += i {
				flags[j] = false
			}
		}
	}
	fmt.Println(count)
}
