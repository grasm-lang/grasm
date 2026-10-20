"""SBCL 2.6.8's PRIN1-TO-STRING and PRINC-TO-STRING, one call per line:
  <prin1|princ> <*print-base*> <*print-radix* 0|1> <double-float default 0|1> <kind> <data...>
kinds: int <decimal>, ratio <num> <den>, single <32-bit hex>, double <64-bit hex>,
string <UTF-8 hex>, char <code>. Every call goes into one Lisp program run by
sbcl; each answer is the printed text's UTF-8 in hex, or E:sbcl-error when
SBCL signals an error (2.6.8 does for most-negative-fixnum, -2^62, in base 2:
"Should not happen")."""
import os
import subprocess
import sys
import tempfile

lines = sys.stdin.read().split("\n")
if lines and lines[-1] == "":
    lines.pop()


def signed(v, bits):
    return v - (1 << bits) if v >= 1 << (bits - 1) else v


def obj(kind, data):
    if kind == "int":
        return data[0]
    if kind == "ratio":
        return f"(/ {data[0]} {data[1]})"
    if kind == "single":
        return f"(sb-kernel:make-single-float {signed(int(data[0], 16), 32)})"
    if kind == "double":
        v = int(data[0], 16)
        return f"(sb-kernel:make-double-float {signed(v >> 32, 32)} {v & 0xffffffff})"
    if kind == "string":
        codes = [ord(c) for c in (bytes.fromhex(data[0]).decode() if data[0] != "-" else "")]
        return "(map 'string #'code-char '(" + " ".join(map(str, codes)) + "))"
    return f"(code-char {data[0]})"


forms = []
for line in lines:
    f = line.split(" ")
    fn = "prin1-to-string" if f[0] == "prin1" else "princ-to-string"
    fmt = "'double-float" if f[3] == "1" else "'single-float"
    forms.append(f"(out (handler-case (let ((*print-base* {f[1]}) (*print-radix* {'t' if f[2] == '1' else 'nil'}) (*read-default-float-format* {fmt})) "
                 f"({fn} {obj(f[4], f[5:])})) (error () :error)))")

prelude = """(declaim (sb-ext:muffle-conditions warning))
(setf sb-impl::*default-external-format* :utf-8)
(defun out (s)
  (when (eq s :error) (write-line "E:sbcl-error") (return-from out))
  (let ((o (sb-ext:string-to-octets s :external-format :utf-8)))
    (if (zerop (length o)) (write-string "-") (loop for b across o do (format t "~(~2,'0x~)" b)))
    (terpri)))
"""
work = tempfile.mkdtemp(prefix="lispprint")
path = os.path.join(work, "cases.lisp")
with open(path, "w") as h:
    h.write(prelude + "\n".join(forms) + "\n")
r = subprocess.run(["sbcl", "--script", path], capture_output=True, text=True)
if r.returncode != 0:
    sys.stderr.write(r.stderr[-3000:])
    sys.exit(1)
sys.stdout.write(r.stdout)
