// The oracle for packages/gojson: Go's encoding/json. Each input line is an
// operation and hex arguments; each answer is one line.
package main

import (
	"bufio"
	"bytes"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"os"
	"sort"
	"strconv"
	"strings"
)

func unhex(h string) []byte {
	if h == "-" {
		return nil
	}
	b, _ := hex.DecodeString(h)
	return b
}

func hx(b []byte) string {
	if len(b) == 0 {
		return "-"
	}
	return hex.EncodeToString(b)
}

func errText(err error) string {
	var se *json.SyntaxError
	var ue *json.UnmarshalTypeError
	var ve *json.UnsupportedValueError
	switch {
	case errors.As(err, &se):
		return "S" + hx([]byte(se.Error())) + ":" + strconv.FormatInt(se.Offset, 10)
	case errors.As(err, &ue):
		return "U" + hx([]byte(ue.Error()))
	case errors.As(err, &ve):
		return "V" + hx([]byte(ve.Error()))
	}
	return "?" + hx([]byte(err.Error()))
}

func dump(v any) string {
	switch t := v.(type) {
	case nil:
		return "n"
	case bool:
		if t {
			return "t"
		}
		return "f"
	case float64:
		return "d" + strconv.FormatUint(math.Float64bits(t), 16)
	case string:
		return "s" + hx([]byte(t))
	case []any:
		parts := make([]string, len(t))
		for i, x := range t {
			parts[i] = dump(x)
		}
		return "[" + strings.Join(parts, ",") + "]"
	case map[string]any:
		keys := make([]string, 0, len(t))
		for k := range t {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		parts := make([]string, len(keys))
		for i, k := range keys {
			parts[i] = "s" + hx([]byte(k)) + "=" + dump(t[k])
		}
		return "{" + strings.Join(parts, ",") + "}"
	}
	return "?"
}

func marshal(v any, html bool) ([]byte, error) {
	if html {
		return json.Marshal(v)
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(v); err != nil {
		return nil, err
	}
	return bytes.TrimSuffix(buf.Bytes(), []byte("\n")), nil
}

func answer(out []byte, err error) string {
	if err != nil {
		return errText(err)
	}
	return hx(out)
}

func line(f []string) string {
	switch f[0] {
	case "va":
		return strconv.FormatBool(json.Valid(unhex(f[1])))
	case "co":
		var buf bytes.Buffer
		err := json.Compact(&buf, unhex(f[1]))
		return answer(buf.Bytes(), err)
	case "in":
		var buf bytes.Buffer
		err := json.Indent(&buf, unhex(f[1]), string(unhex(f[2])), string(unhex(f[3])))
		return answer(buf.Bytes(), err)
	case "he":
		var buf bytes.Buffer
		json.HTMLEscape(&buf, unhex(f[1]))
		return hx(buf.Bytes())
	case "un":
		var v any
		err := json.Unmarshal(unhex(f[1]), &v)
		e := "-"
		if err != nil {
			e = errText(err)
		}
		return dump(v) + "|" + e
	case "ma", "mi":
		var v any
		if err := json.Unmarshal(unhex(f[1]), &v); err != nil {
			var ue *json.UnmarshalTypeError
			if !errors.As(err, &ue) {
				return "X"
			}
		}
		if f[0] == "mi" {
			return answer(json.MarshalIndent(v, string(unhex(f[2])), string(unhex(f[3]))))
		}
		return answer(marshal(v, f[2] == "1"))
	case "nu":
		bits, _ := strconv.ParseUint(f[1], 16, 64)
		return answer(marshal(math.Float64frombits(bits), true))
	case "st":
		return answer(marshal(string(unhex(f[1])), f[2] == "1"))
	}
	return "?"
}

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<20), 1<<26)
	w := bufio.NewWriter(os.Stdout)
	defer w.Flush()
	for in.Scan() {
		if t := in.Text(); t != "" {
			fmt.Fprintln(w, line(strings.Split(t, " ")))
		}
	}
}
