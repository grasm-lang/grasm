package main

// The fifth part of the oracle: ML-DSA and SLH-DSA (cloudflare/circl),
// signing deterministically through the standard interface.

import (
	"bytes"
	"encoding/hex"
	"fmt"

	"github.com/cloudflare/circl/sign/mldsa/mldsa44"
	"github.com/cloudflare/circl/sign/mldsa/mldsa65"
	"github.com/cloudflare/circl/sign/mldsa/mldsa87"
	"github.com/cloudflare/circl/sign/slhdsa"
)

func answer5(r request) string {
	switch r.Op {
	case "mldsa.keygen":
		var seed [32]byte
		copy(seed[:], unhex(r.Hex))
		switch r.Args[0] {
		case "ML-DSA-44":
			pk, sk := mldsa44.NewKeyFromSeed(&seed)
			return hex.EncodeToString(pk.Bytes()) + "|" + hex.EncodeToString(sk.Bytes())
		case "ML-DSA-65":
			pk, sk := mldsa65.NewKeyFromSeed(&seed)
			return hex.EncodeToString(pk.Bytes()) + "|" + hex.EncodeToString(sk.Bytes())
		default:
			pk, sk := mldsa87.NewKeyFromSeed(&seed)
			return hex.EncodeToString(pk.Bytes()) + "|" + hex.EncodeToString(sk.Bytes())
		}
	case "mldsa.sign":
		f := fields(r)
		ctx := unhex(r.Args[1])
		switch r.Args[0] {
		case "ML-DSA-44":
			var sk mldsa44.PrivateKey
			if sk.UnmarshalBinary(f[0]) != nil {
				return "ERR"
			}
			sig := make([]byte, mldsa44.SignatureSize)
			if mldsa44.SignTo(&sk, f[1], ctx, false, sig) != nil {
				return "ERR"
			}
			return hex.EncodeToString(sig)
		case "ML-DSA-65":
			var sk mldsa65.PrivateKey
			if sk.UnmarshalBinary(f[0]) != nil {
				return "ERR"
			}
			sig := make([]byte, mldsa65.SignatureSize)
			if mldsa65.SignTo(&sk, f[1], ctx, false, sig) != nil {
				return "ERR"
			}
			return hex.EncodeToString(sig)
		default:
			var sk mldsa87.PrivateKey
			if sk.UnmarshalBinary(f[0]) != nil {
				return "ERR"
			}
			sig := make([]byte, mldsa87.SignatureSize)
			if mldsa87.SignTo(&sk, f[1], ctx, false, sig) != nil {
				return "ERR"
			}
			return hex.EncodeToString(sig)
		}
	case "mldsa.verify":
		f := fields(r)
		ctx := unhex(r.Args[1])
		switch r.Args[0] {
		case "ML-DSA-44":
			var pk mldsa44.PublicKey
			if pk.UnmarshalBinary(f[0]) != nil {
				return "false"
			}
			return fmt.Sprint(mldsa44.Verify(&pk, f[1], ctx, f[2]))
		case "ML-DSA-65":
			var pk mldsa65.PublicKey
			if pk.UnmarshalBinary(f[0]) != nil {
				return "false"
			}
			return fmt.Sprint(mldsa65.Verify(&pk, f[1], ctx, f[2]))
		default:
			var pk mldsa87.PublicKey
			if pk.UnmarshalBinary(f[0]) != nil {
				return "false"
			}
			return fmt.Sprint(mldsa87.Verify(&pk, f[1], ctx, f[2]))
		}
	case "slhdsa.keygen":
		id, err := slhdsa.IDByName(r.Args[0])
		if err != nil {
			return "ERR"
		}
		pub, priv, err := slhdsa.GenerateKey(bytes.NewReader(unhex(r.Hex)), id)
		if err != nil {
			return "ERR"
		}
		pb, _ := pub.MarshalBinary()
		sb, _ := priv.MarshalBinary()
		return hex.EncodeToString(pb) + "|" + hex.EncodeToString(sb)
	case "slhdsa.sign":
		f := fields(r)
		id, err := slhdsa.IDByName(r.Args[0])
		if err != nil {
			return "ERR"
		}
		priv := slhdsa.PrivateKey{ID: id}
		if priv.UnmarshalBinary(f[0]) != nil {
			return "ERR"
		}
		sig, err := slhdsa.SignDeterministic(&priv, slhdsa.NewMessage(f[1]), unhex(r.Args[1]))
		if err != nil {
			return "ERR"
		}
		return hex.EncodeToString(sig)
	case "slhdsa.verify":
		f := fields(r)
		id, err := slhdsa.IDByName(r.Args[0])
		if err != nil {
			return "false"
		}
		pub := slhdsa.PublicKey{ID: id}
		if pub.UnmarshalBinary(f[0]) != nil {
			return "false"
		}
		return fmt.Sprint(slhdsa.Verify(&pub, slhdsa.NewMessage(f[1]), f[2], unhex(r.Args[1])))
	}
	return answer6(r)
}
