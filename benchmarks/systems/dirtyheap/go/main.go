// Dirty heap, Go: a slice of *Node with pointer edges. The drop is letting go
// of the root slice and forcing a collection with runtime.GC(); the timed
// figure is that GC call, and the runtime's own stop-the-world pause figures
// are reported beside it, because most of a Go collection is concurrent and
// the wall time of runtime.GC() is not the pause the program would feel.
package main

import (
	"fmt"
	"os"
	"runtime"
	"strconv"
	"time"
)

type Node struct {
	URL   string
	Edges []*Node
	Seen  bool
}

func lcg(x int64) int64 {
	return (x*1103515245 + 12345) % 2147483648
}

func build(n int) []*Node {
	nodes := make([]*Node, n)
	for i := range nodes {
		nodes[i] = &Node{URL: fmt.Sprintf("https://site%d.example/page/%d", i%1000, i)}
	}
	s := int64(42)
	for _, node := range nodes {
		s = lcg(s)
		deg := 1 + s%9
		node.Edges = make([]*Node, 0, deg)
		for e := int64(0); e < deg; e++ {
			s = lcg(s)
			node.Edges = append(node.Edges, nodes[s%int64(n)])
		}
	}
	return nodes
}

func bfs(root *Node) (int, int) {
	root.Seen = true
	queue := []*Node{root}
	bytes := 0
	for head := 0; head < len(queue); head++ {
		v := queue[head]
		bytes += len(v.URL)
		for _, w := range v.Edges {
			if !w.Seen {
				w.Seen = true
				queue = append(queue, w)
			}
		}
	}
	return len(queue), bytes
}

func main() {
	start := time.Now()
	n := 5000000
	if len(os.Args) > 1 {
		if v, err := strconv.Atoi(os.Args[1]); err == nil {
			n = v
		}
	}
	graph := build(n)
	built := time.Since(start)
	reached, bytes := bfs(graph[0])
	fmt.Printf("checksum nodes=%d reachable=%d url_bytes=%d\n", n, reached, bytes)

	var before, after runtime.MemStats
	runtime.ReadMemStats(&before)
	t0 := time.Now()
	graph = nil
	runtime.GC()
	dropT := time.Since(t0)
	runtime.ReadMemStats(&after)

	fmt.Printf("build_ms %d\n", built.Milliseconds())
	fmt.Printf("drop_ms %.3f\n", float64(dropT.Nanoseconds())/1e6)
	fmt.Printf("gc_drop_stw_pause_ms %.3f\n", float64(after.PauseTotalNs-before.PauseTotalNs)/1e6)
	fmt.Printf("gc_total_pause_ms %.3f over %d GCs\n", float64(after.PauseTotalNs)/1e6, after.NumGC)
	fmt.Printf("gc_heap_alloc_mb before=%d after=%d\n", before.HeapAlloc>>20, after.HeapAlloc>>20)
	fmt.Printf("total_ms %d\n", time.Since(start).Milliseconds())
	runtime.KeepAlive(graph)
}
