// v1.0.0 — pure.
package mathlib

func Total(xs []int64) int64 {
	var t int64
	for _, x := range xs { t += x }
	return t
}
