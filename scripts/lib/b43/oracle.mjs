// The reference for b43_diff.py's JavaScript families: Node's URL,
// URLSearchParams, util.MIMEType, fetch on data: URLs, and npm's own
// hosted-git-info, semver, npm-package-arg and negotiator. Usage:
// node oracle.mjs <op> <file> -- one answer per input line, in the
// harness's encoding.
import fs from "fs";
import util from "util";
import url from "url";
import { createRequire } from "module";
import path from "path";
import { execSync } from "child_process";

const npmRoot = path.join(execSync("npm root -g").toString().trim(), "npm", "node_modules");
const require = createRequire(import.meta.url);
const need = (m) => require(path.join(npmRoot, m));
const hx = (s) => (s == null ? "-" : Buffer.from(String(s), "utf8").toString("hex"));
const un = (h) => (h === "-" || h === "_" ? "" : Buffer.from(h, "hex").toString("utf8"));
const [op, file] = process.argv.slice(2);
const lines = fs.readFileSync(file, "utf8").split("\n").filter((l) => l.length);
const out = [];

const urlFields = (u) => [u.href, u.origin, u.protocol, u.username, u.password, u.host, u.hostname, u.port, u.pathname, u.search, u.hash].map(hx).join(",");

for (const line of lines) {
  const f = line.split(" ");
  if (op === "domain") {
    const d = un(f[0]);
    const a = url.domainToASCII(d);
    out.push((a === "" ? "-" : hx(a)) + "," + hx(url.domainToUnicode(d)));
  } else if (op === "url") {
    try { out.push(urlFields(f[1] === "-" ? new URL(un(f[0])) : new URL(un(f[0]), un(f[1])))); } catch { out.push("FAIL"); }
  } else if (op === "set") {
    const u = new URL(un(f[0]));
    try { u[un(f[1])] = un(f[2]); } catch {}
    out.push(hx(u.href));
  } else if (op === "qs") {
    const v = un(f[0]);
    const p = new URLSearchParams(v); const q = new URLSearchParams(v); q.sort();
    out.push(hx(p.toString()) + "," + hx(q.toString()) + "," + hx([...p].map(([a, b]) => a + "=" + b).join("&")));
  } else if (op === "mime") {
    try { const m = new util.MIMEType(un(f[0])); out.push(hx(m.toString()) + "," + hx(m.essence)); } catch { out.push("FAIL"); }
  } else if (op === "data") {
    try { const r = await fetch(un(f[0])); const b = Buffer.from(await r.arrayBuffer()); out.push(hx(r.headers.get("content-type")) + "," + b.toString("hex")); }
    catch { out.push("FAIL"); }
  } else if (op === "hosted") {
    const h = need("hosted-git-info").fromUrl(un(f[0]));
    if (!h) { out.push("NONE"); continue; }
    const p = [h.type, h.user, h.auth, h.project, h.committish, h.default];
    for (const w of ["ssh", "sshurl", "docs", "bugs", "https", "git", "shortcut", "path", "tarball", "browse"]) p.push(h[w]());
    p.push(h.browse("/a/b"), h.browse("a/b", "Some Frag!"), h.browseFile("x.js"), h.file("p/q.txt"), h.edit("e.md"));
    p.push(h.https({ noGitPlus: true, noCommittish: true }), h.toString());
    out.push(p.map(hx).join(","));
  } else if (op === "semver") {
    const semver = need("semver");
    const v = un(f[0]), rg = un(f[1]), versions = f[2].split(",").map(un);
    const o = [];
    for (const loose of [false, true]) {
      o.push(hx(semver.valid(v, { loose })), hx(semver.clean(v, { loose })));
      for (const includePrerelease of [false, true]) {
        const opt = { loose, includePrerelease };
        o.push(hx(semver.validRange(rg, opt)), semver.satisfies(v, rg, opt) ? "T" : "F", hx(semver.maxSatisfying(versions, rg, opt)), hx(semver.minSatisfying(versions, rg, opt)));
      }
    }
    out.push(o.join(","));
  } else if (op === "npa") {
    try {
      const r = need("npm-package-arg")(un(f[0]), "/w/proj");
      out.push([hx(r.type), hx(r.name), hx(r.escapedName), hx(r.scope), hx(r.rawSpec), hx(r.saveSpec), hx(r.fetchSpec), hx(r.gitRange), hx(r.gitCommittish),
        hx(r.gitSubdir), r.registry ? "T" : "-", r.subSpec ? hx(r.subSpec.toString()) : "-", hx(r.toString())].join(","));
    } catch { out.push("ERR"); }
  } else if (op === "neg") {
    const N = need("negotiator");
    const hdr = f[1] === "-" ? undefined : un(f[1]);
    const offers = f[2] === "-" ? undefined : f[2] === "_" ? [] : f[2].split(",").map(un);
    const pref = f[3] && f[3] !== "-" ? f[3].split(",").map(un) : undefined;
    const name = { m: "accept", c: "accept-charset", e: "accept-encoding", l: "accept-language" }[f[0]];
    const n = new N({ headers: hdr === undefined ? {} : { [name]: hdr } });
    const r = f[0] === "m" ? n.mediaTypes(offers) : f[0] === "c" ? n.charsets(offers) : f[0] === "e" ? n.encodings(offers, pref ? { preferred: pref } : undefined) : n.languages(offers);
    out.push(r.map(hx).join(",") + ".");
  }
}
console.log(out.join("\n"));
