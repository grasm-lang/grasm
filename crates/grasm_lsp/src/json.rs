//! Minimal JSON reader/writer.
//!
//! The LSP protocol needs only a small slice of JSON, and pulling in a full
//! serialization stack for it would outweigh the code here. This handles
//! exactly what the protocol sends and receives.

use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub fn obj(pairs: Vec<(&str, Json)>) -> Json {
        Json::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    pub fn str(s: impl Into<String>) -> Json {
        Json::Str(s.into())
    }

    pub fn int(i: i64) -> Json {
        Json::Num(i as f64)
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(m) => m.get(key),
            _ => None,
        }
    }

    /// Follows a dotted path: `params.textDocument.uri`.
    pub fn path(&self, dotted: &str) -> Option<&Json> {
        let mut cur = self;
        for part in dotted.split('.') {
            cur = cur.get(part)?;
        }
        Some(cur)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Json::Num(n) => Some(*n as i64),
            _ => None,
        }
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => {
                let _ = write!(out, "{b}");
            }
            Json::Num(n) => {
                if n.fract() == 0.0 && n.is_finite() {
                    let _ = write!(out, "{}", *n as i64);
                } else {
                    let _ = write!(out, "{n}");
                }
            }
            Json::Str(s) => write_escaped(s, out),
            Json::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Json::Obj(m) => {
                out.push('{');
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_escaped(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn write_escaped(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

pub fn parse(src: &str) -> Result<Json, String> {
    let bytes: Vec<char> = src.chars().collect();
    let mut p = Parser { s: &bytes, i: 0 };
    p.skip_ws();
    let v = p.value()?;
    Ok(v)
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.i += 1;
        }
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected `{c}` at offset {}", self.i))
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('"') => Ok(Json::Str(self.string()?)),
            Some('t') => self.literal("true", Json::Bool(true)),
            Some('f') => self.literal("false", Json::Bool(false)),
            Some('n') => self.literal("null", Json::Null),
            Some(c) if c == '-' || c.is_ascii_digit() => self.number(),
            other => Err(format!("unexpected {other:?} at offset {}", self.i)),
        }
    }

    fn literal(&mut self, word: &str, v: Json) -> Result<Json, String> {
        for c in word.chars() {
            self.expect(c)?;
        }
        Ok(v)
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.i;
        if self.peek() == Some('-') {
            self.i += 1;
        }
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-')
        {
            self.i += 1;
        }
        let text: String = self.s[start..self.i].iter().collect();
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|e| format!("bad number `{text}`: {e}"))
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string".into()),
                Some('"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some('\\') => {
                    self.i += 1;
                    let c = self.peek().ok_or("truncated escape")?;
                    self.i += 1;
                    match c {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'u' => {
                            let hex: String = self.s[self.i..(self.i + 4).min(self.s.len())]
                                .iter()
                                .collect();
                            self.i += 4;
                            let code = u32::from_str_radix(&hex, 16)
                                .map_err(|e| format!("bad \\u escape: {e}"))?;
                            out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        }
                        other => return Err(format!("unknown escape \\{other}")),
                    }
                }
                Some(c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }

    fn array(&mut self) -> Result<Json, String> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.i += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.i += 1;
                }
                Some(']') => {
                    self.i += 1;
                    return Ok(Json::Arr(items));
                }
                other => return Err(format!("expected `,` or `]`, got {other:?}")),
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.expect('{')?;
        let mut map = BTreeMap::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.i += 1;
            return Ok(Json::Obj(map));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(':')?;
            let val = self.value()?;
            map.insert(key, val);
            self.skip_ws();
            match self.peek() {
                Some(',') => {
                    self.i += 1;
                }
                Some('}') => {
                    self.i += 1;
                    return Ok(Json::Obj(map));
                }
                other => return Err(format!("expected `,` or `}}`, got {other:?}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let src = r#"{"a":1,"b":[true,null,"x\ny"],"c":{"d":-2.5}}"#;
        let v = parse(src).unwrap();
        assert_eq!(v.path("c.d").unwrap().as_i64(), Some(-2));
        assert_eq!(v.path("b").unwrap(), &Json::Arr(vec![
            Json::Bool(true),
            Json::Null,
            Json::Str("x\ny".into())
        ]));
        // Re-parsing our own output must give the same value.
        assert_eq!(parse(&v.to_text()).unwrap(), v);
    }

    #[test]
    fn escapes_and_unicode() {
        let v = parse(r#"{"k":"tab\there A"}"#).unwrap();
        assert_eq!(v.path("k").unwrap().as_str(), Some("tab\there A"));
        let out = Json::str("quote\" back\\slash").to_text();
        assert_eq!(parse(&out).unwrap().as_str(), Some("quote\" back\\slash"));
    }

    #[test]
    fn empty_containers() {
        assert_eq!(parse("{}").unwrap(), Json::Obj(Default::default()));
        assert_eq!(parse("[]").unwrap(), Json::Arr(vec![]));
    }

    #[test]
    fn malformed_is_an_error_not_a_panic() {
        for bad in ["{", "[1,", "\"unterminated", "{\"a\"}", "tru", "@"] {
            assert!(parse(bad).is_err(), "`{bad}` should fail");
        }
    }
}
