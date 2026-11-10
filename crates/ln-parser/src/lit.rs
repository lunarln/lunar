//! Literal decoding: numbers, escapes, and format-string pieces.

use crate::{IdGen, parse_expr_at};
use ln_ast::{FmtPiece, FmtSpec, FmtString, Span};
use ln_diag::FileId;

const INT_SUFFIXES: &[&str] = &["i8", "i16", "i32", "i64", "isize", "u8", "u16", "u32", "u64", "usize"];
const FLOAT_SUFFIXES: &[&str] = &["f32", "f64"];

/// Split a numeric literal into its digits and an optional type suffix.
fn split_suffix(text: &str, radix_prefixed: bool) -> (&str, Option<&str>) {
    for s in INT_SUFFIXES.iter().chain(FLOAT_SUFFIXES) {
        if let Some(body) = text.strip_suffix(s) {
            // `0xff` must not lose an `f`-suffix-looking tail.
            if radix_prefixed && s.starts_with('f') {
                continue;
            }
            if !body.is_empty() {
                return (body, Some(s));
            }
        }
    }
    (text, None)
}

/// Parse an integer literal. Returns the value and suffix, or an error message.
pub fn parse_int_lit(text: &str) -> Result<(u128, Option<String>), String> {
    let (radix, digits_start) = match text.get(..2) {
        Some("0x") => (16, 2),
        Some("0o") => (8, 2),
        Some("0b") => (2, 2),
        _ => (10, 0),
    };
    let (body, suffix) = split_suffix(&text[digits_start..], radix != 10);
    if let Some(s) = suffix
        && FLOAT_SUFFIXES.contains(&s)
    {
        return Err(format!("float suffix `{s}` on an integer literal"));
    }
    let cleaned: String = body.chars().filter(|&c| c != '_').collect();
    if cleaned.is_empty() {
        return Err("integer literal has no digits".into());
    }
    let rest_has_letters = cleaned.chars().any(|c| !c.is_digit(radix));
    if rest_has_letters {
        return Err(format!("invalid digit or suffix in `{text}`"));
    }
    u128::from_str_radix(&cleaned, radix).map(|v| (v, suffix.map(str::to_string))).map_err(|_| "integer literal is too large".into())
}

pub fn parse_float_lit(text: &str) -> Result<(f64, Option<String>), String> {
    let (body, suffix) = split_suffix(text, false);
    if let Some(s) = suffix
        && INT_SUFFIXES.contains(&s)
    {
        return Err(format!("integer suffix `{s}` on a float literal"));
    }
    let cleaned: String = body.chars().filter(|&c| c != '_').collect();
    cleaned.parse::<f64>().map(|v| (v, suffix.map(str::to_string))).map_err(|_| format!("invalid float literal `{text}`"))
}

/// Unescape the body of a string/char literal (without quotes). On error
/// returns the byte offset (within `raw`) and message.
pub(crate) fn unescape(raw: &str) -> Result<String, (usize, usize, String)> {
    let mut out = String::with_capacity(raw.len());
    let mut it = raw.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some((_, e)) = it.next() else {
            return Err((i, i + 1, "trailing backslash".into()));
        };
        match e {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '\'' => out.push('\''),
            '\n' => {
                // Line continuation: skip leading whitespace on the next line.
                while let Some(&(_, w)) = it.peek() {
                    if w.is_whitespace() {
                        it.next();
                    } else {
                        break;
                    }
                }
            }
            'x' => {
                let h: String = (0..2).filter_map(|_| it.next().map(|(_, c)| c)).collect();
                match u8::from_str_radix(&h, 16) {
                    Ok(b) if b < 0x80 => out.push(b as char),
                    _ => return Err((i, i + 4, "`\\x` escapes must be two hex digits below 0x80".into())),
                }
            }
            'u' => {
                if it.next().map(|x| x.1) != Some('{') {
                    return Err((i, i + 2, "expected `{` after `\\u`".into()));
                }
                let mut h = String::new();
                let mut end = i + 3;
                for (j, c) in it.by_ref() {
                    end = j + 1;
                    if c == '}' {
                        break;
                    }
                    h.push(c);
                }
                match u32::from_str_radix(&h, 16).ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => return Err((i, end, format!("invalid unicode escape `\\u{{{h}}}`"))),
                }
            }
            other => return Err((i, i + 1 + other.len_utf8(), format!("unknown escape `\\{other}`"))),
        }
    }
    Ok(out)
}

/// The value of a byte literal body (between `b'` and `'`): one ASCII
/// character, or an escape up to `\xFF`. On error returns a message and
/// an optional help.
pub(crate) fn unescape_byte(raw: &str) -> Result<u8, (String, Option<String>)> {
    if let Some(hex) = raw.strip_prefix("\\x") {
        return match u8::from_str_radix(hex, 16) {
            Ok(b) if hex.len() == 2 => Ok(b),
            _ => Err(("`\\x` escapes in byte literals must be two hex digits".into(), None)),
        };
    }
    if raw.starts_with("\\u") {
        return Err(("unicode escapes cannot be used in byte literals".into(), Some("a byte is a single `\\x00`..`\\xFF` value".into())));
    }
    let s = unescape(raw).map_err(|(_, _, m)| (m, None))?;
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii() => Ok(c as u8),
        (Some(c), None) => Err((
            format!("non-ASCII character `{c}` in a byte literal"),
            Some(format!("its UTF-8 encoding is {} bytes; write one byte as `\\xNN`, or use the `char` literal `'{c}'`", c.len_utf8())),
        )),
        (None, _) => Err(("byte literal must contain exactly one byte".into(), Some("for the zero byte write `b'\\0'`".into()))),
        _ => Err(("byte literal must contain exactly one byte".into(), None)),
    }
}

/// Split a format hole into its expression and format spec: `p:?` is `p`
/// with `Debug`. The spec starts at the first `:` outside brackets,
/// strings and character literals that is not part of `::`; no expression
/// contains such a `:` (Lunar has no ternary and no type ascription).
fn split_spec(inner: &str) -> Result<(&str, FmtSpec), String> {
    let b = inner.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            // Skip string and character literals (`':'`, `"a:b"`).
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
            }
            b':' if depth == 0 => {
                if b.get(i + 1) == Some(&b':') {
                    i += 2;
                    continue;
                }
                let spec = inner[i + 1..].trim();
                return match spec {
                    "?" => Ok((&inner[..i], FmtSpec::Debug)),
                    "" => Err("empty format spec after `:`; write `{x:?}` for `Debug`, or `{x}` for `Display`".into()),
                    s => Err(format!("unknown format spec `{s}`; the only one is `?` (`{{x:?}}` uses `Debug`)")),
                };
            }
            _ => {}
        }
        i += 1;
    }
    Ok((inner, FmtSpec::Display))
}

/// Interpret a string literal body (`raw`, between the quotes, starting at
/// byte `offset` of `file`) as a format string: `{expr}` holes, `{{`/`}}`
/// escapes.
pub(crate) fn parse_format(raw: &str, offset: u32, file: FileId, ids: &mut IdGen) -> FmtString {
    let mut pieces = Vec::new();
    let mut text_start = 0;
    let bytes = raw.as_bytes();
    let mut i = 0;
    let mut text = String::new();
    let span_at = |a: usize, b: usize| Span::new(file, offset + a as u32, offset + b as u32);

    let flush_text = |text: &mut String, seg: &str, start: usize| -> Result<(), FmtString> {
        match unescape(seg) {
            Ok(s) => {
                text.push_str(&s);
                Ok(())
            }
            Err((a, b, m)) => Err(FmtString::Err(m, span_at(start + a, start + b))),
        }
    };

    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                // Skip the escaped character so `\{` is not seen as a hole.
                i += 2;
            }
            b'{' if bytes.get(i + 1) == Some(&b'{') => {
                if let Err(e) = flush_text(&mut text, &raw[text_start..i], text_start) {
                    return e;
                }
                text.push('{');
                i += 2;
                text_start = i;
            }
            b'}' if bytes.get(i + 1) == Some(&b'}') => {
                if let Err(e) = flush_text(&mut text, &raw[text_start..i], text_start) {
                    return e;
                }
                text.push('}');
                i += 2;
                text_start = i;
            }
            b'}' => return FmtString::Err("unmatched `}` in format string (use `}}` for a literal brace)".into(), span_at(i, i + 1)),
            b'{' => {
                if let Err(e) = flush_text(&mut text, &raw[text_start..i], text_start) {
                    return e;
                }
                if !text.is_empty() {
                    pieces.push(FmtPiece::Text(std::mem::take(&mut text)));
                }
                // The matching close brace, as the lexer found it.
                let open = i;
                let Some(j) = ln_lexer::hole_end(bytes, i + 1) else {
                    return FmtString::Err("unclosed `{` in format string (use `{{` for a literal brace)".into(), span_at(open, open + 1));
                };
                let inner = &raw[open + 1..j - 1];
                if inner.trim().is_empty() {
                    return FmtString::Err("empty `{}` in format string: put an expression inside, e.g. `{x}`".into(), span_at(open, j));
                }
                let (inner, spec) = match split_spec(inner) {
                    Ok(x) => x,
                    Err(m) => return FmtString::Err(m, span_at(open, j)),
                };
                match parse_expr_at(inner, offset + open as u32 + 1, file, ids) {
                    Ok(e) => pieces.push(FmtPiece::Expr(Box::new(e), spec)),
                    Err(diags) => {
                        let msg = diags.first().map(|d| d.message.clone()).unwrap_or_default();
                        return FmtString::Err(format!("invalid expression in format string: {msg}"), span_at(open, j));
                    }
                }
                i = j;
                text_start = i;
            }
            _ => i += 1,
        }
    }
    if let Err(e) = flush_text(&mut text, &raw[text_start.min(raw.len())..], text_start) {
        return e;
    }
    if !text.is_empty() {
        pieces.push(FmtPiece::Text(text));
    }
    FmtString::Ok(pieces)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ints() {
        assert_eq!(parse_int_lit("1_000").unwrap(), (1000, None));
        assert_eq!(parse_int_lit("0xffu8").unwrap(), (255, Some("u8".into())));
        assert_eq!(parse_int_lit("0b101").unwrap(), (5, None));
        assert!(parse_int_lit("12abc").is_err());
    }

    #[test]
    fn floats() {
        assert_eq!(parse_float_lit("0.").unwrap(), (0.0, None));
        assert_eq!(parse_float_lit("1.5f32").unwrap(), (1.5, Some("f32".into())));
        assert_eq!(parse_float_lit("2e3").unwrap(), (2000.0, None));
    }

    #[test]
    fn escapes() {
        assert_eq!(unescape(r"a\nb\u{41}\x41").unwrap(), "a\nbAA");
        assert!(unescape(r"\q").is_err());
    }
}
