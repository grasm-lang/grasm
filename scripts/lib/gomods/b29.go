package main

// The fourth part of the oracle: X448, Ed448 and HPKE (cloudflare/circl)
// and the age file format (metacubex/age, a fork of filippo.io/age).

import (
	"bytes"
	"encoding/hex"
	"fmt"
	"io"
	"strconv"
	"strings"

	"github.com/cloudflare/circl/dh/x448"
	"github.com/cloudflare/circl/hpke"
	"github.com/cloudflare/circl/sign/ed448"
	"github.com/metacubex/age"
	"github.com/metacubex/age/armor"
)

func unhex(s string) []byte {
	b, _ := hex.DecodeString(s)
	return b
}

// fields splits Hex at "|" into byte strings.
func fields(r request) [][]byte {
	var out [][]byte
	for _, f := range strings.Split(r.Hex, "|") {
		out = append(out, unhex(f))
	}
	return out
}

func answer4(r request) string {
	switch r.Op {
	case "x448":
		f := fields(r)
		var sk, pk, ss x448.Key
		copy(sk[:], f[0])
		copy(pk[:], f[1])
		ok := x448.Shared(&ss, &sk, &pk)
		return hex.EncodeToString(ss[:]) + "|" + fmt.Sprint(ok)
	case "x448.pub":
		var sk, pk x448.Key
		copy(sk[:], unhex(r.Hex))
		x448.KeyGen(&pk, &sk)
		return hex.EncodeToString(pk[:])
	case "ed448.pub":
		priv := ed448.NewKeyFromSeed(unhex(r.Hex))
		return hex.EncodeToString(priv.Public().(ed448.PublicKey))
	case "ed448.sign":
		f := fields(r)
		priv := ed448.NewKeyFromSeed(f[0])
		ctx := string(unhex(r.Args[0]))
		if r.Args[1] == "ph" {
			return hex.EncodeToString(ed448.SignPh(priv, f[1], ctx))
		}
		return hex.EncodeToString(ed448.Sign(priv, f[1], ctx))
	case "ed448.verify":
		f := fields(r)
		ctx := string(unhex(r.Args[0]))
		if len(f[0]) != ed448.PublicKeySize {
			return "false"
		}
		if r.Args[1] == "ph" {
			return fmt.Sprint(ed448.VerifyPh(ed448.PublicKey(f[0]), f[1], f[2], ctx))
		}
		return fmt.Sprint(ed448.Verify(ed448.PublicKey(f[0]), f[1], f[2], ctx))
	case "hpke.seal":
		return hpkeSeal(r)
	case "hpke.open":
		return hpkeOpen(r)
	case "age.encrypt":
		return ageEncrypt(r)
	case "age.decrypt":
		return ageDecrypt(r)
	case "age.keygen":
		id, err := age.GenerateX25519Identity()
		if err != nil {
			return "ERR"
		}
		return id.String() + "|" + id.Recipient().String()
	}
	return answer5(r)
}

func suiteOf(r request) (hpke.Suite, hpke.KEM, int) {
	k, _ := strconv.Atoi(r.Args[0])
	d, _ := strconv.Atoi(r.Args[1])
	a, _ := strconv.Atoi(r.Args[2])
	mode, _ := strconv.Atoi(r.Args[3])
	return hpke.NewSuite(hpke.KEM(k), hpke.KDF(d), hpke.AEAD(a)), hpke.KEM(k), mode
}

// hpkeSeal: Hex is ikmR|ikmE|ikmS|info|psk|pskID|aad|pt|exporterContext,
// Args are kem, kdf, aead, mode, export length. The answer is the
// receiver's public key, enc, two ciphertexts of pt (sequence numbers 0
// and 1) and the exported secret.
func hpkeSeal(r request) string {
	f := fields(r)
	suite, kemID, mode := suiteOf(r)
	scheme := kemID.Scheme()
	pkR, _ := scheme.DeriveKeyPair(f[0])
	_, skS := scheme.DeriveKeyPair(f[2])
	sender, err := suite.NewSender(pkR, f[3])
	if err != nil {
		return "ERR " + err.Error()
	}
	var enc []byte
	var sealer hpke.Sealer
	rnd := bytes.NewReader(f[1])
	switch mode {
	case 0:
		enc, sealer, err = sender.Setup(rnd)
	case 1:
		enc, sealer, err = sender.SetupPSK(rnd, f[4], f[5])
	case 2:
		enc, sealer, err = sender.SetupAuth(rnd, skS)
	default:
		enc, sealer, err = sender.SetupAuthPSK(rnd, skS, f[4], f[5])
	}
	if err != nil {
		return "ERR " + err.Error()
	}
	pkb, _ := pkR.MarshalBinary()
	n, _ := strconv.Atoi(r.Args[4])
	ct1, err1 := sealer.Seal(f[7], f[6])
	ct2, _ := sealer.Seal(f[7], f[6])
	exp := sealer.Export(f[8], uint(n))
	c1, c2 := hex.EncodeToString(ct1), hex.EncodeToString(ct2)
	if err1 != nil {
		c1, c2 = "-", "-"
	}
	return strings.Join([]string{hex.EncodeToString(pkb), hex.EncodeToString(enc), c1, c2, hex.EncodeToString(exp)}, "|")
}

// hpkeOpen: Hex is ikmR|ikmS|enc|info|psk|pskID|aad|ct, Args as for seal.
func hpkeOpen(r request) string {
	f := fields(r)
	suite, kemID, mode := suiteOf(r)
	scheme := kemID.Scheme()
	_, skR := scheme.DeriveKeyPair(f[0])
	pkS, _ := scheme.DeriveKeyPair(f[1])
	recv, err := suite.NewReceiver(skR, f[3])
	if err != nil {
		return "ERR"
	}
	var opener hpke.Opener
	switch mode {
	case 0:
		opener, err = recv.Setup(f[2])
	case 1:
		opener, err = recv.SetupPSK(f[2], f[4], f[5])
	case 2:
		opener, err = recv.SetupAuth(f[2], pkS)
	default:
		opener, err = recv.SetupAuthPSK(f[2], f[4], f[5], pkS)
	}
	if err != nil {
		return "ERR"
	}
	pt, err := opener.Open(f[7], f[6])
	if err != nil {
		return "ERR"
	}
	return hex.EncodeToString(pt)
}

// ageEncrypt: Args are recipients (age1... or scrypt:password:logN), then
// "armor" or "binary"; Hex is the plaintext.
func ageEncrypt(r request) string {
	var recips []age.Recipient
	for _, a := range r.Args[1:] {
		if strings.HasPrefix(a, "scrypt:") {
			parts := strings.SplitN(a, ":", 3)
			s, err := age.NewScryptRecipient(parts[1])
			if err != nil {
				return "ERR"
			}
			n, _ := strconv.Atoi(parts[2])
			s.SetWorkFactor(n)
			recips = append(recips, s)
		} else {
			x, err := age.ParseX25519Recipient(a)
			if err != nil {
				return "ERR"
			}
			recips = append(recips, x)
		}
	}
	var buf bytes.Buffer
	var dst io.Writer = &buf
	var aw io.WriteCloser
	if r.Args[0] == "armor" {
		aw = armor.NewWriter(&buf)
		dst = aw
	}
	w, err := age.Encrypt(dst, recips...)
	if err != nil {
		return "ERR " + err.Error()
	}
	w.Write(unhex(r.Hex))
	w.Close()
	if aw != nil {
		aw.Close()
	}
	return hex.EncodeToString(buf.Bytes())
}

// ageDecrypt: Args are identities (AGE-SECRET-KEY-1... or scrypt:password);
// Hex is the file, armored or not.
func ageDecrypt(r request) string {
	var ids []age.Identity
	for _, a := range r.Args {
		if strings.HasPrefix(a, "scrypt:") {
			s, err := age.NewScryptIdentity(strings.TrimPrefix(a, "scrypt:"))
			if err != nil {
				return "ERR"
			}
			s.SetMaxWorkFactor(22)
			ids = append(ids, s)
		} else {
			x, err := age.ParseX25519Identity(a)
			if err != nil {
				return "ERR"
			}
			ids = append(ids, x)
		}
	}
	data := unhex(r.Hex)
	var src io.Reader = bytes.NewReader(data)
	if bytes.HasPrefix(bytes.TrimLeft(data, " \t\r\n"), []byte("-----BEGIN AGE ENCRYPTED FILE-----")) {
		src = armor.NewReader(src)
	}
	out, err := age.Decrypt(src, ids...)
	if err != nil {
		return "ERR"
	}
	pt, err := io.ReadAll(out)
	if err != nil {
		return "ERR"
	}
	return hex.EncodeToString(pt)
}
