package main

// The sixth part of the oracle: go.mod files (x/mod/modfile), signed notes
// and transparency logs (x/mod/sumdb/note and tlog), and coverage profiles
// (x/tools/cover).

import (
	"bytes"
	"encoding/hex"
	"fmt"
	"sort"
	"strconv"
	"strings"

	"golang.org/x/mod/modfile"
	"golang.org/x/mod/sumdb/note"
	"golang.org/x/mod/sumdb/tlog"
	"golang.org/x/tools/cover"
)

// hashStore is an in-memory tlog.HashReader over stored hashes.
type hashStore []tlog.Hash

func (s hashStore) ReadHashes(indexes []int64) ([]tlog.Hash, error) {
	out := make([]tlog.Hash, len(indexes))
	for i, x := range indexes {
		if x < 0 || x >= int64(len(s)) {
			return nil, fmt.Errorf("index %d out of range", x)
		}
		out[i] = s[x]
	}
	return out, nil
}

func storeFor(records [][]byte) (hashStore, error) {
	var s hashStore
	for i, r := range records {
		hs, err := tlog.StoredHashes(int64(i), r, s)
		if err != nil {
			return nil, err
		}
		s = append(s, hs...)
	}
	return s, nil
}

func hashes(list []tlog.Hash) string {
	var out []string
	for _, h := range list {
		out = append(out, hex.EncodeToString(h[:]))
	}
	return strings.Join(out, ",")
}

func answer6(r request) string {
	switch r.Op {
	case "modfile":
		data := unhex(r.Hex)
		f, err := modfile.Parse("go.mod", data, nil)
		if err != nil {
			return "ERR"
		}
		return modSummary(f) + "\x1f" + hex.EncodeToString(modfile.Format(f.Syntax))
	case "note.keygen":
		skey, vkey, err := note.GenerateKey(bytes.NewReader(unhex(r.Hex)), r.Args[0])
		if err != nil {
			return "ERR"
		}
		return skey + "|" + vkey
	case "note.sign":
		var signers []note.Signer
		for _, k := range r.Args {
			s, err := note.NewSigner(k)
			if err != nil {
				return "ERR"
			}
			signers = append(signers, s)
		}
		msg, err := note.Sign(&note.Note{Text: string(unhex(r.Hex))}, signers...)
		if err != nil {
			return "ERR"
		}
		return hex.EncodeToString(msg)
	case "note.open":
		var vs []note.Verifier
		for _, k := range r.Args {
			v, err := note.NewVerifier(k)
			if err != nil {
				return "ERR"
			}
			vs = append(vs, v)
		}
		n, err := note.Open(unhex(r.Hex), note.VerifierList(vs...))
		if err != nil {
			if e, ok := err.(*note.UnverifiedNoteError); ok {
				return "UNVERIFIED|" + hex.EncodeToString([]byte(e.Note.Text)) + "|" + sigNames(e.Note.UnverifiedSigs)
			}
			return "ERR"
		}
		return "OK|" + hex.EncodeToString([]byte(n.Text)) + "|" + sigNames(n.Sigs) + "|" + sigNames(n.UnverifiedSigs)
	case "tlog":
		// Hex: records, comma-separated hex; Args: pairs of t,n for proofs.
		var records [][]byte
		for _, x := range strings.Split(r.Hex, ",") {
			records = append(records, unhex(x))
		}
		store, err := storeFor(records)
		if err != nil {
			return "ERR"
		}
		var out []string
		for _, a := range r.Args {
			parts := strings.Split(a, ":")
			kind := parts[0]
			x, _ := strconv.ParseInt(parts[1], 10, 64)
			y, _ := strconv.ParseInt(parts[2], 10, 64)
			switch kind {
			case "th":
				h, err := tlog.TreeHash(x, store)
				if err != nil {
					out = append(out, "ERR")
				} else {
					out = append(out, hex.EncodeToString(h[:]))
				}
			case "rp":
				p, err := tlog.ProveRecord(x, y, store)
				if err != nil {
					out = append(out, "ERR")
				} else {
					out = append(out, hashes(p))
				}
			case "tp":
				p, err := tlog.ProveTree(x, y, store)
				if err != nil {
					out = append(out, "ERR")
				} else {
					out = append(out, hashes(p))
				}
			case "sc":
				out = append(out, fmt.Sprint(tlog.StoredHashCount(x)))
			case "si":
				out = append(out, fmt.Sprint(tlog.StoredHashIndex(int(x), y)))
			}
		}
		return strings.Join(out, "|")
	case "tlog.check":
		// Args: kind, t, n, then hex proof (comma list), th, h.
		t, _ := strconv.ParseInt(r.Args[1], 10, 64)
		n, _ := strconv.ParseInt(r.Args[2], 10, 64)
		var proof []tlog.Hash
		if r.Args[3] != "" {
			for _, x := range strings.Split(r.Args[3], ",") {
				var h tlog.Hash
				copy(h[:], unhex(x))
				proof = append(proof, h)
			}
		}
		var th, h tlog.Hash
		copy(th[:], unhex(r.Args[4]))
		copy(h[:], unhex(r.Args[5]))
		var err error
		if r.Args[0] == "record" {
			err = tlog.CheckRecord(proof, t, th, n, h)
		} else {
			err = tlog.CheckTree(proof, t, th, n, h)
		}
		return fmt.Sprint(err == nil)
	case "tlog.tile":
		var out []string
		for _, a := range r.Args {
			parts := strings.Split(a, ":")
			switch parts[0] {
			case "for":
				h, _ := strconv.Atoi(parts[1])
				idx, _ := strconv.ParseInt(parts[2], 10, 64)
				out = append(out, tlog.TileForIndex(h, idx).Path())
			case "new":
				h, _ := strconv.Atoi(parts[1])
				o, _ := strconv.ParseInt(parts[2], 10, 64)
				n, _ := strconv.ParseInt(parts[3], 10, 64)
				var ps []string
				for _, t := range tlog.NewTiles(h, o, n) {
					ps = append(ps, t.Path())
				}
				out = append(out, strings.Join(ps, " "))
			case "parse":
				t, err := tlog.ParseTilePath(parts[1])
				if err != nil {
					out = append(out, "ERR")
				} else {
					out = append(out, fmt.Sprintf("%d/%d/%d/%d", t.H, t.L, t.N, t.W))
				}
			}
		}
		return strings.Join(out, "|")
	case "tree":
		t, err := tlog.ParseTree(unhex(r.Hex))
		if err != nil {
			return "ERR"
		}
		return fmt.Sprintf("%d|%s|%s", t.N, hex.EncodeToString(t.Hash[:]), hex.EncodeToString(tlog.FormatTree(t)))
	case "cover":
		ps, err := cover.ParseProfilesFromReader(bytes.NewReader(unhex(r.Hex)))
		if err != nil {
			return "ERR"
		}
		var out []string
		for _, p := range ps {
			var bs []string
			for _, b := range p.Blocks {
				bs = append(bs, fmt.Sprintf("%d.%d,%d.%d:%d:%d", b.StartLine, b.StartCol, b.EndLine, b.EndCol, b.NumStmt, b.Count))
			}
			out = append(out, p.FileName+"|"+p.Mode+"|"+strings.Join(bs, ";"))
		}
		return strings.Join(out, "\n")
	}
	return "UNKNOWN"
}

func sigNames(sigs []note.Signature) string {
	var out []string
	for _, s := range sigs {
		out = append(out, fmt.Sprintf("%s+%08x", s.Name, s.Hash))
	}
	sort.Strings(out)
	return strings.Join(out, ",")
}

func modSummary(f *modfile.File) string {
	var parts []string
	mod, dep := "", ""
	if f.Module != nil {
		mod, dep = f.Module.Mod.Path, f.Module.Deprecated
	}
	goV, tc := "", ""
	if f.Go != nil {
		goV = f.Go.Version
	}
	if f.Toolchain != nil {
		tc = f.Toolchain.Name
	}
	parts = append(parts, "module="+mod, "deprecated="+dep, "go="+goV, "toolchain="+tc)
	var g []string
	for _, x := range f.Godebug {
		g = append(g, x.Key+"="+x.Value)
	}
	parts = append(parts, "godebug="+strings.Join(g, ","))
	var req []string
	for _, x := range f.Require {
		s := x.Mod.Path + "@" + x.Mod.Version
		if x.Indirect {
			s += "!"
		}
		req = append(req, s)
	}
	parts = append(parts, "require="+strings.Join(req, ","))
	var ex []string
	for _, x := range f.Exclude {
		ex = append(ex, x.Mod.Path+"@"+x.Mod.Version)
	}
	parts = append(parts, "exclude="+strings.Join(ex, ","))
	var rep []string
	for _, x := range f.Replace {
		rep = append(rep, x.Old.Path+"@"+x.Old.Version+"=>"+x.New.Path+"@"+x.New.Version)
	}
	parts = append(parts, "replace="+strings.Join(rep, ","))
	var ret []string
	for _, x := range f.Retract {
		ret = append(ret, x.Low+".."+x.High+"#"+x.Rationale)
	}
	parts = append(parts, "retract="+strings.Join(ret, ","))
	var tools, ign []string
	for _, x := range f.Tool {
		tools = append(tools, x.Path)
	}
	for _, x := range f.Ignore {
		ign = append(ign, x.Path)
	}
	parts = append(parts, "tool="+strings.Join(tools, ","), "ignore="+strings.Join(ign, ","))
	return strings.Join(parts, "\x1e")
}
