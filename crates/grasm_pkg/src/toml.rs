//! A small TOML reader — enough for manifests and lockfiles, no dependency.
//!
//! Supports: comments, `[table]` and `[table.sub]` headers, string / integer /
//! boolean / array-of-string values, and basic escapes. Anything outside that
//! is a clear error rather than a silent misparse.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    Arr(Vec<Value>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_str_array(&self) -> Option<Vec<&str>> {
        match self {
            Value::Arr(items) => items.iter().map(|i| i.as_str()).collect(),
            _ => None,
        }
    }
}

/// A parsed document: table path (dotted) → key → value.
#[derive(Debug, Default, Clone)]
pub struct Doc {
    pub tables: BTreeMap<String, BTreeMap<String, Value>>,
    /// `[[name]]` arrays of tables, in document order. Kept separate from
    /// `tables` because each occurrence is its own table — collapsing them
    /// would silently keep only the last one, which for a lockfile means
    /// verifying one package and ignoring the rest.
    pub arrays: BTreeMap<String, Vec<BTreeMap<String, Value>>>,
}

impl Doc {
    pub fn get(&self, table: &str, key: &str) -> Option<&Value> {
        self.tables.get(table)?.get(key)
    }

    pub fn table(&self, name: &str) -> Option<&BTreeMap<String, Value>> {
        self.tables.get(name)
    }

    /// Every occurrence of `[[name]]`, in document order.
    pub fn array(&self, name: &str) -> &[BTreeMap<String, Value>] {
        self.arrays.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Sub-tables of `prefix`, as (leaf name, entries).
    pub fn subtables(&self, prefix: &str) -> Vec<(String, &BTreeMap<String, Value>)> {
        let want = format!("{prefix}.");
        self.tables
            .iter()
            .filter_map(|(k, v)| k.strip_prefix(&want).map(|leaf| (leaf.to_string(), v)))
            .collect()
    }
}

/// Where subsequent `key = value` lines are being collected.
enum Target {
    Table(String),
    /// Index into `Doc::arrays[name]`.
    ArrayEntry(String, usize),
}

pub fn parse(src: &str) -> Result<Doc, String> {
    let mut doc = Doc::default();
    let mut current = Target::Table(String::new());
    doc.tables.insert(String::new(), BTreeMap::new());

    for (lineno, raw) in src.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let at = || format!("line {}", lineno + 1);

        // `[[name]]` starts a new entry in an array of tables; `[name]`
        // starts (or reopens) an ordinary table.
        if let Some(rest) = line.strip_prefix("[[") {
            let name = rest
                .strip_suffix("]]")
                .ok_or_else(|| format!("{}: unclosed array-of-tables header", at()))?
                .trim();
            if name.is_empty() {
                return Err(format!("{}: empty table name", at()));
            }
            let entries = doc.arrays.entry(name.to_string()).or_default();
            entries.push(BTreeMap::new());
            current = Target::ArrayEntry(name.to_string(), entries.len() - 1);
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let name = rest
                .strip_suffix(']')
                .ok_or_else(|| format!("{}: unclosed table header", at()))?
                .trim();
            if name.is_empty() {
                return Err(format!("{}: empty table name", at()));
            }
            // Defining a table twice is invalid TOML, and it was accepted with
            // the later one silently winning: two `[package]` tables published
            // under whichever name came last, and that name went into the
            // lockfile. Two `[dependencies.n]` tables surfaced later as a
            // resolution error that never mentioned the duplicate.
            if doc.tables.contains_key(name) {
                return Err(format!("{}: table `{name}` is defined twice", at()));
            }
            current = Target::Table(name.to_string());
            doc.tables.entry(name.to_string()).or_default();
            continue;
        }

        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("{}: expected `key = value`", at()))?;
        let key = key.trim().trim_matches('"').to_string();
        if key.is_empty() {
            return Err(format!("{}: empty key", at()));
        }
        let value = parse_value(value.trim()).map_err(|e| format!("{}: {e}", at()))?;
        match &current {
            Target::Table(name) => {
                let table = doc.tables.entry(name.clone()).or_default();
                if table.contains_key(&key) {
                    return Err(format!("{}: key `{key}` is set twice", at()));
                }
                table.insert(key, value);
            }
            Target::ArrayEntry(name, i) => {
                doc.arrays
                    .get_mut(name)
                    .expect("array exists once its header was seen")[*i]
                    .insert(key, value);
            }
        }
    }
    Ok(doc)
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..i],
            _ => {}
        }
    }
    line
}

fn parse_value(text: &str) -> Result<Value, String> {
    if text == "true" {
        return Ok(Value::Bool(true));
    }
    if text == "false" {
        return Ok(Value::Bool(false));
    }
    if let Some(rest) = text.strip_prefix('"') {
        let body = rest
            .strip_suffix('"')
            .ok_or("unterminated string value")?;
        return Ok(Value::Str(unescape(body)));
    }
    if let Some(rest) = text.strip_prefix('[') {
        let body = rest.strip_suffix(']').ok_or("unterminated array")?;
        let mut items = Vec::new();
        for part in split_top_level(body) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            items.push(parse_value(part)?);
        }
        return Ok(Value::Arr(items));
    }
    text.parse::<i64>()
        .map(Value::Int)
        .map_err(|_| format!("cannot parse value `{text}`"))
}

fn split_top_level(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut in_string = false;
    for (i, c) in body.char_indices() {
        match c {
            '"' => in_string = !in_string,
            ',' if !in_string => {
                out.push(&body[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&body[start..]);
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Quotes a string for writing back out.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_manifest_shape() {
        let doc = parse(
            r#"
# a comment
[package]
name = "demo"
version = "1.2.3"
capabilities = ["io", "Clock"]

[dependencies.utils]
path = "../utils"
"#,
        )
        .unwrap();
        assert_eq!(doc.get("package", "name").unwrap().as_str(), Some("demo"));
        assert_eq!(
            doc.get("package", "capabilities").unwrap().as_str_array(),
            Some(vec!["io", "Clock"])
        );
        let deps = doc.subtables("dependencies");
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].0, "utils");
    }

    #[test]
    fn handles_comments_hashes_in_strings_and_escapes() {
        let doc = parse("[p]\nk = \"has # hash\" # trailing\ne = \"tab\\there\"\n").unwrap();
        assert_eq!(doc.get("p", "k").unwrap().as_str(), Some("has # hash"));
        assert_eq!(doc.get("p", "e").unwrap().as_str(), Some("tab\there"));
    }

    #[test]
    fn reports_malformed_input_with_line_numbers() {
        for bad in ["[unclosed\n", "novalue\n", "[t]\nk = \"open\n", "[t]\nk = ?\n"] {
            let e = parse(bad).expect_err("should fail");
            assert!(e.starts_with("line "), "expected a located error, got: {e}");
        }
    }

    #[test]
    fn ints_bools_and_arrays() {
        let doc = parse("[t]\nn = 42\nb = true\na = [\"x\", \"y\"]\n").unwrap();
        assert_eq!(doc.get("t", "n"), Some(&Value::Int(42)));
        assert_eq!(doc.get("t", "b"), Some(&Value::Bool(true)));
        assert_eq!(
            doc.get("t", "a").unwrap().as_str_array(),
            Some(vec!["x", "y"])
        );
    }
}
