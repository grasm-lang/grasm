// The reference for b42_diff.py's Go families: go/scanner with comments,
// one line per file -- the tokens as "offset token hex(literal)" lines,
// hex-encoded, then "|" and the number of errors reported.
package main

import (
	"encoding/hex"
	"fmt"
	"go/scanner"
	"go/token"
	"os"
	"strings"
)

func main() {
	for _, path := range os.Args[1:] {
		src, err := os.ReadFile(path)
		if err != nil {
			fmt.Println("NOFILE")
			continue
		}
		fset := token.NewFileSet()
		file := fset.AddFile(path, fset.Base(), len(src))
		var s scanner.Scanner
		errs := 0
		s.Init(file, src, func(pos token.Position, msg string) { errs++ }, scanner.ScanComments)
		var lines []string
		for {
			pos, tok, lit := s.Scan()
			lines = append(lines, fmt.Sprintf("%d %s %x", file.Offset(pos), tok.String(), lit))
			if tok == token.EOF {
				break
			}
		}
		fmt.Printf("%s|%d\n", hex.EncodeToString([]byte(strings.Join(lines, "\n"))), errs)
	}
}
