//! RFC 8785 JSON Canonicalization Scheme (JCS).
//!
//! Ordinary `serde_json::to_string` and `jq -S` are not JCS. Public entry
//! is raw JSON / bytes through a duplicate-aware parser. A `Value` may be
//! encoded only after that parser has guaranteed uniqueness.

use serde_json::{Map, Value};
use thiserror::Error;

/// Canonicalization failure.
#[derive(Debug, Error)]
pub enum JcsError {
    /// Input is not well-formed JSON.
    #[error("invalid JSON")]
    InvalidJson,
    /// An object member name was repeated.
    #[error("duplicate object member name")]
    DuplicateKey,
    /// A lone UTF-16 surrogate is not permitted.
    #[error("lone surrogate is not permitted in JCS")]
    LoneSurrogate,
    /// NaN or Infinity is not permitted.
    #[error("non-finite numbers are not permitted in JCS")]
    NonFiniteNumber,
    /// An integer is not exactly representable as IEEE 754 binary64.
    #[error("integer is outside the I-JSON IEEE 754 binary64 domain")]
    IntegerOutsideIjson,
    /// A value type cannot be canonicalized.
    #[error("unsupported JSON value")]
    Unsupported,
}

/// Parse JSON with I-JSON / RFC 8785 constraints, then canonicalize.
pub fn canonicalize_json(raw: &str) -> Result<Vec<u8>, JcsError> {
    canonicalize_bytes(raw.as_bytes())
}

/// Parse UTF-8 JSON bytes with I-JSON / RFC 8785 constraints, then canonicalize.
pub fn canonicalize_bytes(raw: &[u8]) -> Result<Vec<u8>, JcsError> {
    let text = std::str::from_utf8(raw).map_err(|_| JcsError::InvalidJson)?;
    let value = parse_strict(text)?;
    encode_unique(&value)
}

/// Encode a value that was produced by [`parse_strict`] (uniqueness guaranteed).
pub(crate) fn encode_unique(value: &Value) -> Result<Vec<u8>, JcsError> {
    let mut out = String::new();
    encode(value, &mut out)?;
    Ok(out.into_bytes())
}

/// Parse JSON, rejecting duplicate keys, lone surrogates, non-finite
/// numbers, and integers outside the I-JSON binary64 domain.
pub fn parse_strict(raw: &str) -> Result<Value, JcsError> {
    let mut parser = Parser {
        bytes: raw.as_bytes(),
        pos: 0,
    };
    let value = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(JcsError::InvalidJson);
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn parse_value(&mut self) -> Result<Value, JcsError> {
        self.skip_ws();
        let b = self.peek().ok_or(JcsError::InvalidJson)?;
        match b {
            b'n' => self.parse_literal(b"null", Value::Null),
            b't' => self.parse_literal(b"true", Value::Bool(true)),
            b'f' => self.parse_literal(b"false", Value::Bool(false)),
            b'"' => Ok(Value::String(self.parse_string()?)),
            b'[' => self.parse_array(),
            b'{' => self.parse_object(),
            b'-' | b'0'..=b'9' => self.parse_number(),
            _ => Err(JcsError::InvalidJson),
        }
    }

    fn parse_literal(&mut self, token: &[u8], value: Value) -> Result<Value, JcsError> {
        if self.bytes.get(self.pos..self.pos + token.len()) != Some(token) {
            return Err(JcsError::InvalidJson);
        }
        self.pos += token.len();
        Ok(value)
    }

    fn parse_array(&mut self) -> Result<Value, JcsError> {
        self.pos += 1;
        self.skip_ws();
        let mut items = Vec::new();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(JcsError::InvalidJson),
            }
        }
        Ok(Value::Array(items))
    }

    fn parse_object(&mut self) -> Result<Value, JcsError> {
        self.pos += 1;
        self.skip_ws();
        let mut map = Map::new();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(map));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(JcsError::InvalidJson);
            }
            let key = self.parse_string()?;
            if map.contains_key(&key) {
                return Err(JcsError::DuplicateKey);
            }
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(JcsError::InvalidJson);
            }
            self.pos += 1;
            let value = self.parse_value()?;
            map.insert(key, value);
            self.skip_ws();
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                }
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(JcsError::InvalidJson),
            }
        }
        Ok(Value::Object(map))
    }

    fn parse_string(&mut self) -> Result<String, JcsError> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            let b = self.next().ok_or(JcsError::InvalidJson)?;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = self.next().ok_or(JcsError::InvalidJson)?;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.parse_unicode_escape()?),
                        _ => return Err(JcsError::InvalidJson),
                    }
                }
                0x00..=0x1f => return Err(JcsError::InvalidJson),
                _ => {
                    // Continue a UTF-8 scalar starting at this byte.
                    self.pos -= 1;
                    let ch = self.next_char()?;
                    out.push(ch);
                }
            }
        }
    }

    fn parse_unicode_escape(&mut self) -> Result<char, JcsError> {
        let unit = self.read_hex4()?;
        if (0xd800..=0xdbff).contains(&unit) {
            if self.peek() != Some(b'\\') {
                return Err(JcsError::LoneSurrogate);
            }
            self.pos += 1;
            if self.next() != Some(b'u') {
                return Err(JcsError::LoneSurrogate);
            }
            let low = self.read_hex4()?;
            if !(0xdc00..=0xdfff).contains(&low) {
                return Err(JcsError::LoneSurrogate);
            }
            let cp = 0x10000 + (((u32::from(unit) - 0xd800) << 10) | (u32::from(low) - 0xdc00));
            return char::from_u32(cp).ok_or(JcsError::LoneSurrogate);
        }
        if (0xdc00..=0xdfff).contains(&unit) {
            return Err(JcsError::LoneSurrogate);
        }
        char::from_u32(u32::from(unit)).ok_or(JcsError::InvalidJson)
    }

    fn read_hex4(&mut self) -> Result<u16, JcsError> {
        let mut value = 0u16;
        for _ in 0..4 {
            let b = self.next().ok_or(JcsError::InvalidJson)?;
            let digit = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => return Err(JcsError::InvalidJson),
            };
            value = (value << 4) | u16::from(digit);
        }
        Ok(value)
    }

    fn parse_number(&mut self) -> Result<Value, JcsError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        if self.peek() == Some(b'0') {
            self.pos += 1;
            if matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(JcsError::InvalidJson);
            }
        } else if matches!(self.peek(), Some(b'1'..=b'9')) {
            self.pos += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        } else {
            return Err(JcsError::InvalidJson);
        }
        let mut is_integer_token = true;
        if self.peek() == Some(b'.') {
            is_integer_token = false;
            self.pos += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(JcsError::InvalidJson);
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            is_integer_token = false;
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                return Err(JcsError::InvalidJson);
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.pos += 1;
            }
        }
        let token =
            std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| JcsError::InvalidJson)?;
        if is_integer_token {
            let parsed: i128 = token.parse().map_err(|_| JcsError::IntegerOutsideIjson)?;
            return Ok(Value::Number(ijson_int(parsed)?));
        }
        let number: f64 = token.parse().map_err(|_| JcsError::InvalidJson)?;
        if !number.is_finite() {
            return Err(JcsError::NonFiniteNumber);
        }
        Ok(Value::Number(
            serde_json::Number::from_f64(number).ok_or(JcsError::NonFiniteNumber)?,
        ))
    }

    fn next_char(&mut self) -> Result<char, JcsError> {
        let rest =
            std::str::from_utf8(&self.bytes[self.pos..]).map_err(|_| JcsError::InvalidJson)?;
        let ch = rest.chars().next().ok_or(JcsError::InvalidJson)?;
        self.pos += ch.len_utf8();
        Ok(ch)
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }
}

fn ijson_int(value: i128) -> Result<serde_json::Number, JcsError> {
    let as_float = value as f64;
    if !as_float.is_finite() || as_float as i128 != value {
        return Err(JcsError::IntegerOutsideIjson);
    }
    if let Ok(i) = i64::try_from(value) {
        return Ok(serde_json::Number::from(i));
    }
    serde_json::Number::from_f64(as_float).ok_or(JcsError::IntegerOutsideIjson)
}

fn encode(value: &Value, out: &mut String) -> Result<(), JcsError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::String(s) => encode_string(s, out)?,
        Value::Number(n) => encode_number(n, out)?,
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                encode(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, key) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                encode_string(key, out)?;
                out.push(':');
                encode(&map[*key], out)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn encode_string(value: &str, out: &mut String) -> Result<(), JcsError> {
    out.push('"');
    for ch in value.chars() {
        let code = ch as u32;
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ if (0xd800..=0xdfff).contains(&code) => return Err(JcsError::LoneSurrogate),
            _ if code < 0x20 => {
                out.push_str(&format!("\\u{code:04x}"));
            }
            _ => out.push(ch),
        }
    }
    out.push('"');
    Ok(())
}

fn encode_number(number: &serde_json::Number, out: &mut String) -> Result<(), JcsError> {
    if let Some(i) = number.as_i64() {
        let as_float = i as f64;
        if as_float as i64 != i {
            return Err(JcsError::IntegerOutsideIjson);
        }
        encode_f64(as_float, out)?;
        return Ok(());
    }
    if let Some(u) = number.as_u64() {
        let as_float = u as f64;
        if as_float as u64 != u {
            return Err(JcsError::IntegerOutsideIjson);
        }
        encode_f64(as_float, out)?;
        return Ok(());
    }
    let f = number.as_f64().ok_or(JcsError::Unsupported)?;
    encode_f64(f, out)
}

fn encode_f64(number: f64, out: &mut String) -> Result<(), JcsError> {
    if !number.is_finite() {
        return Err(JcsError::NonFiniteNumber);
    }
    if number == 0.0 {
        out.push('0');
        return Ok(());
    }
    let mut buffer = ryu_js::Buffer::new();
    out.push_str(buffer.format_finite(number));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_keys_and_elides_whitespace() {
        let canonical = canonicalize_json("{\n  \"y\": 2,\n  \"x\": 1\n}\n").unwrap();
        assert_eq!(canonical, br#"{"x":1,"y":2}"#);
    }

    #[test]
    fn integer_valued_float_is_es_integer() {
        let canonical = canonicalize_json("{\"n\": 50.0}").unwrap();
        assert_eq!(canonical, br#"{"n":50}"#);
    }

    #[test]
    fn rejects_duplicate_keys() {
        let err = canonicalize_json("{\"a\":1,\"a\":2}").unwrap_err();
        assert!(matches!(err, JcsError::DuplicateKey));
    }

    #[test]
    fn rejects_lone_surrogate() {
        let err = canonicalize_json("{\"a\":\"\\uD800\"}").unwrap_err();
        assert!(matches!(err, JcsError::LoneSurrogate));
    }

    #[test]
    fn rejects_integer_outside_ijson() {
        let err = canonicalize_json("9007199254740993").unwrap_err();
        assert!(matches!(err, JcsError::IntegerOutsideIjson));
    }

    #[test]
    fn rejects_non_finite_exponent() {
        let err = canonicalize_json("1e400").unwrap_err();
        assert!(matches!(err, JcsError::NonFiniteNumber));
    }

    #[test]
    fn accepts_exact_binary64_integer_bound() {
        let canonical = canonicalize_json("9007199254740992").unwrap();
        assert_eq!(canonical, b"9007199254740992");
    }
}
