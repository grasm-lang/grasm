# 09 — Password Generator

**Tests:** character-set construction, seeded lookups, string building, and
whether a generated password satisfies the policy it came from.

**Result:** works. All three engines agree.

## What it actually exercises

The seed is what makes this testable. The same seed must give the same password
under all three engines — a much stronger claim than "it produced something
random-looking". If the interpreter and the Cranelift build disagreed here, the
arithmetic in the generator differs between them, and a program that printed one
password would never reveal it.

The last section is the part worth keeping. Uniform sampling does **not**
guarantee one character of each class, so over 200 seeds it counts how often a
password actually satisfies its own policy:

* length 16, all four classes present: **172 / 200**
* length 4, all four classes present: **8 / 200**

That is the number a real generator uses to decide whether to reject and
resample, and it is invisible unless you measure it.

## Findings

None new. Confirmed F1 again (wrapped boolean expression needs trailing
operators). Noted: `{` opens an interpolation even in an ordinary string, so a
literal brace is `\{` — the diagnostic says exactly that, which is why it is a
note and not a finding.
