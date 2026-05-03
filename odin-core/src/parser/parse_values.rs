//! Value parsing — typed conversion helpers used by the streaming parser.

use crate::types::values::{OdinValue, OdinValues};
use crate::types::errors::{ParseError, ParseErrorCode};

/// Unescape a quoted-string body. Mirrors the escape processing the
/// tokenizer used to do eagerly; called only for `QuotedStringEscaped` tokens.
pub(super) fn unescape_string(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'\\' && i + 1 < bytes.len() {
            let esc = bytes[i + 1];
            match esc {
                b'n' => { out.push('\n'); i += 2; }
                b'r' => { out.push('\r'); i += 2; }
                b't' => { out.push('\t'); i += 2; }
                b'\\' => { out.push('\\'); i += 2; }
                b'"' => { out.push('"'); i += 2; }
                b'/' => { out.push('/'); i += 2; }
                b'0' => { out.push('\0'); i += 2; }
                b'u' if i + 5 < bytes.len() => {
                    let hex = &raw[i + 2..i + 6];
                    if let Ok(code) = u32::from_str_radix(hex, 16) {
                        if (0xD800..=0xDBFF).contains(&code)
                            && i + 11 < bytes.len()
                            && bytes[i + 6] == b'\\'
                            && bytes[i + 7] == b'u'
                        {
                            let low_hex = &raw[i + 8..i + 12];
                            if let Ok(low) = u32::from_str_radix(low_hex, 16) {
                                if (0xDC00..=0xDFFF).contains(&low) {
                                    let combined = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                    if let Some(c) = char::from_u32(combined) {
                                        out.push(c);
                                        i += 12;
                                        continue;
                                    }
                                }
                            }
                        }
                        if let Some(c) = char::from_u32(code) {
                            out.push(c);
                        }
                        i += 6;
                    } else {
                        i += 2;
                    }
                }
                b'U' if i + 9 < bytes.len() => {
                    let hex = &raw[i + 2..i + 10];
                    if let Ok(code) = u32::from_str_radix(hex, 16) {
                        if let Some(c) = char::from_u32(code) {
                            out.push(c);
                        }
                    }
                    i += 10;
                }
                _ => { out.push(ch as char); i += 1; }
            }
        } else if ch >= 0x80 {
            // Multi-byte UTF-8: copy the whole codepoint.
            let n = utf8_byte_len(ch);
            let end = (i + n).min(bytes.len());
            out.push_str(&raw[i..end]);
            i = end;
        } else {
            out.push(ch as char);
            i += 1;
        }
    }
    out
}

#[inline]
fn utf8_byte_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

/// Normalize leading zeros in array indices: `path[007]` -> `path[7]`.
/// Returns `Cow::Borrowed` when no normalization is needed (the common case).
fn normalize_reference_path(raw: &str) -> std::borrow::Cow<'_, str> {
    if !needs_reference_normalization(raw.as_bytes()) {
        return std::borrow::Cow::Borrowed(raw);
    }
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            out.push('[');
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i > start && i < bytes.len() && bytes[i] == b']' {
                let idx: i64 = raw[start..i].parse().unwrap_or(0);
                out.push_str(&idx.to_string());
            } else {
                out.push_str(&raw[start..i]);
            }
        } else {
            let run_end = bytes[i..].iter().position(|&b| b == b'[').map(|p| i + p).unwrap_or(bytes.len());
            out.push_str(&raw[i..run_end]);
            i = run_end;
        }
    }
    std::borrow::Cow::Owned(out)
}

#[inline]
fn needs_reference_normalization(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'[' && bytes[i + 1] == b'0' && bytes[i + 2].is_ascii_digit() {
            return true;
        }
        i += 1;
    }
    false
}

pub(super) fn parse_number(raw: &str, line: usize, col: usize) -> Result<OdinValue, ParseError> {
    if raw.is_empty() {
        return Err(ParseError::with_message(
            ParseErrorCode::InvalidTypePrefix, line, col, "empty number after '#'",
        ));
    }

    if raw.starts_with("--") {
        return Err(ParseError::with_message(
            ParseErrorCode::InvalidTypePrefix, line, col, &format!("invalid number: {raw}"),
        ));
    }

    let value: f64 = raw.parse().map_err(|_| {
        ParseError::with_message(ParseErrorCode::InvalidTypePrefix, line, col, &format!("invalid number: {raw}"))
    })?;

    let decimal_places = if raw.contains('.') {
        let num_part = match raw.find(|c: char| c == 'e' || c == 'E') {
            Some(e_pos) => &raw[..e_pos],
            None => raw,
        };
        num_part.find('.').map(|dot_pos| (num_part.len() - dot_pos - 1) as u8)
    } else {
        None
    };

    Ok(OdinValue::Number {
        value,
        decimal_places,
        raw: Some(raw.to_string()),
        modifiers: None,
        directives: Vec::new(),
    })
}

pub(super) fn parse_integer(raw: &str, line: usize, col: usize) -> Result<OdinValue, ParseError> {
    if raw.is_empty() {
        return Err(ParseError::with_message(
            ParseErrorCode::InvalidTypePrefix, line, col, "empty integer after '##'",
        ));
    }

    match raw.parse::<i64>() {
        Ok(value) => {
            Ok(OdinValue::Integer {
                value,
                raw: Some(raw.to_string()),
                modifiers: None,
                directives: Vec::new(),
            })
        }
        Err(_) => {
            Ok(OdinValue::Integer {
                value: 0,
                raw: Some(raw.to_string()),
                modifiers: None,
                directives: Vec::new(),
            })
        }
    }
}

pub(super) fn parse_currency(raw: &str, line: usize, col: usize) -> Result<OdinValue, ParseError> {
    let (num_part, currency_code) = if let Some(colon_pos) = raw.find(':') {
        (&raw[..colon_pos], Some(raw[colon_pos + 1..].to_uppercase()))
    } else {
        (raw, None)
    };

    let value: f64 = num_part.parse().map_err(|_| {
        ParseError::with_message(ParseErrorCode::InvalidTypePrefix, line, col, &format!("invalid currency: {raw}"))
    })?;

    let decimal_places = {
        let e_pos = num_part.find(|c: char| c == 'e' || c == 'E');
        let check_part = match e_pos {
            Some(pos) => &num_part[..pos],
            None => num_part,
        };
        check_part.find('.').map_or(2, |dot_pos| {
            (check_part.len() - dot_pos - 1) as u8
        })
    };

    Ok(OdinValue::Currency {
        value,
        decimal_places,
        currency_code,
        raw: Some(raw.to_string()),
        modifiers: None,
        directives: Vec::new(),
    })
}

pub(super) fn parse_percent(raw: &str, line: usize, col: usize) -> Result<OdinValue, ParseError> {
    let value: f64 = raw.parse().map_err(|_| {
        ParseError::with_message(ParseErrorCode::InvalidTypePrefix, line, col, &format!("invalid percent: {raw}"))
    })?;

    Ok(OdinValue::Percent {
        value,
        raw: Some(raw.to_string()),
        modifiers: None,
        directives: Vec::new(),
    })
}

pub(super) fn parse_binary(raw: &str, line: usize, col: usize) -> Result<OdinValue, ParseError> {
    if raw.is_empty() {
        return Ok(OdinValues::binary(Vec::new()));
    }

    if let Some(colon_pos) = raw.find(':') {
        let algorithm = &raw[..colon_pos];
        let b64_data = &raw[colon_pos + 1..];
        validate_base64(b64_data, line, col)?;
        let data = base64_decode(b64_data);
        Ok(OdinValues::binary_with_algorithm(data, algorithm))
    } else {
        validate_base64(raw, line, col)?;
        let data = base64_decode(raw);
        Ok(OdinValues::binary(data))
    }
}

fn validate_base64(input: &str, line: usize, col: usize) -> Result<(), ParseError> {
    let mut padding_started = false;
    for (i, ch) in input.bytes().enumerate() {
        match ch {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' => {
                if padding_started {
                    return Err(ParseError::with_message(
                        ParseErrorCode::UnexpectedCharacter,
                        line, col,
                        "Invalid Base64 padding",
                    ));
                }
            }
            b'=' => {
                padding_started = true;
            }
            b'\n' | b'\r' => {}
            _ => {
                return Err(ParseError::with_message(
                    ParseErrorCode::UnexpectedCharacter,
                    line, col,
                    &format!("Invalid Base64 character at position {i}"),
                ));
            }
        }
    }
    Ok(())
}

fn base64_decode(input: &str) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits: u8 = 0;

    for ch in input.bytes() {
        let val = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => continue,
        };
        buffer = (buffer << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }

    output
}

pub(super) fn parse_date_value(raw: &str, line: usize, col: usize) -> Result<OdinValue, ParseError> {
    let mut iter = raw.split('-');
    let (year_s, month_s, day_s) = match (iter.next(), iter.next(), iter.next(), iter.next()) {
        (Some(y), Some(m), Some(d), None) => (y, m, d),
        _ => {
            return Err(ParseError::with_message(
                ParseErrorCode::UnexpectedCharacter,
                line, col,
                &format!("invalid date: {raw}"),
            ));
        }
    };
    let year = year_s.parse::<i32>().map_err(|_| {
        ParseError::with_message(ParseErrorCode::UnexpectedCharacter, line, col, &format!("invalid date: {raw}"))
    })?;
    let month = month_s.parse::<u8>().map_err(|_| {
        ParseError::with_message(ParseErrorCode::UnexpectedCharacter, line, col, &format!("invalid date: {raw}"))
    })?;
    let day = day_s.parse::<u8>().map_err(|_| {
        ParseError::with_message(ParseErrorCode::UnexpectedCharacter, line, col, &format!("invalid date: {raw}"))
    })?;

    if !(1..=12).contains(&month) {
        return Err(ParseError::with_message(
            ParseErrorCode::UnexpectedCharacter,
            line, col,
            &format!("Invalid month {month} in date {raw}"),
        ));
    }

    let max_day = days_in_month(year, month);
    if day < 1 || day > max_day {
        return Err(ParseError::with_message(
            ParseErrorCode::UnexpectedCharacter,
            line, col,
            &format!("Invalid day {day} for month {month} in date {raw}"),
        ));
    }

    Ok(OdinValue::Date {
        year,
        month,
        day,
        raw: raw.to_string(),
        modifiers: None,
        directives: Vec::new(),
    })
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn is_date_like(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() >= 10
        && bytes[..4].iter().all(|b| b.is_ascii_digit())
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(|b| b.is_ascii_digit())
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(|b| b.is_ascii_digit())
}
