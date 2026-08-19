// Allocation-shaped work: build a tree, walk it, let it go, repeat.
package main

import "fmt"

type Node struct {
	value int64
	kids  []*Node
}

func build(depth, branch, seed int64) *Node {
	if depth <= 0 {
		return &Node{value: seed % 1000}
	}
	kids := make([]*Node, 0, branch)
	for i := int64(0); i < branch; i++ {
		kids = append(kids, build(depth-1, branch, seed*31+i+1))
	}
	return &Node{value: seed % 1000, kids: kids}
}

func total(n *Node) int64 {
	sum := n.value
	for _, k := range n.kids {
		sum += total(k)
	}
	return sum
}

func main() {
	var acc int64
	for r := int64(0); r < 20; r++ {
		t := build(9, 3, r+1)
		acc = (acc + total(t)) % 1000000007
	}
	fmt.Println(acc)
}
