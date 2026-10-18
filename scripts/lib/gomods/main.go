// The reference side of the gates that check Grasm packages against Go
// modules already in the module cache (built offline: GOFLAGS=-mod=mod
// GOPROXY=off GOSUMDB=off). It reads a JSON list of requests on stdin and
// writes a JSON list of answers.
package main

import (
	"encoding/json"
	"os"
	"sort"
	"strings"

	"github.com/cespare/xxhash/v2"
	rendezvous "github.com/dgryski/go-rendezvous"
)

type request struct {
	Op    string   `json:"op"`
	Nodes []string `json:"nodes"`
	Keys  []string `json:"keys"`
}

// score is go-rendezvous's xorshiftMult64 over the two hashes, which the
// package does not export; Lookup below is the package's own.
func score(k, n uint64) uint64 {
	x := k ^ n
	x ^= x >> 12
	x ^= x << 25
	x ^= x >> 27
	return x * 2685821657736338717
}

func answer(r request) string {
	switch r.Op {
	case "rdv":
		ring := rendezvous.New(r.Nodes, xxhash.Sum64String)
		var out []string
		for _, k := range r.Keys {
			order := make([]int, len(r.Nodes))
			for i := range order {
				order[i] = i
			}
			kh := xxhash.Sum64String(k)
			sort.SliceStable(order, func(a, b int) bool {
				return score(kh, xxhash.Sum64String(r.Nodes[order[a]])) > score(kh, xxhash.Sum64String(r.Nodes[order[b]]))
			})
			names := make([]string, len(order))
			for i, o := range order {
				names[i] = r.Nodes[o]
			}
			out = append(out, ring.Lookup(k)+"/"+strings.Join(names, ","))
		}
		return strings.Join(out, " ")
	}
	return "UNKNOWN"
}

func main() {
	var reqs []request
	if err := json.NewDecoder(os.Stdin).Decode(&reqs); err != nil {
		panic(err)
	}
	out := make([]string, len(reqs))
	for i, r := range reqs {
		out[i] = answer(r)
	}
	json.NewEncoder(os.Stdout).Encode(out)
}
