package main

import (
	"fmt"
	"go/scanner"
	"go/token"
	"os"
)

func main() {
	for _, path := range os.Args[1:] {
		src, err := os.ReadFile(path)
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			os.Exit(1)
		}
		fset := token.NewFileSet()
		file := fset.AddFile(path, fset.Base(), len(src))
		var s scanner.Scanner
		s.Init(file, src, nil, 0)
		n := 0
		for {
			_, tok, _ := s.Scan()
			if tok == token.EOF {
				break
			}
			if tok == token.COMMENT {
				continue
			}
			n++
		}
		fmt.Println(n)
	}
}
