"""Graph algorithms against brute force.

Each package answers on small random graphs -- with parallel edges,
self-loops, isolated vertices and the structured cases (paths, cycles,
complete graphs, trees) where algorithms break -- and each answer is held
to an implementation here that is slow, written independently in another
language, and obviously right: reachability by search from every vertex,
a bridge as an edge whose removal splits a component, every subset for
cliques, every permutation for isomorphism, every colouring for the
chromatic number, every integer flow for minimum-cost flow.

Where the answer is not unique (an Euler walk, an isomorphism, a flow) the
answer is checked for being one, and the brute force decides whether one
exists.
"""
import itertools
import os
import random
import subprocess
import sys
import tempfile

GRASM = os.environ.get("GRASM", "./target/release/grasm")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
rng = random.Random(20261018)
work = tempfile.mkdtemp(prefix="graphdiff")
harness = os.path.join(work, "graphtest")
built = subprocess.run([GRASM, "build", "packages/graphtest/src/main.gr", "--native", "-o", harness],
                       cwd=ROOT, capture_output=True, text=True)
if built.returncode != 0:
    print(built.stderr[-1500:]); sys.exit(1)

cases = []


def add(op, a, b, check):
    cases.append((op, a, b, check))


def text(n, edges):
    return f"{n};" + ",".join(f"{a}-{b}" for a, b in edges)


def random_edges(n, m, loops=True, parallel=True):
    out = []
    for _ in range(m):
        a, b = rng.randrange(n), rng.randrange(n)
        if a == b and not loops:
            continue
        if not parallel and ((a, b) in out or (b, a) in out):
            continue
        out.append((a, b))
    return out


def shapes(n):
    """Paths, cycles, stars and complete graphs on n vertices."""
    path = [(k, k + 1) for k in range(n - 1)]
    cycle = path + ([(n - 1, 0)] if n > 2 else [])
    star = [(0, k) for k in range(1, n)]
    complete = [(a, b) for a in range(n) for b in range(a + 1, n)]
    return [path, cycle, star, complete]


def ints(s):
    return [int(x) for x in s.split(",")] if s else []


def components(n, edges, removed_vertex=-1, removed_edge=-1):
    adj = [[] for _ in range(n)]
    for k, (a, b) in enumerate(edges):
        if k != removed_edge and removed_vertex not in (a, b):
            adj[a].append(b)
            adj[b].append(a)
    label, count = [-1] * n, 0
    for s in range(n):
        if s == removed_vertex or label[s] != -1:
            continue
        label[s], todo = count, [s]
        while todo:
            v = todo.pop()
            for w in adj[v]:
                if label[w] == -1:
                    label[w] = count
                    todo.append(w)
        count += 1
    return label, count


# --- strongly connected components ------------------------------------------
def check_scc(n, edges, got):
    labels_s, arcs_s, strong = got.split(";")
    lab = ints(labels_s)
    reach = []
    for s in range(n):
        seen, todo = {s}, [s]
        while todo:
            v = todo.pop()
            for a, b in edges:
                if a == v and b not in seen:
                    seen.add(b)
                    todo.append(b)
        reach.append(seen)
    if len(lab) != n:
        return "wrong length"
    for u in range(n):
        for v in range(n):
            if (lab[u] == lab[v]) != (v in reach[u] and u in reach[v]):
                return f"{u} and {v} wrongly {'together' if lab[u] == lab[v] else 'apart'}"
    k = len(set(lab))
    if sorted(set(lab)) != list(range(k)):
        return "components not numbered 0..k-1"
    for a, b in edges:
        if lab[a] < lab[b]:
            return f"edge {a}->{b} leads to a later component"
    want = {}
    for a, b in edges:
        if lab[a] != lab[b]:
            want[(lab[a], lab[b])] = want.get((lab[a], lab[b]), 0) + 1
    want_arcs = sorted(f"{c}-{d}-{m}" for (c, d), m in want.items())
    if sorted(arcs_s.split(",") if arcs_s else []) != want_arcs:
        return f"condensation {arcs_s} != {want_arcs}"
    if strong != str(k <= 1).lower():
        return "strongly connected wrong"
    return None


for _ in range(160):
    n = rng.randrange(1, 9)
    edges = random_edges(n, rng.randrange(0, 2 * n + 2))
    add("scc", text(n, edges), "", lambda got, n=n, e=edges: check_scc(n, e, got))
for n in range(1, 7):
    for sh in shapes(n):
        add("scc", text(n, sh), "", lambda got, n=n, e=sh: check_scc(n, e, got))


# --- bridges and articulation points ----------------------------------------
def brute_bridges(n, edges):
    _, base = components(n, edges)
    bridges = [k for k in range(len(edges)) if components(n, edges, removed_edge=k)[1] > base]
    lab, _ = components(n, edges)
    points = []
    for v in range(n):
        alone = all(lab[w] != lab[v] for w in range(n) if w != v)
        after = components(n, edges, removed_vertex=v)[1]
        if not alone and after > base:
            points.append(v)
    kept = [e for k, e in enumerate(edges) if k not in bridges]
    two, _ = components(n, kept)
    return ",".join(map(str, bridges)) + ";" + ",".join(map(str, points)) + ";" + ",".join(map(str, two))


for _ in range(200):
    n = rng.randrange(1, 10)
    edges = random_edges(n, rng.randrange(0, 2 * n))
    want = brute_bridges(n, edges)
    add("bridges", text(n, edges), "", lambda got, w=want: None if got == w else f"want {w}")
for n in range(1, 8):
    for sh in shapes(n):
        want = brute_bridges(n, sh)
        add("bridges", text(n, sh), "", lambda got, w=want: None if got == w else f"want {w}")


# --- Euler walks ---------------------------------------------------------------
def euler_exists(n, edges, directed, circuit):
    m = len(edges)
    if m == 0:
        return True
    used = [False] * m

    def go(v, left, start):
        if left == 0:
            return not circuit or v == start
        for k, (a, b) in enumerate(edges):
            if used[k]:
                continue
            nxt = b if a == v else (a if (not directed and b == v) else None)
            if nxt is None:
                continue
            used[k] = True
            if go(nxt, left - 1, start):
                return True
            used[k] = False
        return False

    return any(go(s, m, s) for s in range(n))


def check_euler(n, edges, directed, got):
    if got == "none":
        return "a walk exists" if euler_exists(n, edges, directed, False) else None
    vs_s, es_s, circ = got.split(";")
    vs, es = ints(vs_s), ints(es_s)
    m = len(edges)
    if sorted(es) != list(range(m)) or len(vs) != (m + 1 if m else 0):
        return "not every edge once"
    for i, k in enumerate(es):
        a, b = edges[k]
        if not ((vs[i], vs[i + 1]) == (a, b) or (not directed and (vs[i], vs[i + 1]) == (b, a))):
            return f"step {i} does not follow edge {k}"
    is_circ = m > 0 and vs[0] == vs[-1]
    if circ != str(is_circ).lower():
        return "is_circuit wrong"
    if not is_circ and m > 0 and euler_exists(n, edges, directed, True):
        return "a circuit exists but a path came back"
    return None


for directed in (False, True):
    op = "euler.d" if directed else "euler.u"
    for _ in range(170):
        n = rng.randrange(1, 6)
        edges = random_edges(n, rng.randrange(0, 8))
        add(op, text(n, edges), "", lambda got, n=n, e=edges, d=directed: check_euler(n, e, d, got))
    for n in range(1, 6):
        for sh in shapes(n):
            if len(sh) <= 9:
                add(op, text(n, sh), "", lambda got, n=n, e=sh, d=directed: check_euler(n, e, d, got))


# --- lowest common ancestors --------------------------------------------------
def brute_lca(parent, queries):
    n = len(parent)

    def chain(v):
        out = [v]
        while parent[out[-1]] != -1:
            out.append(parent[out[-1]])
            if len(out) > n:
                return None
        return out

    for v in range(n):
        if chain(v) is None:
            return "ERR"
    answers = []
    for a, b, k in queries:
        ca, cb = chain(a), chain(b)
        common = next((x for x in cb if x in ca), -1)
        dist = -1 if common == -1 else ca.index(common) + cb.index(common)
        anc = ca[k] if 0 <= k < len(ca) else -1
        answers.append(f"{common}/{dist}/{anc}/{len(ca) - 1}")
    return ",".join(answers)


for _ in range(200):
    n = rng.randrange(1, 40)
    parent = [-1] + [rng.randrange(-1, v) if rng.random() < 0.1 else rng.randrange(0, v) for v in range(1, n)]
    order = list(range(n))
    rng.shuffle(order)
    rename = {old: new for new, old in enumerate(order)}
    shuffled = [0] * n
    for v in range(n):
        shuffled[rename[v]] = -1 if parent[v] == -1 else rename[parent[v]]
    if rng.random() < 0.1 and n > 1:
        shuffled[rng.randrange(n)] = rng.randrange(n)
    qs = [(rng.randrange(n), rng.randrange(n), rng.randrange(-1, 8)) for _ in range(6)]
    want = brute_lca(shuffled, qs)
    a = ",".join(map(str, shuffled)) + ";" + ",".join(f"{x}:{y}:{k}" for x, y, k in qs)
    add("lca", a, "", lambda got, w=want: None if got == w else f"want {w}")


# --- colouring ----------------------------------------------------------------
def dsatur(n, edges):
    adj = [[] for _ in range(n)]
    for a, b in edges:
        adj[a].append(b)
        adj[b].append(a)
    colour = [-1] * n
    for _ in range(n):
        best, best_key = -1, None
        for v in range(n):
            if colour[v] >= 0:
                continue
            key = (len({colour[w] for w in adj[v] if colour[w] >= 0}), len(adj[v]))
            if best_key is None or key > best_key:
                best, best_key = v, key
        used = {colour[w] for w in adj[best] if w != best}
        c = 0
        while c in used:
            c += 1
        colour[best] = c
    return colour


def chromatic(n, edges):
    if any(a == b for a, b in edges):
        return -1
    for k in range(0, n + 1):
        if k == 0 and n > 0:
            continue
        for col in itertools.product(range(k), repeat=n):
            if all(col[a] != col[b] for a, b in edges):
                return k
    return n


def check_colour(n, edges, got):
    ds, chi, best = got.split(";")
    want_ds = ",".join(map(str, dsatur(n, edges)))
    if ds != want_ds:
        return f"dsatur {ds} != {want_ds}"
    want = chromatic(n, edges)
    if int(chi) != want:
        return f"chromatic {chi} != {want}"
    if want == -1:
        return None if best == "none" else "coloured a graph with a self-loop"
    col = ints(best)
    if len(col) != n or any(col[a] == col[b] for a, b in edges) or (max(col) + 1 if col else 0) != want:
        return "optimal colouring wrong"
    return None


for _ in range(180):
    n = rng.randrange(0, 8)
    edges = random_edges(n, rng.randrange(0, 2 * n + 1), loops=rng.random() < 0.15) if n else []
    add("colour", text(n, edges), "", lambda got, n=n, e=edges: check_colour(n, e, got))
for n in range(1, 8):
    for sh in shapes(n):
        add("colour", text(n, sh), "", lambda got, n=n, e=sh: check_colour(n, e, got))


# --- cliques ------------------------------------------------------------------
def brute_cliques(n, edges):
    adj = {(a, b) for a, b in edges if a != b} | {(b, a) for a, b in edges if a != b}
    cl = [list(s) for r in range(n + 1) for s in itertools.combinations(range(n), r)
          if all((a, b) in adj for a, b in itertools.combinations(s, 2))]
    cs = [set(c) for c in cl]
    maximal = sorted(c for c in cl if not any(set(c) < d for d in cs))
    size = max(len(c) for c in maximal)
    biggest = next(c for c in maximal if len(c) == size)
    return ("|".join(".".join(map(str, c)) for c in maximal) + ";" + ",".join(map(str, biggest)) + f";{size}")


for _ in range(160):
    n = rng.randrange(0, 11)
    edges = random_edges(n, rng.randrange(0, 3 * n + 1)) if n else []
    want = brute_cliques(n, edges)
    add("cliques", text(n, edges), "", lambda got, w=want: None if got == w else f"want {w}")
for n in range(1, 8):
    for sh in shapes(n):
        want = brute_cliques(n, sh)
        add("cliques", text(n, sh), "", lambda got, w=want: None if got == w else f"want {w}")


# --- isomorphism --------------------------------------------------------------
def matrix(n, edges):
    m = [[0] * n for _ in range(n)]
    for a, b in edges:
        m[a][b] += 1
        if a != b:
            m[b][a] += 1
    return m


def check_iso(n1, e1, n2, e2, got):
    exists = n1 == n2 and len(e1) == len(e2) and any(
        all(matrix(n1, e1)[a][b] == matrix(n2, e2)[p[a]][p[b]] for a in range(n1) for b in range(n1))
        for p in itertools.permutations(range(n2)))
    if got == "none":
        return "an isomorphism exists" if exists else None
    p = ints(got)
    m1, m2 = matrix(n1, e1), matrix(n2, e2)
    if sorted(p) != list(range(n2)) or any(m1[a][b] != m2[p[a]][p[b]] for a in range(n1) for b in range(n1)):
        return "not an isomorphism"
    return None


for _ in range(220):
    n = rng.randrange(0, 7)
    e1 = random_edges(n, rng.randrange(0, 2 * n + 1)) if n else []
    perm = list(range(n))
    rng.shuffle(perm)
    e2 = [(perm[a], perm[b]) if rng.random() < 0.5 else (perm[b], perm[a]) for a, b in e1]
    rng.shuffle(e2)
    if e2 and rng.random() < 0.4:
        k = rng.randrange(len(e2))
        e2[k] = (rng.randrange(n), rng.randrange(n))
    n2 = n + (1 if rng.random() < 0.05 else 0)
    add("iso", text(n, e1), text(n2, e2), lambda got, a=(n, e1, n2, e2): check_iso(*a, got))


# --- minimum-cost flow ------------------------------------------------------
def negative_cycle(n, arcs):
    inf = float("inf")
    d = [[0 if i == j else inf for j in range(n)] for i in range(n)]
    for a, b, cap, cost in arcs:
        if cap > 0:
            d[a][b] = min(d[a][b], cost)
    for k in range(n):
        for i in range(n):
            for j in range(n):
                if d[i][k] + d[k][j] < d[i][j]:
                    d[i][j] = d[i][k] + d[k][j]
    return any(d[i][i] < 0 for i in range(n))


def check_flow(n, arcs, s, t, limit, got):
    if negative_cycle(n, arcs):
        return None if got == "NEGCYCLE" else "a negative cycle went unnoticed"
    best = {}
    for f in itertools.product(*[range(c + 1) for _, _, c, _ in arcs]):
        net = [0] * n
        for (a, b, _, _), x in zip(arcs, f):
            net[a] += x
            net[b] -= x
        if any(net[v] for v in range(n) if v not in (s, t)) or net[s] < 0:
            continue
        cost = sum(x * c for (_, _, _, c), x in zip(arcs, f))
        best[net[s]] = min(best.get(net[s], cost), cost)
    value = min(limit, max(best))
    parts = got.split(";")
    if len(parts) != 3:
        return f"unexpected {got}"
    flow, cost, fl = int(parts[0]), int(parts[1]), ints(parts[2])
    if flow != value or cost != best[value]:
        return f"flow {flow} cost {cost}, want {value} at {best[value]}"
    net = [0] * n
    for (a, b, cap, _), x in zip(arcs, fl):
        if not 0 <= x <= cap:
            return "arc flow out of range"
        net[a] += x
        net[b] -= x
    if any(net[v] for v in range(n) if v not in (s, t)) or net[s] != flow:
        return "arc flows do not conserve"
    if sum(x * c for (_, _, _, c), x in zip(arcs, fl)) != cost:
        return "arc flows do not cost what was reported"
    return None


for _ in range(170):
    n = rng.randrange(2, 6)
    arcs = []
    for _ in range(rng.randrange(1, 8)):
        a, b = rng.randrange(n), rng.randrange(n)
        if a != b:
            low = -3 if rng.random() < 0.3 else 0
            arcs.append((a, b, rng.randrange(0, 4), rng.randrange(low, 6)))
    s, t = rng.sample(range(n), 2)
    limit = rng.choice([1, 2, 3, 100])
    a = f"{n};" + ",".join(f"{x}:{y}:{c}:{w}" for x, y, c, w in arcs)
    add("mcf", a, f"{s},{t},{limit}", lambda got, args=(n, arcs, s, t, limit): check_flow(*args, got))

# A cycle that costs less than nothing, sometimes where the source cannot
# reach it: the answer has to be a refusal either way.
for _ in range(30):
    n = rng.randrange(4, 6)
    a, b = rng.sample(range(2, n), 2)
    arcs = [(0, 1, rng.randrange(1, 3), rng.randrange(0, 4)), (a, b, 1, -3), (b, a, 1, rng.randrange(0, 3))]
    if rng.random() < 0.5:
        arcs.append((1, a, 1, 1))
    text_arcs = f"{n};" + ",".join(f"{x}:{y}:{c}:{w}" for x, y, c, w in arcs)
    add("mcf", text_arcs, "0,1,5", lambda got, args=(n, arcs, 0, 1, 5): check_flow(*args, got))


def run(picked):
    argv = []
    for op, a, b, _ in picked:
        argv += [op, a, b]
    lines = []
    for start in range(0, len(argv), 3 * 500):
        ran = subprocess.run([harness] + argv[start:start + 1500], capture_output=True, text=True)
        if ran.returncode != 0:
            print(ran.stderr[-2000:]); sys.exit(1)
        lines += ran.stdout.split("\n")[:-1]
    return lines


got = run(cases)
families = {}
agree = differ = 0
for i, (op, a, b, check) in enumerate(cases):
    mine = got[i] if i < len(got) else "<missing>"
    why = check(mine) if mine != "<missing>" else "missing"
    fam = families.setdefault(op, [0, 0])
    if why is None:
        agree += 1
        fam[0] += 1
    else:
        differ += 1
        fam[1] += 1
        if differ <= 10:
            print(f"  {op} {a} {b}")
            print(f"    grasm {mine[:200]}")
            print(f"    {why[:300]}")
bad = [f"{k}:{d}" for k, (a, d) in sorted(families.items()) if d]
if bad:
    print("disagreeing:", " ".join(bad))
thin = [k for k, (a, d) in families.items() if a + d < 100]
if thin or len(families) < 9:
    print(f"too few cases to mean anything: {thin}")
    sys.exit(1)
print(f"cases: {len(cases)}   agreeing: {agree}   differing: {differ}")
sys.exit(1 if differ else 0)
