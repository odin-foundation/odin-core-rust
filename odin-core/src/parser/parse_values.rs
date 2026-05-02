//! Value parsing — detecting and converting ODIN value types from tokens.

use crate::types::values::{OdinValue, OdinValues, OdinModifiers};
use crate::types::errors::{ParseError, ParseErrorCode};
use super::tokens::{Token, TokenType};

/// Parse a value from a sequence of tokens starting at the given position.
///
/// Returns the parsed value and the number of tokens consumed.
pub fn parse_value<'a>(tokens: &[Token<'a>], pos: usize) -> Result<(OdinValue, usize), ParseError> {
    if pos >= tokens.len() {
        return Err(ParseError::new(ParseErrorCode::UnexpectedCharacter, 0, 0));
    }

    let token = &tokens[pos];

    match token.token_type {
        TokenType::Null => Ok((OdinValues::null(), 1)),
        TokenType::BooleanLiteral => {
            let value = token.value == "true";
            Ok((OdinValues::boolean(value), 1))
        }
        TokenType::BooleanPrefix => {
            // ?true or ?false — consume next token
            if pos + 1 < tokens.len() {
                let next = &tokens[pos + 1];
                match next.value.as_ref() {
                    "true" => Ok((OdinValues::boolean(true), 2)),
                    "false" => Ok((OdinValues::boolean(false), 2)),
                    _ => {
                        // Just `?` alone — treat as boolean true
                        Ok((OdinValues::boolean(true), 1))
                    }
                }
            } else {
                Ok((OdinValues::boolean(true), 1))
            }
        }
        TokenType::QuotedString => {
            Ok((OdinValues::string(&*token.value), 1))
        }
        TokenType::BareWord => {
            // Check for boolean bare words
            match token.value.as_ref() {
                "true" => Ok((OdinValues::boolean(true), 1)),
                "false" => Ok((OdinValues::boolean(false), 1)),
                _ => {
                    // Bare strings are not allowed in ODIN
                    Err(ParseError::with_message(
                        ParseErrorCode::BareStringNotAllowed,
                        token.line as usize, token.column as usize,
                        &format!("Unquoted string \"{}\" - use double quotes", token.value),
                    ))
                }
            }
        }
        TokenType::NumberPrefix => {
            let value = parse_number(&token.value, token.line as usize, token.column as usize)?;
            Ok((value, 1))
        }
        TokenType::IntegerPrefix => {
            let value = parse_integer(&token.value, token.line as usize, token.column as usize)?;
            Ok((value, 1))
        }
        TokenType::CurrencyPrefix => {
            let value = parse_currency(&token.value, token.line as usize, token.column as usize)?;
            Ok((value, 1))
        }
        TokenType::PercentPrefix => {
            let value = parse_percent(&token.value, token.line as usize, token.column as usize)?;
            Ok((value, 1))
        }
        TokenType::ReferencePrefix => {
            Ok((OdinValues::reference(&*token.value), 1))
        }
        TokenType::BinaryPrefix => {
            let value = parse_binary(&token.value, token.line as usize, token.column as usize)?;
            Ok((value, 1))
        }
        TokenType::DateLiteral => {
            parse_date_value(&token.value, token.line as usize, token.column as usize)
                .map(|v| (v, 1))
        }
        TokenType::TimeLiteral => {
            Ok((OdinValues::time(&*token.value), 1))
        }
        TokenType::DurationLiteral => {
            Ok((OdinValues::duration(&*token.value), 1))
        }
        TokenType::TimestampLiteral => {
            Ok((OdinValues::timestamp(0, &*token.value), 1))
        }
        TokenType::Path => {
            // Path tokens in value position can be temporal values
            if is_date_like(&token.value) {
                if let Ok(val) = parse_date_value(&token.value, token.line as usize, token.column as usize) {
                    return Ok((val, 1));
                }
            }
            if token.value.starts_with('T') && token.value.contains(':') {
                return Ok((OdinValues::time(&*token.value), 1));
            }
            if token.value.starts_with('P') && token.value.len() > 1 {
                let second = token.value.as_bytes()[1];
                if second.is_ascii_digit() || second == b'T' {
                    return Ok((OdinValues::duration(&*token.value), 1));
                }
            }

            // Bare string — not allowed
            Err(ParseError::with_message(
                ParseErrorCode::BareStringNotAllowed,
                token.line as usize, token.column as usize,
                &format!("Unquoted string \"{}\" - use double quotes", token.value),
            ))
        }
        TokenType::VerbPrefix => {
            // Unquoted verb expression: %verbName args... — collect rest of line
            // as a raw string. Build into one buffer instead of allocating per token.
            let is_custom = token.value.starts_with('&');
            let mut raw_expr = String::with_capacity(token.value.len() + 16);
            raw_expr.push('%');
            raw_expr.push_str(&token.value);
            let mut consumed = 1;
            let mut i = pos + 1;
            while i < tokens.len() {
                let t = &tokens[i];
                if t.token_type == TokenType::Newline || t.token_type == TokenType::Comment {
                    break;
                }
                raw_expr.push(' ');
                match t.token_type {
                    TokenType::ReferencePrefix => { raw_expr.push('@'); raw_expr.push_str(&t.value); }
                    TokenType::IntegerPrefix => { raw_expr.push_str("##"); raw_expr.push_str(&t.value); }
                    TokenType::NumberPrefix => { raw_expr.push('#'); raw_expr.push_str(&t.value); }
                    TokenType::CurrencyPrefix => { raw_expr.push_str("#$"); raw_expr.push_str(&t.value); }
                    TokenType::PercentPrefix => { raw_expr.push_str("#%"); raw_expr.push_str(&t.value); }
                    TokenType::BooleanPrefix => raw_expr.push('?'),
                    TokenType::QuotedString => { raw_expr.push('"'); raw_expr.push_str(&t.value); raw_expr.push('"'); }
                    TokenType::Null => raw_expr.push('~'),
                    TokenType::Directive => { raw_expr.push(':'); raw_expr.push_str(&t.value); }
                    TokenType::VerbPrefix => { raw_expr.push('%'); raw_expr.push_str(&t.value); }
                    _ => raw_expr.push_str(&t.value),
                }
                consumed += 1;
                i += 1;
            }
            Ok((OdinValue::Verb {
                verb: raw_expr,
                is_custom,
                args: vec![],
                modifiers: None,
                directives: vec![],
            }, consumed))
        }
        _ => {
            Err(ParseError::with_message(
                ParseErrorCode::UnexpectedCharacter,
                token.line as usize, token.column as usize,
                &format!("unexpected token type {:?} for value", token.token_type),
            ))
        }
    }
}

/// Parse modifiers preceding a value (!required, *confidential, -deprecated).
pub fn parse_modifiers<'a>(tokens: &[Token<'a>], pos: usize) -> (OdinModifiers, usize) {
    let mut modifiers = OdinModifiers::default();
    let mut consumed = 0;

    while pos + consumed < tokens.len() && tokens[pos + consumed].token_type == TokenType::Modifier {
        match tokens[pos + consumed].value.as_ref() {
            "!" => modifiers.required = true,
            "*" => modifiers.confidential = true,
            "-" => modifiers.deprecated = true,
            _ => break,
        }
        consumed += 1;
    }

    (modifiers, consumed)
}

fn parse_number(raw: &str, line: usize, col: usize) -> Result<OdinValue, ParseError> {
    if raw.is_empty() {
        return Err(ParseError::with_message(
            ParseErrorCode::InvalidTypePrefix, line, col, "empty number after '#'",
        ));
    }

    // Check for double negatives
    if raw.starts_with("--") {
        return Err(ParseError::with_message(
            ParseErrorCode::InvalidTypePrefix, line, col, &format!("invalid number: {raw}"),
        ));
    }

    let value: f64 = raw.parse().map_err(|_| {
        ParseError::with_message(ParseErrorCode::InvalidTypePrefix, line, col, &format!("invalid number: {raw}"))
    })?;

    let decimal_places = if raw.contains('.') {
        // Find the decimal part (before any 'e'/'E')
        let lower = raw.to_lowercase();
        let num_part = if let Some(e_pos) = lower.find('e') {
            &raw[..e_pos]
        } else {
            raw
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

fn parse_integer(raw: &str, line: usize, col: usize) -> Result<OdinValue, ParseError> {
    if raw.is_empty() {
        return Err(ParseError::with_message(
            ParseErrorCode::InvalidTypePrefix, line, col, "empty integer after '##'",
        ));
    }

    // Try parsing as i64 first
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
            // For very large integers, store 0 but preserve raw
            // This handles numbers beyond i64 range
            Ok(OdinValue::Integer {
                value: 0,
                raw: Some(raw.to_string()),
                modifiers: None,
                directives: Vec::new(),
            })
        }
    }
}

fn parse_currency(raw: &str, line: usize, col: usize) -> Result<OdinValue, ParseError> {
    // Format: "100.00" or "100.00:USD"
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

fn parse_percent(raw: &str, line: usize, col: usize) -> Result<OdinValue, ParseError> {
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

fn parse_binary(raw: &str, line: usize, col: usize) -> Result<OdinValue, ParseError> {
    // Empty binary
    if raw.is_empty() {
        return Ok(OdinValues::binary(Vec::new()));
    }

    // Format: "base64data" or "algorithm:base64data"
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

/// Validate base64 content - check for invalid characters and padding.
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

/// Simple base64 decoder (no external dependency).
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

/// Parse and validate a date string (YYYY-MM-DD).
fn parse_date_value(raw: &str, line: usize, col: usize) -> Result<OdinValue, ParseError> {
    let parts: Vec<&str> = raw.split('-').collect();
    if parts.len() != 3 {
        return Err(ParseError::with_message(
            ParseErrorCode::UnexpectedCharacter,
            line, col,
            &format!("invalid date: {raw}"),
        ));
    }
    let year = parts[0].parse::<i32>().map_err(|_| {
        ParseError::with_message(ParseErrorCode::UnexpectedCharacter, line, col, &format!("invalid date: {raw}"))
    })?;
    let month = parts[1].parse::<u8>().map_err(|_| {
        ParseError::with_message(ParseErrorCode::UnexpectedCharacter, line, col, &format!("invalid date: {raw}"))
    })?;
    let day = parts[2].parse::<u8>().map_err(|_| {
        ParseError::with_message(ParseErrorCode::UnexpectedCharacter, line, col, &format!("invalid date: {raw}"))
    })?;

    // Validate month
    if !(1..=12).contains(&month) {
        return Err(ParseError::with_message(
            ParseErrorCode::UnexpectedCharacter,
            line, col,
            &format!("Invalid month {month} in date {raw}"),
        ));
    }

    // Validate day
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

fn is_date_like(s: &str) -> bool {
    s.len() >= 10
        && s.as_bytes().get(4) == Some(&b'-')
        && s.as_bytes().get(7) == Some(&b'-')
        && s.as_bytes()[..4].iter().all(u8::is_ascii_digit)
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap_year(year) { 29 } else { 28 },
        _ => 0,
    }
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}
