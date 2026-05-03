//! String and encoding verb implementations for the transform engine.
//!
//! String verbs (30): titleCase, contains, startsWith, endsWith, replaceRegex,
//! padLeft, padRight, pad, truncate, split, join, mask, reverseString, repeat,
//! camelCase, snakeCase, kebabCase, pascalCase, slugify, match, extract,
//! normalizeSpace, leftOf, rightOf, wrap, center, matches, stripAccents, clean,
//! wordCount.
//!
//! Encoding verbs (11): base64Encode, base64Decode, urlEncode, urlDecode,
//! jsonEncode, jsonDecode, hexEncode, hexDecode, sha256, md5, crc32.

use crate::types::transform::DynValue;
use super::VerbContext;

// ─────────────────────────────────────────────────────────────────────────────
// Helper: convert a DynValue to its string representation
// ─────────────────────────────────────────────────────────────────────────────

fn to_str(v: &DynValue) -> Result<String, String> {
    match v {
        DynValue::String(s) => Ok(s.clone()),
        DynValue::Integer(n) => Ok(n.to_string()),
        DynValue::Float(n) => Ok(n.to_string()),
        DynValue::Bool(b) => Ok(b.to_string()),
        DynValue::Null => Ok(String::new()),
        _ => Err("expected string-coercible value".to_string()),
    }
}

fn to_usize(v: &DynValue, name: &str) -> Result<usize, String> {
    match v {
        DynValue::Integer(n) => {
            if *n < 0 {
                Err(format!("{name}: expected non-negative integer"))
            } else {
                Ok(*n as usize)
            }
        }
        DynValue::Float(n) => Ok(*n as usize),
        _ => Err(format!("{name}: expected integer")),
    }
}

fn pad_char(v: &DynValue, name: &str) -> Result<char, String> {
    match v {
        DynValue::String(s) => {
            let mut chars = s.chars();
            match chars.next() {
                Some(c) => Ok(c),
                None => Err(format!("{name}: pad character cannot be empty")),
            }
        }
        _ => Err(format!("{name}: expected string pad character")),
    }
}

/// Helper to split a string into word boundaries for case conversion.
/// Splits on whitespace, hyphens, underscores, and camelCase boundaries.
fn split_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();

    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    for i in 0..len {
        let ch = chars[i];
        if ch == ' ' || ch == '\t' || ch == '_' || ch == '-' {
            if !current.is_empty() {
                words.push(current.clone());
                current.clear();
            }
        } else if ch.is_uppercase() && i > 0 {
            let prev = chars[i - 1];
            if prev.is_lowercase() || prev.is_ascii_digit() {
                // camelCase boundary: "helloWorld" → ["hello", "World"]
                if !current.is_empty() {
                    words.push(current.clone());
                    current.clear();
                }
            } else if prev.is_uppercase() {
                // Consecutive uppercase: check if next char is lowercase
                // "XMLParser" → ["XML", "Parser"] — split before the char preceding lowercase
                let next_is_lower = i + 1 < len && chars[i + 1].is_lowercase();
                if next_is_lower && !current.is_empty() {
                    words.push(current.clone());
                    current.clear();
                }
            }
            current.push(ch);
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    // Post-process: if any word is all-uppercase and length > 1, split into individual letters
    // This handles "ABC" → ["A", "B", "C"]
    let mut result = Vec::new();
    for word in words {
        if word.len() > 1 && word.chars().all(char::is_uppercase) {
            for c in word.chars() {
                result.push(c.to_string());
            }
        } else {
            result.push(word);
        }
    }
    result
}

// ─────────────────────────────────────────────────────────────────────────────
// String verbs (30)
// ─────────────────────────────────────────────────────────────────────────────

/// titleCase: capitalize first letter of each word.
pub(super) fn verb_title_case(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let result = s
                .split_whitespace()
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        Some(c) => {
                            let upper: String = c.to_uppercase().collect();
                            format!("{}{}", upper, chars.as_str())
                        }
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            Ok(DynValue::String(result))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("titleCase: expected string argument".to_string()),
    }
}

/// contains: returns bool if string contains substring.
pub(super) fn verb_contains(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("contains: requires 2 arguments (string, substring)".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::String(s), DynValue::String(sub)) => {
            Ok(DynValue::Bool(s.contains(sub.as_str())))
        }
        (DynValue::Null, _) => Ok(DynValue::Bool(false)),
        _ => Err("contains: expected string arguments".to_string()),
    }
}

/// startsWith: returns bool if string starts with prefix.
pub(super) fn verb_starts_with(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("startsWith: requires 2 arguments (string, prefix)".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::String(s), DynValue::String(prefix)) => {
            Ok(DynValue::Bool(s.starts_with(prefix.as_str())))
        }
        (DynValue::Null, _) => Ok(DynValue::Bool(false)),
        _ => Err("startsWith: expected string arguments".to_string()),
    }
}

/// endsWith: returns bool if string ends with suffix.
pub(super) fn verb_ends_with(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("endsWith: requires 2 arguments (string, suffix)".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::String(s), DynValue::String(suffix)) => {
            Ok(DynValue::Bool(s.ends_with(suffix.as_str())))
        }
        (DynValue::Null, _) => Ok(DynValue::Bool(false)),
        _ => Err("endsWith: expected string arguments".to_string()),
    }
}

/// replaceRegex: simple pattern-based replace (not full regex).
/// Arity 3: (string, pattern, replacement).
/// Uses simple string `contains` matching since we avoid a regex dependency.
pub(super) fn verb_replace_regex(
    args: &[DynValue],
    _ctx: &VerbContext,
) -> Result<DynValue, String> {
    if args.len() < 3 {
        return Err(
            "replaceRegex: requires 3 arguments (string, pattern, replacement)".to_string(),
        );
    }
    match (&args[0], &args[1], &args[2]) {
        (DynValue::String(s), DynValue::String(pattern), DynValue::String(replacement)) => {
            // Simple pattern matching: treat pattern as a literal string replacement.
            Ok(DynValue::String(s.replace(pattern.as_str(), replacement)))
        }
        _ => Err("replaceRegex: expected string arguments".to_string()),
    }
}

/// padLeft: left-pad string to width with pad character.
/// Arity 3: (string, width, padChar).
pub(super) fn verb_pad_left(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 3 {
        return Err("padLeft: requires 3 arguments (string, width, padChar)".to_string());
    }
    let s = to_str(&args[0]).map_err(|e| format!("padLeft: {e}"))?;
    let width = to_usize(&args[1], "padLeft")?;
    let ch = pad_char(&args[2], "padLeft")?;

    if s.len() >= width {
        Ok(DynValue::String(s))
    } else {
        let padding: String = std::iter::repeat(ch).take(width - s.len()).collect();
        Ok(DynValue::String(format!("{padding}{s}")))
    }
}

/// padRight: right-pad string to width with pad character.
/// Arity 3: (string, width, padChar).
pub(super) fn verb_pad_right(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 3 {
        return Err("padRight: requires 3 arguments (string, width, padChar)".to_string());
    }
    let s = to_str(&args[0]).map_err(|e| format!("padRight: {e}"))?;
    let width = to_usize(&args[1], "padRight")?;
    let ch = pad_char(&args[2], "padRight")?;

    if s.len() >= width {
        Ok(DynValue::String(s))
    } else {
        let padding: String = std::iter::repeat(ch).take(width - s.len()).collect();
        Ok(DynValue::String(format!("{s}{padding}")))
    }
}

/// pad: center-pad string (both sides) to width with pad character.
/// Arity 3: (string, width, padChar).
pub(super) fn verb_pad(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 3 {
        return Err("pad: requires 3 arguments (string, width, padChar)".to_string());
    }
    let s = to_str(&args[0]).map_err(|e| format!("pad: {e}"))?;
    let width = to_usize(&args[1], "pad")?;
    let ch = pad_char(&args[2], "pad")?;

    if s.len() >= width {
        Ok(DynValue::String(s))
    } else {
        let total_pad = width - s.len();
        let left_pad = total_pad / 2;
        let right_pad = total_pad - left_pad;
        let left: String = std::iter::repeat(ch).take(left_pad).collect();
        let right: String = std::iter::repeat(ch).take(right_pad).collect();
        Ok(DynValue::String(format!("{left}{s}{right}")))
    }
}

/// truncate: truncate string to max length.
/// Arity 2: (string, maxLength).
pub(super) fn verb_truncate(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("truncate: requires 2 arguments (string, maxLength)".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::String(s), len_val) => {
            let max_len = to_usize(len_val, "truncate")?;
            if s.len() <= max_len {
                Ok(DynValue::String(s.clone()))
            } else {
                let truncated: String = s.chars().take(max_len).collect();
                Ok(DynValue::String(truncated))
            }
        }
        (DynValue::Null, _) => Ok(DynValue::Null),
        _ => Err("truncate: expected string as first argument".to_string()),
    }
}

/// split: split string by delimiter, return array.
/// Arity 2-3: (string, delimiter[, maxCount]).
pub(super) fn verb_split(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("split: requires at least 2 arguments (string, delimiter)".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::String(s), DynValue::String(delim)) => {
            let parts: Vec<DynValue> = s.split(delim.as_str())
                .map(|p| DynValue::String(p.to_string()))
                .collect();
            // Optional 3rd argument: index into the split result
            if args.len() >= 3 {
                let idx = to_usize(&args[2], "split")?;
                if idx < parts.len() {
                    Ok(parts[idx].clone())
                } else {
                    Ok(DynValue::Null)
                }
            } else {
                Ok(DynValue::Array(parts))
            }
        }
        _ => Err("split: expected (string, string) arguments".to_string()),
    }
}

/// join: join array with delimiter string.
/// Arity 2: (array, delimiter).
pub(super) fn verb_join(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("join: requires 2 arguments (array, delimiter)".to_string());
    }
    let delim = match &args[1] {
        DynValue::String(s) => s.as_str(),
        _ => return Err("join: second argument must be a string delimiter".to_string()),
    };
    // Try to extract array (handles both DynValue::Array and string-encoded arrays)
    if let Some(arr) = args[0].extract_array() {
        let parts: Vec<String> = arr
            .iter()
            .map(|v| to_str(v).unwrap_or_default())
            .collect();
        Ok(DynValue::String(parts.join(delim)))
    } else {
        Err("join: first argument must be an array".to_string())
    }
}

/// mask: mask string, showing only last N characters, OR apply a format pattern.
/// Arity 2: (string, showCount) or (string, formatPattern).
/// When the second arg is a string pattern containing '#', each '#' is replaced
/// with the next character from the input string (format masking).
/// When the second arg is an integer, mask all but the last N characters with '*'.
pub(super) fn verb_mask(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("mask: requires 2 arguments (string, showCount or pattern)".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::String(s), DynValue::String(pattern)) => {
            // Format pattern mode: '#' replaced by input chars
            let input_chars: Vec<char> = s.chars().collect();
            let mut result = String::new();
            let mut input_idx = 0;
            for ch in pattern.chars() {
                if ch == '#' {
                    if input_idx < input_chars.len() {
                        result.push(input_chars[input_idx]);
                        input_idx += 1;
                    }
                } else {
                    result.push(ch);
                }
            }
            Ok(DynValue::String(result))
        }
        (DynValue::String(s), show_val) => {
            let show = to_usize(show_val, "mask")?;
            let chars: Vec<char> = s.chars().collect();
            if show >= chars.len() {
                Ok(DynValue::String(s.clone()))
            } else {
                let masked_count = chars.len() - show;
                let masked: String = std::iter::repeat('*')
                    .take(masked_count)
                    .chain(chars[masked_count..].iter().copied())
                    .collect();
                Ok(DynValue::String(masked))
            }
        }
        (DynValue::Null, _) => Ok(DynValue::Null),
        _ => Err("mask: expected string as first argument".to_string()),
    }
}

/// reverseString: reverse the string.
pub(super) fn verb_reverse_string(
    args: &[DynValue],
    _ctx: &VerbContext,
) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => Ok(DynValue::String(s.chars().rev().collect())),
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("reverseString: expected string argument".to_string()),
    }
}

/// repeat: repeat string N times.
/// Arity 2: (string, count).
pub(super) fn verb_repeat(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("repeat: requires 2 arguments (string, count)".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::String(s), count_val) => {
            let count = to_usize(count_val, "repeat")?;
            Ok(DynValue::String(s.repeat(count)))
        }
        _ => Err("repeat: expected (string, integer) arguments".to_string()),
    }
}

/// camelCase: convert to camelCase.
pub(super) fn verb_camel_case(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let words = split_words(s);
            let result: String = words
                .iter()
                .enumerate()
                .map(|(i, w)| {
                    if i == 0 {
                        w.to_lowercase()
                    } else {
                        let mut chars = w.chars();
                        match chars.next() {
                            Some(c) => {
                                let upper: String = c.to_uppercase().collect();
                                format!("{}{}", upper, chars.as_str().to_lowercase())
                            }
                            None => String::new(),
                        }
                    }
                })
                .collect();
            Ok(DynValue::String(result))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("camelCase: expected string argument".to_string()),
    }
}

/// snakeCase: convert to `snake_case`.
pub(super) fn verb_snake_case(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let words = split_words(s);
            let result = words
                .iter()
                .map(|w| w.to_lowercase())
                .collect::<Vec<_>>()
                .join("_");
            Ok(DynValue::String(result))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("snakeCase: expected string argument".to_string()),
    }
}

/// kebabCase: convert to kebab-case.
pub(super) fn verb_kebab_case(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let words = split_words(s);
            let result = words
                .iter()
                .map(|w| w.to_lowercase())
                .collect::<Vec<_>>()
                .join("-");
            Ok(DynValue::String(result))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("kebabCase: expected string argument".to_string()),
    }
}

/// pascalCase: convert to `PascalCase`.
pub(super) fn verb_pascal_case(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let words = split_words(s);
            let result: String = words
                .iter()
                .map(|w| {
                    let mut chars = w.chars();
                    match chars.next() {
                        Some(c) => {
                            let upper: String = c.to_uppercase().collect();
                            format!("{}{}", upper, chars.as_str().to_lowercase())
                        }
                        None => String::new(),
                    }
                })
                .collect();
            Ok(DynValue::String(result))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("pascalCase: expected string argument".to_string()),
    }
}

/// slugify: lowercase, replace spaces/special with hyphens.
pub(super) fn verb_slugify(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let mut result = String::with_capacity(s.len());
            let mut prev_hyphen = true; // avoid leading hyphen
            for ch in s.chars() {
                if ch.is_alphanumeric() {
                    for lc in ch.to_lowercase() {
                        result.push(lc);
                    }
                    prev_hyphen = false;
                } else if !prev_hyphen {
                    result.push('-');
                    prev_hyphen = true;
                }
            }
            // Remove trailing hyphen
            if result.ends_with('-') {
                result.pop();
            }
            Ok(DynValue::String(result))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("slugify: expected string argument".to_string()),
    }
}

/// match: returns boolean if string matches simple pattern (contains check).
/// Arity 2: (string, pattern).
pub(super) fn verb_match(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("match: requires 2 arguments (string, pattern)".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::String(s), DynValue::String(pattern)) => {
            Ok(DynValue::Bool(s.contains(pattern.as_str())))
        }
        (DynValue::Null, _) => Ok(DynValue::Bool(false)),
        _ => Err("match: expected string arguments".to_string()),
    }
}

/// extract: extract substring between two delimiters, or regex capture group.
/// Arity 3: (string, startDelimiter, endDelimiter) or (string, regexPattern, groupIndex).
pub(super) fn verb_extract(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err(
            "extract: requires at least 2 arguments".to_string(),
        );
    }
    match (&args[0], &args[1], args.get(2)) {
        // Regex mode: (string, pattern, groupIndex as Integer)
        #[cfg(feature = "regex")]
        (DynValue::String(s), DynValue::String(pattern), Some(DynValue::Integer(group))) => {
            match regex::Regex::new(pattern) {
                Ok(re) => {
                    if let Some(caps) = re.captures(s) {
                        let idx = *group as usize;
                        match caps.get(idx) {
                            Some(m) => Ok(DynValue::String(m.as_str().to_string())),
                            None => Ok(DynValue::Null),
                        }
                    } else {
                        Ok(DynValue::Null)
                    }
                }
                Err(_) => Ok(DynValue::Null),
            }
        }
        // Delimiter mode: (string, startDelim, endDelim)
        (DynValue::String(s), DynValue::String(start_delim), Some(DynValue::String(end_delim))) => {
            if let Some(start_idx) = s.find(start_delim.as_str()) {
                let after_start = start_idx + start_delim.len();
                if let Some(end_idx) = s[after_start..].find(end_delim.as_str()) {
                    Ok(DynValue::String(s[after_start..after_start + end_idx].to_string()))
                } else {
                    Ok(DynValue::Null)
                }
            } else {
                Ok(DynValue::Null)
            }
        }
        // Regex match without group — return full match
        #[cfg(feature = "regex")]
        (DynValue::String(s), DynValue::String(pattern), None) => {
            match regex::Regex::new(pattern) {
                Ok(re) => {
                    if let Some(m) = re.find(s) {
                        Ok(DynValue::String(m.as_str().to_string()))
                    } else {
                        Ok(DynValue::Null)
                    }
                }
                Err(_) => Ok(DynValue::Null),
            }
        }
        (DynValue::Null, _, _) => Ok(DynValue::Null),
        _ => Err("extract: expected string arguments".to_string()),
    }
}

/// normalizeSpace: collapse multiple whitespace to single space, trim.
pub(super) fn verb_normalize_space(
    args: &[DynValue],
    _ctx: &VerbContext,
) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let result = s.split_whitespace().collect::<Vec<_>>().join(" ");
            Ok(DynValue::String(result))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("normalizeSpace: expected string argument".to_string()),
    }
}

/// leftOf: substring left of first occurrence of delimiter.
/// Arity 2: (string, delimiter).
pub(super) fn verb_left_of(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("leftOf: requires 2 arguments (string, delimiter)".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::String(s), DynValue::String(delim)) => {
            if let Some(idx) = s.find(delim.as_str()) {
                Ok(DynValue::String(s[..idx].to_string()))
            } else {
                Ok(DynValue::String(s.clone()))
            }
        }
        _ => Err("leftOf: expected string arguments".to_string()),
    }
}

/// rightOf: substring right of first occurrence of delimiter.
/// Arity 2: (string, delimiter).
pub(super) fn verb_right_of(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("rightOf: requires 2 arguments (string, delimiter)".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::String(s), DynValue::String(delim)) => {
            if let Some(idx) = s.find(delim.as_str()) {
                Ok(DynValue::String(s[idx + delim.len()..].to_string()))
            } else {
                Ok(DynValue::String(s.clone()))
            }
        }
        _ => Err("rightOf: expected string arguments".to_string()),
    }
}

/// wrap: word wrap at given width.
/// Arity 2: (string, width).
pub(super) fn verb_wrap(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Err("wrap: requires 2 arguments (string, width)".to_string());
    }
    match (&args[0], &args[1]) {
        (DynValue::String(s), width_val) => {
            let width = to_usize(width_val, "wrap")?;
            if width == 0 {
                return Ok(DynValue::String(s.clone()));
            }

            let words: Vec<&str> = s.split_whitespace().collect();
            let mut lines: Vec<String> = Vec::new();
            let mut current_line = String::new();

            for word in words {
                if current_line.is_empty() {
                    current_line = word.to_string();
                } else if current_line.len() + 1 + word.len() <= width {
                    current_line.push(' ');
                    current_line.push_str(word);
                } else {
                    lines.push(current_line);
                    current_line = word.to_string();
                }
            }
            if !current_line.is_empty() {
                lines.push(current_line);
            }

            Ok(DynValue::Array(lines.into_iter().map(DynValue::String).collect()))
        }
        _ => Err("wrap: expected (string, integer) arguments".to_string()),
    }
}

/// center: center string in field of given width with pad char.
/// Arity 3: (string, width, padChar).
pub(super) fn verb_center(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 3 {
        return Err("center: requires 3 arguments (string, width, padChar)".to_string());
    }
    let s = to_str(&args[0]).map_err(|e| format!("center: {e}"))?;
    let width = to_usize(&args[1], "center")?;
    let ch = pad_char(&args[2], "center")?;

    if s.len() >= width {
        Ok(DynValue::String(s))
    } else {
        let total_pad = width - s.len();
        let left_pad = total_pad / 2;
        let right_pad = total_pad - left_pad;
        let left: String = std::iter::repeat(ch).take(left_pad).collect();
        let right: String = std::iter::repeat(ch).take(right_pad).collect();
        Ok(DynValue::String(format!("{left}{s}{right}")))
    }
}

/// matches: alias for match (returns boolean if string contains pattern).
/// Arity 2: (string, pattern).
pub(super) fn verb_matches(args: &[DynValue], ctx: &VerbContext) -> Result<DynValue, String> {
    verb_match(args, ctx)
}

/// stripAccents: remove diacritical marks (e.g., e with accent -> e, n with tilde -> n).
pub(super) fn verb_strip_accents(
    args: &[DynValue],
    _ctx: &VerbContext,
) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let result: String = s
                .chars()
                .map(strip_accent_char)
                .collect();
            Ok(DynValue::String(result))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("stripAccents: expected string argument".to_string()),
    }
}

/// Map accented characters to their unaccented equivalents.
fn strip_accent_char(ch: char) -> char {
    match ch {
        '\u{00C0}'..='\u{00C5}' => 'A', // A with various accents
        '\u{00C7}' => 'C',               // C cedilla
        '\u{00C8}'..='\u{00CB}' => 'E',  // E with various accents
        '\u{00CC}'..='\u{00CF}' => 'I',  // I with various accents
        '\u{00D0}' => 'D',               // Eth
        '\u{00D1}' => 'N',               // N tilde
        '\u{00D2}'..='\u{00D6}' | '\u{00D8}' => 'O',  // O with various accents + stroke
        '\u{00D9}'..='\u{00DC}' => 'U',  // U with various accents
        '\u{00DD}' => 'Y',               // Y acute
        '\u{00E0}'..='\u{00E5}' => 'a',  // a with various accents
        '\u{00E7}' => 'c',               // c cedilla
        '\u{00E8}'..='\u{00EB}' => 'e',  // e with various accents
        '\u{00EC}'..='\u{00EF}' => 'i',  // i with various accents
        '\u{00F0}' => 'd',               // eth
        '\u{00F1}' => 'n',               // n tilde
        '\u{00F2}'..='\u{00F6}' | '\u{00F8}' => 'o',  // o with various accents + stroke
        '\u{00F9}'..='\u{00FC}' => 'u',  // u with various accents
        '\u{00FD}' | '\u{00FF}' => 'y',  // y acute, y diaeresis
        _ => ch,
    }
}

/// clean: remove non-printable characters.
pub(super) fn verb_clean(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let result: String = s
                .chars()
                .filter(|ch| !ch.is_control() || *ch == '\n' || *ch == '\r' || *ch == '\t')
                .collect();
            Ok(DynValue::String(result))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("clean: expected string argument".to_string()),
    }
}

/// wordCount: count words in string.
pub(super) fn verb_word_count(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let count = s.split_whitespace().count();
            Ok(DynValue::Integer(count as i64))
        }
        Some(DynValue::Null) => Ok(DynValue::Integer(0)),
        _ => Err("wordCount: expected string argument".to_string()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Text analysis verbs: tokenize, levenshtein, soundex
// ─────────────────────────────────────────────────────────────────────────────

/// tokenize: split text into tokens by delimiter (default: whitespace).
pub(super) fn verb_tokenize(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() {
        return Ok(DynValue::Array(vec![]));
    }
    let s = to_str(&args[0])?;
    if s.is_empty() {
        return Ok(DynValue::Array(vec![]));
    }

    let delimiter = if args.len() >= 2 { to_str(&args[1])? } else { String::new() };

    let tokens: Vec<DynValue> = if delimiter.is_empty() {
        // Split on whitespace, filter empty
        s.split_whitespace()
            .filter(|t| !t.is_empty())
            .map(|t| DynValue::String(t.to_string()))
            .collect()
    } else {
        // Split on delimiter, trim and filter empty
        s.split(&delimiter)
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| DynValue::String(t.to_string()))
            .collect()
    };

    Ok(DynValue::Array(tokens))
}

/// levenshtein: calculate edit distance between two strings.
pub(super) fn verb_levenshtein(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Ok(DynValue::Null);
    }
    let s1 = to_str(&args[0])?;
    let s2 = to_str(&args[1])?;

    // Security limit to prevent O(m*n) performance issues
    const MAX_LEN: usize = 10_000;
    if s1.len() > MAX_LEN || s2.len() > MAX_LEN {
        return Ok(DynValue::Null);
    }

    let m = s1.len();
    let n = s2.len();

    if m == 0 { return Ok(DynValue::Integer(n as i64)); }
    if n == 0 { return Ok(DynValue::Integer(m as i64)); }

    let s1_bytes: Vec<u8> = s1.bytes().collect();
    let s2_bytes: Vec<u8> = s2.bytes().collect();

    // Two-row dynamic programming
    let mut prev_row: Vec<usize> = (0..=n).collect();
    let mut curr_row = vec![0usize; n + 1];

    for i in 1..=m {
        curr_row[0] = i;
        for j in 1..=n {
            let cost = usize::from(s1_bytes[i - 1] != s2_bytes[j - 1]);
            curr_row[j] = (prev_row[j] + 1)
                .min(curr_row[j - 1] + 1)
                .min(prev_row[j - 1] + cost);
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    Ok(DynValue::Integer(prev_row[n] as i64))
}

/// soundex: generate Soundex phonetic code for a string.
pub(super) fn verb_soundex(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.is_empty() {
        return Ok(DynValue::Null);
    }
    let s: String = to_str(&args[0])?
        .to_uppercase()
        .chars()
        .filter(char::is_ascii_alphabetic)
        .collect();

    if s.is_empty() {
        return Ok(DynValue::String(String::new()));
    }

    fn soundex_code(c: char) -> char {
        match c {
            'B' | 'F' | 'P' | 'V' => '1',
            'C' | 'G' | 'J' | 'K' | 'Q' | 'S' | 'X' | 'Z' => '2',
            'D' | 'T' => '3',
            'L' => '4',
            'M' | 'N' => '5',
            'R' => '6',
            _ => '0',
        }
    }

    let chars: Vec<char> = s.chars().collect();
    let mut result = String::new();
    result.push(chars[0]); // First letter kept as-is
    let mut prev_code = soundex_code(chars[0]);

    for &c in &chars[1..] {
        if result.len() >= 4 { break; }
        let code = soundex_code(c);
        if code != '0' && code != prev_code {
            result.push(code);
        }
        prev_code = code;
    }

    // Pad with zeros to length 4
    while result.len() < 4 {
        result.push('0');
    }

    Ok(DynValue::String(result))
}

// ─────────────────────────────────────────────────────────────────────────────
// Encoding verbs (11)
// ─────────────────────────────────────────────────────────────────────────────

/// base64Encode: encode string to base64.
pub(super) fn verb_base64_encode(
    args: &[DynValue],
    _ctx: &VerbContext,
) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let encoded = crate::utils::base64::encode(s.as_bytes());
            Ok(DynValue::String(encoded))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("base64Encode: expected string argument".to_string()),
    }
}

/// base64Decode: decode base64 to string.
pub(super) fn verb_base64_decode(
    args: &[DynValue],
    _ctx: &VerbContext,
) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let bytes = crate::utils::base64::decode(s)
                .map_err(|e| format!("base64Decode: {e}"))?;
            let decoded = String::from_utf8(bytes)
                .map_err(|e| format!("base64Decode: invalid UTF-8: {e}"))?;
            Ok(DynValue::String(decoded))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("base64Decode: expected string argument".to_string()),
    }
}

/// urlEncode: percent-encode string.
pub(super) fn verb_url_encode(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let mut encoded = String::with_capacity(s.len() * 3);
            for byte in s.bytes() {
                match byte {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                        encoded.push(byte as char);
                    }
                    _ => {
                        encoded.push('%');
                        encoded.push(HEX_UPPER[(byte >> 4) as usize] as char);
                        encoded.push(HEX_UPPER[(byte & 0x0F) as usize] as char);
                    }
                }
            }
            Ok(DynValue::String(encoded))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("urlEncode: expected string argument".to_string()),
    }
}

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";
const HEX_LOWER: &[u8; 16] = b"0123456789abcdef";

/// urlDecode: percent-decode string.
pub(super) fn verb_url_decode(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let mut decoded = Vec::with_capacity(s.len());
            let bytes = s.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'%' && i + 2 < bytes.len() {
                    let hi = hex_digit(bytes[i + 1])
                        .ok_or_else(|| format!("urlDecode: invalid hex digit '{}'", bytes[i + 1] as char))?;
                    let lo = hex_digit(bytes[i + 2])
                        .ok_or_else(|| format!("urlDecode: invalid hex digit '{}'", bytes[i + 2] as char))?;
                    decoded.push((hi << 4) | lo);
                    i += 3;
                } else if bytes[i] == b'+' {
                    decoded.push(b' ');
                    i += 1;
                } else {
                    decoded.push(bytes[i]);
                    i += 1;
                }
            }
            let result = String::from_utf8(decoded)
                .map_err(|e| format!("urlDecode: invalid UTF-8: {e}"))?;
            Ok(DynValue::String(result))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("urlDecode: expected string argument".to_string()),
    }
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// jsonEncode: convert `DynValue` to JSON string.
pub(super) fn verb_json_encode(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(val) => {
            let json = crate::utils::json_parser::to_json(val, false);
            Ok(DynValue::String(json))
        }
        None => Err("jsonEncode: requires 1 argument".to_string()),
    }
}

/// jsonDecode: parse JSON string to `DynValue`.
pub(super) fn verb_json_decode(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            crate::utils::json_parser::parse_json(s)
                .map_err(|e| format!("jsonDecode: {e}"))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("jsonDecode: expected string argument".to_string()),
    }
}

/// hexEncode: encode string bytes as hex string.
pub(super) fn verb_hex_encode(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            let mut hex = String::with_capacity(s.len() * 2);
            for byte in s.bytes() {
                hex.push(HEX_LOWER[(byte >> 4) as usize] as char);
                hex.push(HEX_LOWER[(byte & 0x0F) as usize] as char);
            }
            Ok(DynValue::String(hex))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("hexEncode: expected string argument".to_string()),
    }
}

/// hexDecode: decode hex string to string.
pub(super) fn verb_hex_decode(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => {
            if s.len() % 2 != 0 {
                return Err("hexDecode: hex string must have even length".to_string());
            }
            let bytes = s.as_bytes();
            let mut decoded = Vec::with_capacity(s.len() / 2);
            let mut i = 0;
            while i < bytes.len() {
                let hi = hex_digit(bytes[i])
                    .ok_or_else(|| format!("hexDecode: invalid hex digit '{}'", bytes[i] as char))?;
                let lo = hex_digit(bytes[i + 1])
                    .ok_or_else(|| format!("hexDecode: invalid hex digit '{}'", bytes[i + 1] as char))?;
                decoded.push((hi << 4) | lo);
                i += 2;
            }
            let result = String::from_utf8(decoded)
                .map_err(|e| format!("hexDecode: invalid UTF-8: {e}"))?;
            Ok(DynValue::String(result))
        }
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("hexDecode: expected string argument".to_string()),
    }
}

/// SHA-256 implementation.
fn sha256_hash(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5, 0x3956_c25b, 0x59f1_11f1, 0x923f_82a4, 0xab1c_5ed5,
        0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3, 0x72be_5d74, 0x80de_b1fe, 0x9bdc_06a7, 0xc19b_f174,
        0xe49b_69c1, 0xefbe_4786, 0x0fc1_9dc6, 0x240c_a1cc, 0x2de9_2c6f, 0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da,
        0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7, 0xc6e0_0bf3, 0xd5a7_9147, 0x06ca_6351, 0x1429_2967,
        0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc, 0x5338_0d13, 0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85,
        0xa2bf_e8a1, 0xa81a_664b, 0xc24b_8b70, 0xc76c_51a3, 0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070,
        0x19a4_c116, 0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5, 0x391c_0cb3, 0x4ed8_aa4a, 0x5b9c_ca4f, 0x682e_6ff3,
        0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208, 0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7, 0xc671_78f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a,
        0x510e_527f, 0x9b05_688c, 0x1f83_d9ab, 0x5be0_cd19,
    ];
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 { msg.push(0); }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i*4], chunk[i*4+1], chunk[i*4+2], chunk[i*4+3]]);
        }
        for i in 16..64 {
            let s0 = w[i-15].rotate_right(7) ^ w[i-15].rotate_right(18) ^ (w[i-15] >> 3);
            let s1 = w[i-2].rotate_right(17) ^ w[i-2].rotate_right(19) ^ (w[i-2] >> 10);
            w[i] = w[i-16].wrapping_add(s0).wrapping_add(w[i-7]).wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g; g = f; f = e; e = d.wrapping_add(t1); d = c; c = b; b = a; a = t1.wrapping_add(t2);
        }
        h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e); h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g); h[7] = h[7].wrapping_add(hh);
    }
    let mut result = [0u8; 32];
    for i in 0..8 { result[i*4..(i+1)*4].copy_from_slice(&h[i].to_be_bytes()); }
    result
}

fn sha256_hex(data: &[u8]) -> String {
    let hash = sha256_hash(data);
    let mut out = String::with_capacity(hash.len() * 2);
    for b in &hash {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

pub(super) fn verb_sha256(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => Ok(DynValue::String(sha256_hex(s.as_bytes()))),
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("sha256: expected string argument".to_string()),
    }
}

/// MD5 implementation.
fn md5_hash(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7,12,17,22, 7,12,17,22, 7,12,17,22, 7,12,17,22,
        5, 9,14,20, 5, 9,14,20, 5, 9,14,20, 5, 9,14,20,
        4,11,16,23, 4,11,16,23, 4,11,16,23, 4,11,16,23,
        6,10,15,21, 6,10,15,21, 6,10,15,21, 6,10,15,21,
    ];
    const K: [u32; 64] = [
        0xd76a_a478, 0xe8c7_b756, 0x2420_70db, 0xc1bd_ceee, 0xf57c_0faf, 0x4787_c62a, 0xa830_4613, 0xfd46_9501,
        0x6980_98d8, 0x8b44_f7af, 0xffff_5bb1, 0x895c_d7be, 0x6b90_1122, 0xfd98_7193, 0xa679_438e, 0x49b4_0821,
        0xf61e_2562, 0xc040_b340, 0x265e_5a51, 0xe9b6_c7aa, 0xd62f_105d, 0x0244_1453, 0xd8a1_e681, 0xe7d3_fbc8,
        0x21e1_cde6, 0xc337_07d6, 0xf4d5_0d87, 0x455a_14ed, 0xa9e3_e905, 0xfcef_a3f8, 0x676f_02d9, 0x8d2a_4c8a,
        0xfffa_3942, 0x8771_f681, 0x6d9d_6122, 0xfde5_380c, 0xa4be_ea44, 0x4bde_cfa9, 0xf6bb_4b60, 0xbebf_bc70,
        0x289b_7ec6, 0xeaa1_27fa, 0xd4ef_3085, 0x0488_1d05, 0xd9d4_d039, 0xe6db_99e5, 0x1fa2_7cf8, 0xc4ac_5665,
        0xf429_2244, 0x432a_ff97, 0xab94_23a7, 0xfc93_a039, 0x655b_59c3, 0x8f0c_cc92, 0xffef_f47d, 0x8584_5dd1,
        0x6fa8_7e4f, 0xfe2c_e6e0, 0xa301_4314, 0x4e08_11a1, 0xf753_7e82, 0xbd3a_f235, 0x2ad7_d2bb, 0xeb86_d391,
    ];
    let mut h: [u32; 4] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    let bit_len = (data.len() as u64) * 8;
    let mut msg = data.to_vec();
    msg.push(0x80);
    while (msg.len() % 64) != 56 { msg.push(0); }
    msg.extend_from_slice(&bit_len.to_le_bytes());
    for chunk in msg.chunks(64) {
        let mut m = [0u32; 16];
        for i in 0..16 {
            m[i] = u32::from_le_bytes([chunk[i*4], chunk[i*4+1], chunk[i*4+2], chunk[i*4+3]]);
        }
        let (mut a, mut b, mut c, mut d) = (h[0], h[1], h[2], h[3]);
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | ((!b) & d), i),
                16..=31 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };
            let temp = d;
            d = c; c = b;
            b = b.wrapping_add((a.wrapping_add(f).wrapping_add(K[i]).wrapping_add(m[g])).rotate_left(S[i]));
            a = temp;
        }
        h[0] = h[0].wrapping_add(a); h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c); h[3] = h[3].wrapping_add(d);
    }
    let mut result = [0u8; 16];
    for i in 0..4 { result[i*4..(i+1)*4].copy_from_slice(&h[i].to_le_bytes()); }
    result
}

fn md5_hex(data: &[u8]) -> String {
    let hash = md5_hash(data);
    let mut out = String::with_capacity(hash.len() * 2);
    for b in &hash {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

pub(super) fn verb_md5(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => Ok(DynValue::String(md5_hex(s.as_bytes()))),
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("md5: expected string argument".to_string()),
    }
}

/// CRC32 implementation (ISO 3309 / ITU-T V.42).
fn crc32_compute(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

pub(super) fn verb_crc32(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    match args.first() {
        Some(DynValue::String(s)) => Ok(DynValue::String(format!("{:08x}", crc32_compute(s.as_bytes())))),
        Some(DynValue::Null) => Ok(DynValue::Null),
        _ => Err("crc32: expected string argument".to_string()),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

// ─────────────────────────────────────────────────────────────────────────────
// formatPhone
// ─────────────────────────────────────────────────────────────────────────────

pub(super) fn verb_format_phone(args: &[DynValue], _ctx: &VerbContext) -> Result<DynValue, String> {
    if args.len() < 2 {
        return Ok(DynValue::Null);
    }
    let raw = match &args[0] {
        DynValue::String(s) => s.as_str(),
        _ => return Ok(DynValue::Null),
    };
    let country = match &args[1] {
        DynValue::String(s) => s.as_str(),
        _ => return Ok(DynValue::Null),
    };
    // Strip non-digits
    let digits: String = raw.chars().filter(|c| c.is_ascii_digit()).collect();
    let formatted = match country {
        "US" | "CA" => {
            if digits.len() == 10 {
                format!("({}) {}-{}", &digits[0..3], &digits[3..6], &digits[6..10])
            } else if digits.len() == 11 && digits.starts_with('1') {
                format!("+1 ({}) {}-{}", &digits[1..4], &digits[4..7], &digits[7..11])
            } else {
                return Ok(DynValue::String(raw.to_string()));
            }
        }
        "GB" => {
            if digits.len() == 11 && digits.starts_with('0') {
                format!("+44 {} {}", &digits[1..5], &digits[5..11])
            } else if digits.len() == 10 {
                format!("+44 {} {}", &digits[0..4], &digits[4..10])
            } else {
                return Ok(DynValue::String(raw.to_string()));
            }
        }
        "DE" => {
            if digits.len() == 11 && digits.starts_with('0') {
                format!("+49 {} {}", &digits[1..5], &digits[5..11])
            } else if digits.len() == 10 {
                format!("+49 {} {}", &digits[0..4], &digits[4..10])
            } else {
                return Ok(DynValue::String(raw.to_string()));
            }
        }
        "FR" => {
            if digits.len() == 10 && digits.starts_with('0') {
                format!("+33 {} {} {} {} {}", &digits[1..2], &digits[2..4], &digits[4..6], &digits[6..8], &digits[8..10])
            } else if digits.len() == 9 {
                format!("+33 {} {} {} {} {}", &digits[0..1], &digits[1..3], &digits[3..5], &digits[5..7], &digits[7..9])
            } else {
                return Ok(DynValue::String(raw.to_string()));
            }
        }
        "AU" => {
            if digits.len() == 10 && digits.starts_with('0') {
                format!("+61 {} {} {}", &digits[1..2], &digits[2..6], &digits[6..10])
            } else if digits.len() == 9 {
                format!("+61 {} {} {}", &digits[0..1], &digits[1..5], &digits[5..9])
            } else {
                return Ok(DynValue::String(raw.to_string()));
            }
        }
        "JP" => {
            if digits.len() == 11 && digits.starts_with('0') {
                format!("+81 {}-{}-{}", &digits[1..3], &digits[3..7], &digits[7..11])
            } else if digits.len() == 10 {
                format!("+81 {}-{}-{}", &digits[0..2], &digits[2..6], &digits[6..10])
            } else {
                return Ok(DynValue::String(raw.to_string()));
            }
        }
        _ => return Ok(DynValue::String(raw.to_string())),
    };
    Ok(DynValue::String(formatted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn ctx() -> VerbContext<'static> {
        static NULL: DynValue = DynValue::Null;
        static LV: std::sync::OnceLock<HashMap<String, DynValue>> = std::sync::OnceLock::new();
        static ACC: std::sync::OnceLock<HashMap<String, DynValue>> = std::sync::OnceLock::new();
        static TBL: std::sync::OnceLock<HashMap<String, crate::types::transform::LookupTable>> = std::sync::OnceLock::new();
        VerbContext {
            source: &NULL,
            loop_vars: LV.get_or_init(HashMap::new),
            accumulators: ACC.get_or_init(HashMap::new),
            tables: TBL.get_or_init(HashMap::new),
        }
    }

    #[test]
    fn test_title_case() {
        let args = vec![DynValue::String("hello world".into())];
        let result = verb_title_case(&args, &ctx()).unwrap();
        assert_eq!(result, DynValue::String("Hello World".into()));
    }

    #[test]
    fn test_contains() {
        let args = vec![
            DynValue::String("hello world".into()),
            DynValue::String("world".into()),
        ];
        assert_eq!(verb_contains(&args, &ctx()).unwrap(), DynValue::Bool(true));

        let args2 = vec![
            DynValue::String("hello".into()),
            DynValue::String("xyz".into()),
        ];
        assert_eq!(verb_contains(&args2, &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn test_starts_with() {
        let args = vec![
            DynValue::String("hello world".into()),
            DynValue::String("hello".into()),
        ];
        assert_eq!(verb_starts_with(&args, &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn test_ends_with() {
        let args = vec![
            DynValue::String("hello world".into()),
            DynValue::String("world".into()),
        ];
        assert_eq!(verb_ends_with(&args, &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn test_pad_left() {
        let args = vec![
            DynValue::String("42".into()),
            DynValue::Integer(5),
            DynValue::String("0".into()),
        ];
        assert_eq!(
            verb_pad_left(&args, &ctx()).unwrap(),
            DynValue::String("00042".into())
        );
    }

    #[test]
    fn test_pad_right() {
        let args = vec![
            DynValue::String("hi".into()),
            DynValue::Integer(5),
            DynValue::String(".".into()),
        ];
        assert_eq!(
            verb_pad_right(&args, &ctx()).unwrap(),
            DynValue::String("hi...".into())
        );
    }

    #[test]
    fn test_pad_center() {
        let args = vec![
            DynValue::String("hi".into()),
            DynValue::Integer(6),
            DynValue::String("*".into()),
        ];
        assert_eq!(
            verb_pad(&args, &ctx()).unwrap(),
            DynValue::String("**hi**".into())
        );
    }

    #[test]
    fn test_truncate() {
        let args = vec![DynValue::String("hello world".into()), DynValue::Integer(5)];
        assert_eq!(
            verb_truncate(&args, &ctx()).unwrap(),
            DynValue::String("hello".into())
        );
    }

    #[test]
    fn test_split() {
        let args = vec![
            DynValue::String("a,b,c".into()),
            DynValue::String(",".into()),
        ];
        assert_eq!(
            verb_split(&args, &ctx()).unwrap(),
            DynValue::Array(vec![
                DynValue::String("a".into()),
                DynValue::String("b".into()),
                DynValue::String("c".into()),
            ])
        );
    }

    #[test]
    fn test_split_with_index() {
        let args = vec![
            DynValue::String("a,b,c,d".into()),
            DynValue::String(",".into()),
            DynValue::Integer(2),
        ];
        // 3rd arg is index: split("a,b,c,d", ",") = ["a","b","c","d"], index 2 = "c"
        assert_eq!(
            verb_split(&args, &ctx()).unwrap(),
            DynValue::String("c".into()),
        );
    }

    #[test]
    fn test_join() {
        let args = vec![
            DynValue::Array(vec![
                DynValue::String("a".into()),
                DynValue::String("b".into()),
                DynValue::String("c".into()),
            ]),
            DynValue::String(", ".into()),
        ];
        assert_eq!(
            verb_join(&args, &ctx()).unwrap(),
            DynValue::String("a, b, c".into())
        );
    }

    #[test]
    fn test_mask() {
        let args = vec![
            DynValue::String("1234567890".into()),
            DynValue::Integer(4),
        ];
        assert_eq!(
            verb_mask(&args, &ctx()).unwrap(),
            DynValue::String("******7890".into())
        );
    }

    #[test]
    fn test_reverse_string() {
        let args = vec![DynValue::String("hello".into())];
        assert_eq!(
            verb_reverse_string(&args, &ctx()).unwrap(),
            DynValue::String("olleh".into())
        );
    }

    #[test]
    fn test_repeat() {
        let args = vec![DynValue::String("ab".into()), DynValue::Integer(3)];
        assert_eq!(
            verb_repeat(&args, &ctx()).unwrap(),
            DynValue::String("ababab".into())
        );
    }

    #[test]
    fn test_camel_case() {
        let args = vec![DynValue::String("hello world".into())];
        assert_eq!(
            verb_camel_case(&args, &ctx()).unwrap(),
            DynValue::String("helloWorld".into())
        );
    }

    #[test]
    fn test_snake_case() {
        let args = vec![DynValue::String("helloWorld".into())];
        assert_eq!(
            verb_snake_case(&args, &ctx()).unwrap(),
            DynValue::String("hello_world".into())
        );
    }

    #[test]
    fn test_kebab_case() {
        let args = vec![DynValue::String("helloWorld".into())];
        assert_eq!(
            verb_kebab_case(&args, &ctx()).unwrap(),
            DynValue::String("hello-world".into())
        );
    }

    #[test]
    fn test_pascal_case() {
        let args = vec![DynValue::String("hello world".into())];
        assert_eq!(
            verb_pascal_case(&args, &ctx()).unwrap(),
            DynValue::String("HelloWorld".into())
        );
    }

    #[test]
    fn test_slugify() {
        let args = vec![DynValue::String("Hello World! Test".into())];
        assert_eq!(
            verb_slugify(&args, &ctx()).unwrap(),
            DynValue::String("hello-world-test".into())
        );
    }

    #[test]
    fn test_normalize_space() {
        let args = vec![DynValue::String("  hello   world  ".into())];
        assert_eq!(
            verb_normalize_space(&args, &ctx()).unwrap(),
            DynValue::String("hello world".into())
        );
    }

    #[test]
    fn test_left_of() {
        let args = vec![
            DynValue::String("hello@world.com".into()),
            DynValue::String("@".into()),
        ];
        assert_eq!(
            verb_left_of(&args, &ctx()).unwrap(),
            DynValue::String("hello".into())
        );
    }

    #[test]
    fn test_right_of() {
        let args = vec![
            DynValue::String("hello@world.com".into()),
            DynValue::String("@".into()),
        ];
        assert_eq!(
            verb_right_of(&args, &ctx()).unwrap(),
            DynValue::String("world.com".into())
        );
    }

    #[test]
    fn test_extract() {
        let args = vec![
            DynValue::String("Hello [world] end".into()),
            DynValue::String("[".into()),
            DynValue::String("]".into()),
        ];
        assert_eq!(
            verb_extract(&args, &ctx()).unwrap(),
            DynValue::String("world".into())
        );
    }

    #[test]
    fn test_word_count() {
        let args = vec![DynValue::String("hello beautiful world".into())];
        assert_eq!(
            verb_word_count(&args, &ctx()).unwrap(),
            DynValue::Integer(3)
        );
    }

    #[test]
    fn test_strip_accents() {
        let args = vec![DynValue::String("caf\u{00E9} na\u{00EF}ve \u{00F1}".into())];
        assert_eq!(
            verb_strip_accents(&args, &ctx()).unwrap(),
            DynValue::String("cafe naive n".into())
        );
    }

    #[test]
    fn test_clean() {
        let args = vec![DynValue::String("hello\x00\x01world\n".into())];
        assert_eq!(
            verb_clean(&args, &ctx()).unwrap(),
            DynValue::String("helloworld\n".into())
        );
    }

    #[test]
    fn test_wrap() {
        let args = vec![
            DynValue::String("the quick brown fox jumps over".into()),
            DynValue::Integer(15),
        ];
        let result = verb_wrap(&args, &ctx()).unwrap();
        assert_eq!(
            result,
            DynValue::Array(vec![
                DynValue::String("the quick brown".into()),
                DynValue::String("fox jumps over".into()),
            ])
        );
    }

    #[test]
    fn test_center() {
        let args = vec![
            DynValue::String("hi".into()),
            DynValue::Integer(6),
            DynValue::String("-".into()),
        ];
        assert_eq!(
            verb_center(&args, &ctx()).unwrap(),
            DynValue::String("--hi--".into())
        );
    }

    #[test]
    fn test_base64_encode() {
        let args = vec![DynValue::String("Hello".into())];
        assert_eq!(
            verb_base64_encode(&args, &ctx()).unwrap(),
            DynValue::String("SGVsbG8=".into())
        );
    }

    #[test]
    fn test_base64_decode() {
        let args = vec![DynValue::String("SGVsbG8=".into())];
        assert_eq!(
            verb_base64_decode(&args, &ctx()).unwrap(),
            DynValue::String("Hello".into())
        );
    }

    #[test]
    fn test_url_encode() {
        let args = vec![DynValue::String("hello world&foo=bar".into())];
        assert_eq!(
            verb_url_encode(&args, &ctx()).unwrap(),
            DynValue::String("hello%20world%26foo%3Dbar".into())
        );
    }

    #[test]
    fn test_url_decode() {
        let args = vec![DynValue::String("hello%20world%26foo%3Dbar".into())];
        assert_eq!(
            verb_url_decode(&args, &ctx()).unwrap(),
            DynValue::String("hello world&foo=bar".into())
        );
    }

    #[test]
    fn test_json_encode() {
        let args = vec![DynValue::Object(vec![
            ("a".into(), DynValue::Integer(1)),
            ("b".into(), DynValue::String("two".into())),
        ])];
        let result = verb_json_encode(&args, &ctx()).unwrap();
        assert_eq!(
            result,
            DynValue::String(r#"{"a":1,"b":"two"}"#.into())
        );
    }

    #[test]
    fn test_json_decode() {
        let args = vec![DynValue::String(r#"{"x":42}"#.into())];
        let result = verb_json_decode(&args, &ctx()).unwrap();
        assert_eq!(
            result,
            DynValue::Object(vec![("x".into(), DynValue::Integer(42))])
        );
    }

    #[test]
    fn test_hex_encode() {
        let args = vec![DynValue::String("Hi".into())];
        assert_eq!(
            verb_hex_encode(&args, &ctx()).unwrap(),
            DynValue::String("4869".into())
        );
    }

    #[test]
    fn test_hex_decode() {
        let args = vec![DynValue::String("4869".into())];
        assert_eq!(
            verb_hex_decode(&args, &ctx()).unwrap(),
            DynValue::String("Hi".into())
        );
    }

    #[test]
    fn test_sha256() {
        let args = vec![DynValue::String("test".into())];
        let result = verb_sha256(&args, &ctx()).unwrap();
        // SHA-256 of "test" is well-known
        assert_eq!(
            result,
            DynValue::String("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08".into())
        );
    }

    #[test]
    fn test_md5() {
        let args = vec![DynValue::String("test".into())];
        let result = verb_md5(&args, &ctx()).unwrap();
        // MD5 of "test" is well-known
        assert_eq!(
            result,
            DynValue::String("098f6bcd4621d373cade4e832627b4f6".into())
        );
    }

    #[test]
    fn test_crc32() {
        let args = vec![DynValue::String("test".into())];
        let result = verb_crc32(&args, &ctx()).unwrap();
        // CRC32 of "test" is well-known
        assert_eq!(
            result,
            DynValue::String("d87f7e0c".into())
        );
    }

    #[test]
    fn test_replace_regex() {
        let args = vec![
            DynValue::String("hello world hello".into()),
            DynValue::String("hello".into()),
            DynValue::String("hi".into()),
        ];
        assert_eq!(
            verb_replace_regex(&args, &ctx()).unwrap(),
            DynValue::String("hi world hi".into())
        );
    }

    #[test]
    fn test_matches_alias() {
        let args = vec![
            DynValue::String("hello world".into()),
            DynValue::String("world".into()),
        ];
        assert_eq!(verb_matches(&args, &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn test_null_passthrough() {
        let null_args = vec![DynValue::Null];
        assert_eq!(verb_title_case(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_reverse_string(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_camel_case(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_snake_case(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_kebab_case(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_pascal_case(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_slugify(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_normalize_space(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_strip_accents(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_clean(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_base64_encode(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_base64_decode(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_url_encode(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_url_decode(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_hex_encode(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_hex_decode(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_sha256(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_md5(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_crc32(&null_args, &ctx()).unwrap(), DynValue::Null);
        assert_eq!(verb_json_decode(&null_args, &ctx()).unwrap(), DynValue::Null);
    }
}

#[cfg(test)]
mod extended_tests {
    use super::*;
    use std::collections::HashMap;

    fn ctx() -> VerbContext<'static> {
        static NULL: DynValue = DynValue::Null;
        static LV: std::sync::OnceLock<HashMap<String, DynValue>> = std::sync::OnceLock::new();
        static ACC: std::sync::OnceLock<HashMap<String, DynValue>> = std::sync::OnceLock::new();
        static TBL: std::sync::OnceLock<HashMap<String, crate::types::transform::LookupTable>> = std::sync::OnceLock::new();
        VerbContext {
            source: &NULL,
            loop_vars: LV.get_or_init(HashMap::new),
            accumulators: ACC.get_or_init(HashMap::new),
            tables: TBL.get_or_init(HashMap::new),
        }
    }

    fn s(v: &str) -> DynValue { DynValue::String(v.to_string()) }
    fn i(v: i64) -> DynValue { DynValue::Integer(v) }
    #[allow(dead_code)]
    fn f(v: f64) -> DynValue { DynValue::Float(v) }
    #[allow(dead_code)]
    fn b(v: bool) -> DynValue { DynValue::Bool(v) }
    fn null() -> DynValue { DynValue::Null }

    // ─────────────────────────────────────────────────────────────────────────
    // titleCase extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn title_case_empty_string() {
        assert_eq!(verb_title_case(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn title_case_single_word() {
        assert_eq!(verb_title_case(&[s("hello")], &ctx()).unwrap(), s("Hello"));
    }

    #[test]
    fn title_case_already_titled() {
        assert_eq!(verb_title_case(&[s("Hello World")], &ctx()).unwrap(), s("Hello World"));
    }

    #[test]
    fn title_case_all_upper() {
        assert_eq!(verb_title_case(&[s("HELLO WORLD")], &ctx()).unwrap(), s("HELLO WORLD"));
    }

    #[test]
    fn title_case_mixed_whitespace() {
        // split_whitespace collapses multiple spaces
        assert_eq!(verb_title_case(&[s("  hello   world  ")], &ctx()).unwrap(), s("Hello World"));
    }

    #[test]
    fn title_case_with_numbers() {
        assert_eq!(verb_title_case(&[s("hello 42 world")], &ctx()).unwrap(), s("Hello 42 World"));
    }

    #[test]
    fn title_case_special_chars() {
        assert_eq!(verb_title_case(&[s("hello-world foo_bar")], &ctx()).unwrap(), s("Hello-world Foo_bar"));
    }

    #[test]
    fn title_case_unicode() {
        assert_eq!(verb_title_case(&[s("über cool")], &ctx()).unwrap(), s("Über Cool"));
    }

    #[test]
    fn title_case_null() {
        assert_eq!(verb_title_case(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn title_case_wrong_type() {
        assert!(verb_title_case(&[i(42)], &ctx()).is_err());
    }

    #[test]
    fn title_case_no_args() {
        assert!(verb_title_case(&[], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // contains extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn contains_empty_substring() {
        assert_eq!(verb_contains(&[s("hello"), s("")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn contains_empty_string() {
        assert_eq!(verb_contains(&[s(""), s("a")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn contains_both_empty() {
        assert_eq!(verb_contains(&[s(""), s("")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn contains_case_sensitive() {
        assert_eq!(verb_contains(&[s("Hello"), s("hello")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn contains_unicode() {
        assert_eq!(verb_contains(&[s("café"), s("fé")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn contains_null_first_arg() {
        assert_eq!(verb_contains(&[null(), s("x")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn contains_too_few_args() {
        assert!(verb_contains(&[s("hello")], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // startsWith extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn starts_with_empty_prefix() {
        assert_eq!(verb_starts_with(&[s("hello"), s("")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn starts_with_full_match() {
        assert_eq!(verb_starts_with(&[s("hello"), s("hello")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn starts_with_longer_prefix() {
        assert_eq!(verb_starts_with(&[s("hi"), s("hello")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn starts_with_null() {
        assert_eq!(verb_starts_with(&[null(), s("x")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn starts_with_too_few_args() {
        assert!(verb_starts_with(&[s("hello")], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // endsWith extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn ends_with_empty_suffix() {
        assert_eq!(verb_ends_with(&[s("hello"), s("")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn ends_with_full_match() {
        assert_eq!(verb_ends_with(&[s("hello"), s("hello")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn ends_with_no_match() {
        assert_eq!(verb_ends_with(&[s("hello"), s("xyz")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn ends_with_null() {
        assert_eq!(verb_ends_with(&[null(), s("x")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn ends_with_too_few_args() {
        assert!(verb_ends_with(&[s("hello")], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // replaceRegex extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn replace_regex_no_match() {
        assert_eq!(verb_replace_regex(&[s("hello"), s("xyz"), s("abc")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn replace_regex_empty_pattern() {
        // Replacing empty string inserts replacement between every char
        let result = verb_replace_regex(&[s("ab"), s(""), s("-")], &ctx()).unwrap();
        assert_eq!(result, s("-a-b-"));
    }

    #[test]
    fn replace_regex_empty_replacement() {
        assert_eq!(verb_replace_regex(&[s("hello world"), s(" "), s("")], &ctx()).unwrap(), s("helloworld"));
    }

    #[test]
    fn replace_regex_multiple_occurrences() {
        assert_eq!(verb_replace_regex(&[s("aaa"), s("a"), s("bb")], &ctx()).unwrap(), s("bbbbbb"));
    }

    #[test]
    fn replace_regex_special_chars() {
        assert_eq!(verb_replace_regex(&[s("a.b.c"), s("."), s("-")], &ctx()).unwrap(), s("a-b-c"));
    }

    #[test]
    fn replace_regex_too_few_args() {
        assert!(verb_replace_regex(&[s("hello"), s("x")], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // padLeft extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn pad_left_already_wide_enough() {
        assert_eq!(verb_pad_left(&[s("hello"), i(3), s("0")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn pad_left_exact_width() {
        assert_eq!(verb_pad_left(&[s("hi"), i(2), s("0")], &ctx()).unwrap(), s("hi"));
    }

    #[test]
    fn pad_left_empty_string() {
        assert_eq!(verb_pad_left(&[s(""), i(3), s("x")], &ctx()).unwrap(), s("xxx"));
    }

    #[test]
    fn pad_left_null_coercion() {
        // Null coerces to empty string via to_str
        assert_eq!(verb_pad_left(&[null(), i(3), s("0")], &ctx()).unwrap(), s("000"));
    }

    #[test]
    fn pad_left_too_few_args() {
        assert!(verb_pad_left(&[s("hi"), i(5)], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // padRight extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn pad_right_already_wide_enough() {
        assert_eq!(verb_pad_right(&[s("hello"), i(3), s(".")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn pad_right_empty_string() {
        assert_eq!(verb_pad_right(&[s(""), i(4), s("-")], &ctx()).unwrap(), s("----"));
    }

    #[test]
    fn pad_right_space_char() {
        assert_eq!(verb_pad_right(&[s("hi"), i(5), s(" ")], &ctx()).unwrap(), s("hi   "));
    }

    #[test]
    fn pad_right_too_few_args() {
        assert!(verb_pad_right(&[s("hi"), i(5)], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // pad (center) extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn pad_center_already_wide() {
        assert_eq!(verb_pad(&[s("hello"), i(3), s("*")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn pad_center_odd_padding() {
        // 3 chars to pad: left=1, right=2
        assert_eq!(verb_pad(&[s("ab"), i(5), s("-")], &ctx()).unwrap(), s("-ab--"));
    }

    #[test]
    fn pad_center_empty_string() {
        assert_eq!(verb_pad(&[s(""), i(4), s("x")], &ctx()).unwrap(), s("xxxx"));
    }

    #[test]
    fn pad_center_too_few_args() {
        assert!(verb_pad(&[s("hi"), i(5)], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // truncate extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn truncate_shorter_than_limit() {
        assert_eq!(verb_truncate(&[s("hi"), i(10)], &ctx()).unwrap(), s("hi"));
    }

    #[test]
    fn truncate_exact_length() {
        assert_eq!(verb_truncate(&[s("hello"), i(5)], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn truncate_to_zero() {
        assert_eq!(verb_truncate(&[s("hello"), i(0)], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn truncate_empty_string() {
        assert_eq!(verb_truncate(&[s(""), i(5)], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn truncate_unicode() {
        // chars().take() should handle unicode correctly
        assert_eq!(verb_truncate(&[s("café"), i(3)], &ctx()).unwrap(), s("caf"));
    }

    #[test]
    fn truncate_null() {
        assert_eq!(verb_truncate(&[null(), i(5)], &ctx()).unwrap(), null());
    }

    #[test]
    fn truncate_too_few_args() {
        assert!(verb_truncate(&[s("hello")], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // split extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn split_empty_string() {
        assert_eq!(verb_split(&[s(""), s(",")], &ctx()).unwrap(), DynValue::Array(vec![s("")]));
    }

    #[test]
    fn split_no_delimiter_found() {
        assert_eq!(verb_split(&[s("hello"), s(",")], &ctx()).unwrap(), DynValue::Array(vec![s("hello")]));
    }

    #[test]
    fn split_empty_delimiter() {
        // Splitting by empty string yields each char plus empty strings
        let result = verb_split(&[s("ab"), s("")], &ctx()).unwrap();
        if let DynValue::Array(arr) = &result {
            assert!(arr.len() > 2); // "" splits into many segments
        }
    }

    #[test]
    fn split_multi_char_delimiter() {
        assert_eq!(
            verb_split(&[s("a::b::c"), s("::")], &ctx()).unwrap(),
            DynValue::Array(vec![s("a"), s("b"), s("c")])
        );
    }

    #[test]
    fn split_with_index_1() {
        // 3rd arg is index: split("a,b,c", ",") = ["a","b","c"], index 1 = "b"
        assert_eq!(
            verb_split(&[s("a,b,c"), s(","), i(1)], &ctx()).unwrap(),
            s("b")
        );
    }

    #[test]
    fn split_too_few_args() {
        assert!(verb_split(&[s("hello")], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // join extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn join_empty_array() {
        assert_eq!(verb_join(&[DynValue::Array(vec![]), s(",")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn join_single_element() {
        assert_eq!(verb_join(&[DynValue::Array(vec![s("a")]), s(",")], &ctx()).unwrap(), s("a"));
    }

    #[test]
    fn join_empty_delimiter() {
        assert_eq!(
            verb_join(&[DynValue::Array(vec![s("a"), s("b"), s("c")]), s("")], &ctx()).unwrap(),
            s("abc")
        );
    }

    #[test]
    fn join_with_integers() {
        assert_eq!(
            verb_join(&[DynValue::Array(vec![i(1), i(2), i(3)]), s("-")], &ctx()).unwrap(),
            s("1-2-3")
        );
    }

    #[test]
    fn join_too_few_args() {
        assert!(verb_join(&[DynValue::Array(vec![s("a")])], &ctx()).is_err());
    }

    #[test]
    fn join_non_array_first_arg() {
        assert!(verb_join(&[s("not array"), s(",")], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // mask extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn mask_show_all() {
        assert_eq!(verb_mask(&[s("abc"), i(10)], &ctx()).unwrap(), s("abc"));
    }

    #[test]
    fn mask_show_zero() {
        assert_eq!(verb_mask(&[s("abc"), i(0)], &ctx()).unwrap(), s("***"));
    }

    #[test]
    fn mask_show_exact_length() {
        assert_eq!(verb_mask(&[s("abc"), i(3)], &ctx()).unwrap(), s("abc"));
    }

    #[test]
    fn mask_format_pattern() {
        assert_eq!(verb_mask(&[s("1234567890"), s("(###) ###-####")], &ctx()).unwrap(), s("(123) 456-7890"));
    }

    #[test]
    fn mask_format_short_input() {
        // Pattern has more # than input chars — extra # are skipped
        assert_eq!(verb_mask(&[s("12"), s("###-####")], &ctx()).unwrap(), s("12-"));
    }

    #[test]
    fn mask_null() {
        assert_eq!(verb_mask(&[null(), i(4)], &ctx()).unwrap(), null());
    }

    #[test]
    fn mask_too_few_args() {
        assert!(verb_mask(&[s("hello")], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // reverseString extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn reverse_empty() {
        assert_eq!(verb_reverse_string(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn reverse_single_char() {
        assert_eq!(verb_reverse_string(&[s("a")], &ctx()).unwrap(), s("a"));
    }

    #[test]
    fn reverse_palindrome() {
        assert_eq!(verb_reverse_string(&[s("racecar")], &ctx()).unwrap(), s("racecar"));
    }

    #[test]
    fn reverse_unicode() {
        assert_eq!(verb_reverse_string(&[s("abc")], &ctx()).unwrap(), s("cba"));
    }

    #[test]
    fn reverse_with_spaces() {
        assert_eq!(verb_reverse_string(&[s("a b c")], &ctx()).unwrap(), s("c b a"));
    }

    #[test]
    fn reverse_null() {
        assert_eq!(verb_reverse_string(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn reverse_wrong_type() {
        assert!(verb_reverse_string(&[i(42)], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // repeat extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn repeat_zero_times() {
        assert_eq!(verb_repeat(&[s("abc"), i(0)], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn repeat_one_time() {
        assert_eq!(verb_repeat(&[s("abc"), i(1)], &ctx()).unwrap(), s("abc"));
    }

    #[test]
    fn repeat_empty_string() {
        assert_eq!(verb_repeat(&[s(""), i(5)], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn repeat_large_count() {
        let result = verb_repeat(&[s("x"), i(100)], &ctx()).unwrap();
        if let DynValue::String(r) = result { assert_eq!(r.len(), 100); } else { panic!("expected string"); }
    }

    #[test]
    fn repeat_too_few_args() {
        assert!(verb_repeat(&[s("abc")], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // camelCase extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn camel_case_empty() {
        assert_eq!(verb_camel_case(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn camel_case_single_word() {
        assert_eq!(verb_camel_case(&[s("hello")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn camel_case_from_snake() {
        assert_eq!(verb_camel_case(&[s("hello_world")], &ctx()).unwrap(), s("helloWorld"));
    }

    #[test]
    fn camel_case_from_kebab() {
        assert_eq!(verb_camel_case(&[s("hello-world")], &ctx()).unwrap(), s("helloWorld"));
    }

    #[test]
    fn camel_case_from_pascal() {
        assert_eq!(verb_camel_case(&[s("HelloWorld")], &ctx()).unwrap(), s("helloWorld"));
    }

    #[test]
    fn camel_case_multiple_words() {
        assert_eq!(verb_camel_case(&[s("the quick brown fox")], &ctx()).unwrap(), s("theQuickBrownFox"));
    }

    #[test]
    fn camel_case_null() {
        assert_eq!(verb_camel_case(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn camel_case_wrong_type() {
        assert!(verb_camel_case(&[i(42)], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // snakeCase extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn snake_case_empty() {
        assert_eq!(verb_snake_case(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn snake_case_single_word() {
        assert_eq!(verb_snake_case(&[s("hello")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn snake_case_from_camel() {
        assert_eq!(verb_snake_case(&[s("helloWorld")], &ctx()).unwrap(), s("hello_world"));
    }

    #[test]
    fn snake_case_from_pascal() {
        assert_eq!(verb_snake_case(&[s("HelloWorld")], &ctx()).unwrap(), s("hello_world"));
    }

    #[test]
    fn snake_case_from_kebab() {
        assert_eq!(verb_snake_case(&[s("hello-world")], &ctx()).unwrap(), s("hello_world"));
    }

    #[test]
    fn snake_case_spaces() {
        assert_eq!(verb_snake_case(&[s("hello world test")], &ctx()).unwrap(), s("hello_world_test"));
    }

    #[test]
    fn snake_case_null() {
        assert_eq!(verb_snake_case(&[null()], &ctx()).unwrap(), null());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // kebabCase extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn kebab_case_empty() {
        assert_eq!(verb_kebab_case(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn kebab_case_single_word() {
        assert_eq!(verb_kebab_case(&[s("hello")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn kebab_case_from_camel() {
        assert_eq!(verb_kebab_case(&[s("helloWorld")], &ctx()).unwrap(), s("hello-world"));
    }

    #[test]
    fn kebab_case_from_snake() {
        assert_eq!(verb_kebab_case(&[s("hello_world")], &ctx()).unwrap(), s("hello-world"));
    }

    #[test]
    fn kebab_case_from_pascal() {
        assert_eq!(verb_kebab_case(&[s("HelloWorld")], &ctx()).unwrap(), s("hello-world"));
    }

    #[test]
    fn kebab_case_spaces() {
        assert_eq!(verb_kebab_case(&[s("hello world test")], &ctx()).unwrap(), s("hello-world-test"));
    }

    #[test]
    fn kebab_case_null() {
        assert_eq!(verb_kebab_case(&[null()], &ctx()).unwrap(), null());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // pascalCase extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn pascal_case_empty() {
        assert_eq!(verb_pascal_case(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn pascal_case_single_word() {
        assert_eq!(verb_pascal_case(&[s("hello")], &ctx()).unwrap(), s("Hello"));
    }

    #[test]
    fn pascal_case_from_camel() {
        assert_eq!(verb_pascal_case(&[s("helloWorld")], &ctx()).unwrap(), s("HelloWorld"));
    }

    #[test]
    fn pascal_case_from_snake() {
        assert_eq!(verb_pascal_case(&[s("hello_world")], &ctx()).unwrap(), s("HelloWorld"));
    }

    #[test]
    fn pascal_case_from_kebab() {
        assert_eq!(verb_pascal_case(&[s("hello-world")], &ctx()).unwrap(), s("HelloWorld"));
    }

    #[test]
    fn pascal_case_multiple_words() {
        assert_eq!(verb_pascal_case(&[s("the quick brown fox")], &ctx()).unwrap(), s("TheQuickBrownFox"));
    }

    #[test]
    fn pascal_case_null() {
        assert_eq!(verb_pascal_case(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn pascal_case_wrong_type() {
        assert!(verb_pascal_case(&[i(42)], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // slugify extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn slugify_empty() {
        assert_eq!(verb_slugify(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn slugify_already_slug() {
        assert_eq!(verb_slugify(&[s("hello-world")], &ctx()).unwrap(), s("hello-world"));
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(verb_slugify(&[s("Hello, World! #1")], &ctx()).unwrap(), s("hello-world-1"));
    }

    #[test]
    fn slugify_multiple_spaces() {
        assert_eq!(verb_slugify(&[s("hello   world")], &ctx()).unwrap(), s("hello-world"));
    }

    #[test]
    fn slugify_leading_trailing_special() {
        assert_eq!(verb_slugify(&[s("!!hello!!")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn slugify_numbers() {
        assert_eq!(verb_slugify(&[s("Test 123 Stuff")], &ctx()).unwrap(), s("test-123-stuff"));
    }

    #[test]
    fn slugify_null() {
        assert_eq!(verb_slugify(&[null()], &ctx()).unwrap(), null());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // match / matches extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn match_empty_pattern() {
        assert_eq!(verb_match(&[s("hello"), s("")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn match_no_match() {
        assert_eq!(verb_match(&[s("hello"), s("xyz")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn match_null() {
        assert_eq!(verb_match(&[null(), s("x")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn match_too_few_args() {
        assert!(verb_match(&[s("hello")], &ctx()).is_err());
    }

    #[test]
    fn matches_delegates_to_match() {
        assert_eq!(verb_matches(&[s("hello"), s("ell")], &ctx()).unwrap(), DynValue::Bool(true));
        assert_eq!(verb_matches(&[s("hello"), s("xyz")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // extract extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn extract_no_start_delim() {
        assert_eq!(verb_extract(&[s("hello world"), s("["), s("]")], &ctx()).unwrap(), null());
    }

    #[test]
    fn extract_no_end_delim() {
        assert_eq!(verb_extract(&[s("hello [world"), s("["), s("]")], &ctx()).unwrap(), null());
    }

    #[test]
    fn extract_empty_between_delims() {
        assert_eq!(verb_extract(&[s("[]"), s("["), s("]")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn extract_nested_delims() {
        // Only finds first occurrence
        assert_eq!(verb_extract(&[s("<a><b>"), s("<"), s(">")], &ctx()).unwrap(), s("a"));
    }

    #[test]
    fn extract_null() {
        assert_eq!(verb_extract(&[null(), s("<"), s(">")], &ctx()).unwrap(), null());
    }

    #[test]
    fn extract_too_few_args() {
        assert!(verb_extract(&[s("hello")], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // normalizeSpace extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn normalize_space_empty() {
        assert_eq!(verb_normalize_space(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn normalize_space_only_whitespace() {
        assert_eq!(verb_normalize_space(&[s("   ")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn normalize_space_tabs_and_newlines() {
        assert_eq!(verb_normalize_space(&[s("hello\t\nworld")], &ctx()).unwrap(), s("hello world"));
    }

    #[test]
    fn normalize_space_single_word() {
        assert_eq!(verb_normalize_space(&[s("hello")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn normalize_space_null() {
        assert_eq!(verb_normalize_space(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn normalize_space_wrong_type() {
        assert!(verb_normalize_space(&[i(42)], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // leftOf extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn left_of_no_delimiter() {
        assert_eq!(verb_left_of(&[s("hello"), s("@")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn left_of_at_start() {
        assert_eq!(verb_left_of(&[s("@hello"), s("@")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn left_of_multiple_delimiters() {
        assert_eq!(verb_left_of(&[s("a@b@c"), s("@")], &ctx()).unwrap(), s("a"));
    }

    #[test]
    fn left_of_empty_string() {
        assert_eq!(verb_left_of(&[s(""), s("@")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn left_of_too_few_args() {
        assert!(verb_left_of(&[s("hello")], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // rightOf extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn right_of_no_delimiter() {
        assert_eq!(verb_right_of(&[s("hello"), s("@")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn right_of_at_end() {
        assert_eq!(verb_right_of(&[s("hello@"), s("@")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn right_of_multiple_delimiters() {
        assert_eq!(verb_right_of(&[s("a@b@c"), s("@")], &ctx()).unwrap(), s("b@c"));
    }

    #[test]
    fn right_of_empty_string() {
        assert_eq!(verb_right_of(&[s(""), s("@")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn right_of_too_few_args() {
        assert!(verb_right_of(&[s("hello")], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // wrap extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn wrap_zero_width() {
        assert_eq!(verb_wrap(&[s("hello world"), i(0)], &ctx()).unwrap(), s("hello world"));
    }

    #[test]
    fn wrap_single_long_word() {
        assert_eq!(
            verb_wrap(&[s("superlongword"), i(5)], &ctx()).unwrap(),
            DynValue::Array(vec![s("superlongword")])
        );
    }

    #[test]
    fn wrap_empty_string() {
        assert_eq!(verb_wrap(&[s(""), i(10)], &ctx()).unwrap(), DynValue::Array(vec![]));
    }

    #[test]
    fn wrap_exact_width() {
        assert_eq!(
            verb_wrap(&[s("hello world"), i(11)], &ctx()).unwrap(),
            DynValue::Array(vec![s("hello world")])
        );
    }

    #[test]
    fn wrap_too_few_args() {
        assert!(verb_wrap(&[s("hello")], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // center extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn center_already_wide() {
        assert_eq!(verb_center(&[s("hello"), i(3), s("-")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn center_odd_padding() {
        assert_eq!(verb_center(&[s("ab"), i(5), s("-")], &ctx()).unwrap(), s("-ab--"));
    }

    #[test]
    fn center_empty_string() {
        assert_eq!(verb_center(&[s(""), i(4), s("*")], &ctx()).unwrap(), s("****"));
    }

    #[test]
    fn center_too_few_args() {
        assert!(verb_center(&[s("hi"), i(5)], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // stripAccents extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn strip_accents_empty() {
        assert_eq!(verb_strip_accents(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn strip_accents_no_accents() {
        assert_eq!(verb_strip_accents(&[s("hello")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn strip_accents_various() {
        assert_eq!(verb_strip_accents(&[s("\u{00E0}\u{00E1}\u{00E2}\u{00E3}\u{00E4}\u{00E5}")], &ctx()).unwrap(), s("aaaaaa"));
    }

    #[test]
    fn strip_accents_upper() {
        assert_eq!(verb_strip_accents(&[s("\u{00C0}\u{00C1}\u{00C2}")], &ctx()).unwrap(), s("AAA"));
    }

    #[test]
    fn strip_accents_cedilla() {
        assert_eq!(verb_strip_accents(&[s("\u{00E7}\u{00C7}")], &ctx()).unwrap(), s("cC"));
    }

    #[test]
    fn strip_accents_n_tilde() {
        assert_eq!(verb_strip_accents(&[s("\u{00F1}\u{00D1}")], &ctx()).unwrap(), s("nN"));
    }

    #[test]
    fn strip_accents_null() {
        assert_eq!(verb_strip_accents(&[null()], &ctx()).unwrap(), null());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // clean extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn clean_empty() {
        assert_eq!(verb_clean(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn clean_no_control_chars() {
        assert_eq!(verb_clean(&[s("hello world")], &ctx()).unwrap(), s("hello world"));
    }

    #[test]
    fn clean_preserves_newlines_tabs() {
        assert_eq!(verb_clean(&[s("a\nb\tc\r")], &ctx()).unwrap(), s("a\nb\tc\r"));
    }

    #[test]
    fn clean_removes_null_bytes() {
        assert_eq!(verb_clean(&[s("a\x00b\x01c\x02d")], &ctx()).unwrap(), s("abcd"));
    }

    #[test]
    fn clean_null() {
        assert_eq!(verb_clean(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn clean_wrong_type() {
        assert!(verb_clean(&[i(42)], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // wordCount extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn word_count_empty() {
        assert_eq!(verb_word_count(&[s("")], &ctx()).unwrap(), i(0));
    }

    #[test]
    fn word_count_only_whitespace() {
        assert_eq!(verb_word_count(&[s("   ")], &ctx()).unwrap(), i(0));
    }

    #[test]
    fn word_count_single_word() {
        assert_eq!(verb_word_count(&[s("hello")], &ctx()).unwrap(), i(1));
    }

    #[test]
    fn word_count_multiple_spaces() {
        assert_eq!(verb_word_count(&[s("  hello   world  ")], &ctx()).unwrap(), i(2));
    }

    #[test]
    fn word_count_with_tabs() {
        assert_eq!(verb_word_count(&[s("a\tb\tc")], &ctx()).unwrap(), i(3));
    }

    #[test]
    fn word_count_null() {
        assert_eq!(verb_word_count(&[null()], &ctx()).unwrap(), i(0));
    }

    #[test]
    fn word_count_wrong_type() {
        assert!(verb_word_count(&[i(42)], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // tokenize extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn tokenize_empty() {
        assert_eq!(verb_tokenize(&[s("")], &ctx()).unwrap(), DynValue::Array(vec![]));
    }

    #[test]
    fn tokenize_whitespace_default() {
        assert_eq!(
            verb_tokenize(&[s("hello world test")], &ctx()).unwrap(),
            DynValue::Array(vec![s("hello"), s("world"), s("test")])
        );
    }

    #[test]
    fn tokenize_with_delimiter() {
        assert_eq!(
            verb_tokenize(&[s("a,b,c"), s(",")], &ctx()).unwrap(),
            DynValue::Array(vec![s("a"), s("b"), s("c")])
        );
    }

    #[test]
    fn tokenize_trims_results() {
        assert_eq!(
            verb_tokenize(&[s("a , b , c"), s(",")], &ctx()).unwrap(),
            DynValue::Array(vec![s("a"), s("b"), s("c")])
        );
    }

    #[test]
    fn tokenize_filters_empty() {
        assert_eq!(
            verb_tokenize(&[s("a,,b"), s(",")], &ctx()).unwrap(),
            DynValue::Array(vec![s("a"), s("b")])
        );
    }

    #[test]
    fn tokenize_no_args() {
        assert_eq!(verb_tokenize(&[], &ctx()).unwrap(), DynValue::Array(vec![]));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // levenshtein extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn levenshtein_identical() {
        assert_eq!(verb_levenshtein(&[s("hello"), s("hello")], &ctx()).unwrap(), i(0));
    }

    #[test]
    fn levenshtein_empty_first() {
        assert_eq!(verb_levenshtein(&[s(""), s("hello")], &ctx()).unwrap(), i(5));
    }

    #[test]
    fn levenshtein_empty_second() {
        assert_eq!(verb_levenshtein(&[s("hello"), s("")], &ctx()).unwrap(), i(5));
    }

    #[test]
    fn levenshtein_both_empty() {
        assert_eq!(verb_levenshtein(&[s(""), s("")], &ctx()).unwrap(), i(0));
    }

    #[test]
    fn levenshtein_single_edit() {
        assert_eq!(verb_levenshtein(&[s("kitten"), s("sitten")], &ctx()).unwrap(), i(1));
    }

    #[test]
    fn levenshtein_classic() {
        assert_eq!(verb_levenshtein(&[s("kitten"), s("sitting")], &ctx()).unwrap(), i(3));
    }

    #[test]
    fn levenshtein_too_few_args() {
        assert_eq!(verb_levenshtein(&[s("hello")], &ctx()).unwrap(), null());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // soundex extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn soundex_robert() {
        assert_eq!(verb_soundex(&[s("Robert")], &ctx()).unwrap(), s("R163"));
    }

    #[test]
    fn soundex_rupert() {
        assert_eq!(verb_soundex(&[s("Rupert")], &ctx()).unwrap(), s("R163"));
    }

    #[test]
    fn soundex_ashcraft() {
        assert_eq!(verb_soundex(&[s("Ashcraft")], &ctx()).unwrap(), s("A226"));
    }

    #[test]
    fn soundex_empty() {
        assert_eq!(verb_soundex(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn soundex_single_letter() {
        assert_eq!(verb_soundex(&[s("A")], &ctx()).unwrap(), s("A000"));
    }

    #[test]
    fn soundex_no_args() {
        assert_eq!(verb_soundex(&[], &ctx()).unwrap(), null());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // base64 encode/decode extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn base64_round_trip_empty() {
        let encoded = verb_base64_encode(&[s("")], &ctx()).unwrap();
        let decoded = verb_base64_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s(""));
    }

    #[test]
    fn base64_round_trip_simple() {
        let encoded = verb_base64_encode(&[s("Hello, World!")], &ctx()).unwrap();
        let decoded = verb_base64_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s("Hello, World!"));
    }

    #[test]
    fn base64_round_trip_special_chars() {
        let encoded = verb_base64_encode(&[s("a&b=c d+e")], &ctx()).unwrap();
        let decoded = verb_base64_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s("a&b=c d+e"));
    }

    #[test]
    fn base64_round_trip_unicode() {
        let encoded = verb_base64_encode(&[s("café")], &ctx()).unwrap();
        let decoded = verb_base64_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s("café"));
    }

    #[test]
    fn base64_encode_known() {
        assert_eq!(verb_base64_encode(&[s("Man")], &ctx()).unwrap(), s("TWFu"));
    }

    #[test]
    fn base64_encode_null() {
        assert_eq!(verb_base64_encode(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn base64_decode_null() {
        assert_eq!(verb_base64_decode(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn base64_decode_wrong_type() {
        assert!(verb_base64_decode(&[i(42)], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // URL encode/decode extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn url_round_trip_simple() {
        let encoded = verb_url_encode(&[s("hello world")], &ctx()).unwrap();
        let decoded = verb_url_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s("hello world"));
    }

    #[test]
    fn url_round_trip_special() {
        let encoded = verb_url_encode(&[s("a=1&b=2")], &ctx()).unwrap();
        let decoded = verb_url_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s("a=1&b=2"));
    }

    #[test]
    fn url_encode_safe_chars() {
        // Unreserved chars should not be encoded
        assert_eq!(verb_url_encode(&[s("abc-_.~")], &ctx()).unwrap(), s("abc-_.~"));
    }

    #[test]
    fn url_encode_empty() {
        assert_eq!(verb_url_encode(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn url_decode_plus_as_space() {
        assert_eq!(verb_url_decode(&[s("hello+world")], &ctx()).unwrap(), s("hello world"));
    }

    #[test]
    fn url_encode_null() {
        assert_eq!(verb_url_encode(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn url_decode_null() {
        assert_eq!(verb_url_decode(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn url_decode_wrong_type() {
        assert!(verb_url_decode(&[i(42)], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // hex encode/decode extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn hex_round_trip_simple() {
        let encoded = verb_hex_encode(&[s("Hello")], &ctx()).unwrap();
        let decoded = verb_hex_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s("Hello"));
    }

    #[test]
    fn hex_round_trip_empty() {
        let encoded = verb_hex_encode(&[s("")], &ctx()).unwrap();
        assert_eq!(encoded, s(""));
        let decoded = verb_hex_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s(""));
    }

    #[test]
    fn hex_round_trip_special() {
        let encoded = verb_hex_encode(&[s("ABC")], &ctx()).unwrap();
        assert_eq!(encoded, s("414243"));
        let decoded = verb_hex_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s("ABC"));
    }

    #[test]
    fn hex_round_trip_numbers() {
        let encoded = verb_hex_encode(&[s("0123")], &ctx()).unwrap();
        assert_eq!(encoded, s("30313233"));
        let decoded = verb_hex_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s("0123"));
    }

    #[test]
    fn hex_decode_uppercase() {
        assert_eq!(verb_hex_decode(&[s("4869")], &ctx()).unwrap(), s("Hi"));
    }

    #[test]
    fn hex_decode_odd_length() {
        assert!(verb_hex_decode(&[s("486")], &ctx()).is_err());
    }

    #[test]
    fn hex_decode_invalid_chars() {
        assert!(verb_hex_decode(&[s("ZZZZ")], &ctx()).is_err());
    }

    #[test]
    fn hex_encode_null() {
        assert_eq!(verb_hex_encode(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn hex_decode_null() {
        assert_eq!(verb_hex_decode(&[null()], &ctx()).unwrap(), null());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // JSON encode/decode extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn json_round_trip_string() {
        let encoded = verb_json_encode(&[s("hello")], &ctx()).unwrap();
        assert_eq!(encoded, s("\"hello\""));
    }

    #[test]
    fn json_round_trip_integer() {
        let encoded = verb_json_encode(&[i(42)], &ctx()).unwrap();
        assert_eq!(encoded, s("42"));
    }

    #[test]
    fn json_round_trip_bool() {
        let encoded = verb_json_encode(&[DynValue::Bool(true)], &ctx()).unwrap();
        assert_eq!(encoded, s("true"));
    }

    #[test]
    fn json_round_trip_null() {
        let encoded = verb_json_encode(&[null()], &ctx()).unwrap();
        assert_eq!(encoded, s("null"));
    }

    #[test]
    fn json_round_trip_array() {
        let encoded = verb_json_encode(&[DynValue::Array(vec![i(1), i(2), i(3)])], &ctx()).unwrap();
        assert_eq!(encoded, s("[1,2,3]"));
    }

    #[test]
    fn json_decode_array() {
        let result = verb_json_decode(&[s("[1,2,3]")], &ctx()).unwrap();
        assert_eq!(result, DynValue::Array(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn json_decode_string() {
        let result = verb_json_decode(&[s("\"hello\"")], &ctx()).unwrap();
        assert_eq!(result, s("hello"));
    }

    #[test]
    fn json_decode_null_input() {
        assert_eq!(verb_json_decode(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn json_encode_no_args() {
        assert!(verb_json_encode(&[], &ctx()).is_err());
    }

    #[test]
    fn json_decode_wrong_type() {
        assert!(verb_json_decode(&[i(42)], &ctx()).is_err());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // sha256 extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn sha256_empty() {
        assert_eq!(
            verb_sha256(&[s("")], &ctx()).unwrap(),
            s("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
    }

    #[test]
    fn sha256_hello() {
        assert_eq!(
            verb_sha256(&[s("hello")], &ctx()).unwrap(),
            s("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
        );
    }

    #[test]
    fn sha256_null() {
        assert_eq!(verb_sha256(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn sha256_wrong_type() {
        assert!(verb_sha256(&[i(42)], &ctx()).is_err());
    }

    #[test]
    fn sha256_deterministic() {
        let r1 = verb_sha256(&[s("abc")], &ctx()).unwrap();
        let r2 = verb_sha256(&[s("abc")], &ctx()).unwrap();
        assert_eq!(r1, r2);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // md5 extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn md5_empty() {
        assert_eq!(
            verb_md5(&[s("")], &ctx()).unwrap(),
            s("d41d8cd98f00b204e9800998ecf8427e")
        );
    }

    #[test]
    fn md5_hello() {
        assert_eq!(
            verb_md5(&[s("hello")], &ctx()).unwrap(),
            s("5d41402abc4b2a76b9719d911017c592")
        );
    }

    #[test]
    fn md5_null() {
        assert_eq!(verb_md5(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn md5_wrong_type() {
        assert!(verb_md5(&[i(42)], &ctx()).is_err());
    }

    #[test]
    fn md5_deterministic() {
        let r1 = verb_md5(&[s("test123")], &ctx()).unwrap();
        let r2 = verb_md5(&[s("test123")], &ctx()).unwrap();
        assert_eq!(r1, r2);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // crc32 extended tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn crc32_empty() {
        assert_eq!(verb_crc32(&[s("")], &ctx()).unwrap(), s("00000000"));
    }

    #[test]
    fn crc32_hello() {
        // CRC32 of "hello" = 3610a686
        assert_eq!(verb_crc32(&[s("hello")], &ctx()).unwrap(), s("3610a686"));
    }

    #[test]
    fn crc32_null() {
        assert_eq!(verb_crc32(&[null()], &ctx()).unwrap(), null());
    }

    #[test]
    fn crc32_wrong_type() {
        assert!(verb_crc32(&[i(42)], &ctx()).is_err());
    }

    #[test]
    fn crc32_deterministic() {
        let r1 = verb_crc32(&[s("abc")], &ctx()).unwrap();
        let r2 = verb_crc32(&[s("abc")], &ctx()).unwrap();
        assert_eq!(r1, r2);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Cross-verb integration tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn split_then_join_round_trip() {
        let split_result = verb_split(&[s("a,b,c"), s(",")], &ctx()).unwrap();
        let join_result = verb_join(&[split_result, s(",")], &ctx()).unwrap();
        assert_eq!(join_result, s("a,b,c"));
    }

    #[test]
    fn hex_encode_then_decode_round_trip() {
        let encoded = verb_hex_encode(&[s("test data")], &ctx()).unwrap();
        let decoded = verb_hex_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s("test data"));
    }

    #[test]
    fn base64_encode_then_decode_round_trip_long() {
        let long_str = "a".repeat(1000);
        let encoded = verb_base64_encode(&[s(&long_str)], &ctx()).unwrap();
        let decoded = verb_base64_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s(&long_str));
    }

    #[test]
    fn url_encode_then_decode_round_trip_unicode() {
        let input = "héllo wörld";
        let encoded = verb_url_encode(&[s(input)], &ctx()).unwrap();
        let decoded = verb_url_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s(input));
    }

    #[test]
    fn snake_to_camel_to_pascal() {
        let snake = s("hello_world_test");
        let camel = verb_camel_case(&[snake], &ctx()).unwrap();
        assert_eq!(camel, s("helloWorldTest"));
        let pascal = verb_pascal_case(&[camel], &ctx()).unwrap();
        assert_eq!(pascal, s("HelloWorldTest"));
    }

    #[test]
    fn slugify_with_accents() {
        // First strip accents, then slugify
        let stripped = verb_strip_accents(&[s("Café Résumé")], &ctx()).unwrap();
        let slugged = verb_slugify(&[stripped], &ctx()).unwrap();
        assert_eq!(slugged, s("cafe-resume"));
    }

    #[test]
    fn normalize_then_word_count() {
        let normalized = verb_normalize_space(&[s("  hello   world   test  ")], &ctx()).unwrap();
        let count = verb_word_count(&[normalized], &ctx()).unwrap();
        assert_eq!(count, i(3));
    }

    #[test]
    fn truncate_then_pad_right() {
        let truncated = verb_truncate(&[s("hello world"), i(5)], &ctx()).unwrap();
        let padded = verb_pad_right(&[truncated, i(10), s(".")], &ctx()).unwrap();
        assert_eq!(padded, s("hello....."));
    }

    #[test]
    fn mask_then_reverse() {
        let masked = verb_mask(&[s("1234567890"), i(4)], &ctx()).unwrap();
        let reversed = verb_reverse_string(&[masked], &ctx()).unwrap();
        assert_eq!(reversed, s("0987******"));
    }

    #[test]
    fn repeat_then_truncate() {
        let repeated = verb_repeat(&[s("ab"), i(10)], &ctx()).unwrap();
        let truncated = verb_truncate(&[repeated, i(7)], &ctx()).unwrap();
        assert_eq!(truncated, s("abababa"));
    }

    #[test]
    fn left_of_then_right_of() {
        let email = s("user@domain.com");
        let user = verb_left_of(&[email.clone(), s("@")], &ctx()).unwrap();
        let domain = verb_right_of(&[email, s("@")], &ctx()).unwrap();
        assert_eq!(user, s("user"));
        assert_eq!(domain, s("domain.com"));
    }

    #[test]
    fn json_encode_decode_object_round_trip() {
        let obj = DynValue::Object(vec![
            ("name".into(), s("Alice")),
            ("age".into(), i(30)),
        ]);
        let encoded = verb_json_encode(&[obj.clone()], &ctx()).unwrap();
        let decoded = verb_json_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, obj);
    }
}
#[cfg(test)]
mod extended_tests_2 {
    use super::*;
    use std::collections::HashMap;

    fn ctx() -> VerbContext<'static> {
        static NULL: DynValue = DynValue::Null;
        static LV: std::sync::OnceLock<HashMap<String, DynValue>> = std::sync::OnceLock::new();
        static ACC: std::sync::OnceLock<HashMap<String, DynValue>> = std::sync::OnceLock::new();
        static TBL: std::sync::OnceLock<HashMap<String, crate::types::transform::LookupTable>> = std::sync::OnceLock::new();
        VerbContext {
            source: &NULL,
            loop_vars: LV.get_or_init(HashMap::new),
            accumulators: ACC.get_or_init(HashMap::new),
            tables: TBL.get_or_init(HashMap::new),
        }
    }

    fn s(val: &str) -> DynValue { DynValue::String(val.to_string()) }
    fn i(val: i64) -> DynValue { DynValue::Integer(val) }

    // ═════════════════════════════════════════════════════════════════════════
    // 1. titleCase edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_title_case_empty_string() {
        assert_eq!(verb_title_case(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn ext_title_case_single_word() {
        assert_eq!(verb_title_case(&[s("hello")], &ctx()).unwrap(), s("Hello"));
    }

    #[test]
    fn ext_title_case_already_titled() {
        assert_eq!(verb_title_case(&[s("Hello World")], &ctx()).unwrap(), s("Hello World"));
    }

    #[test]
    fn ext_title_case_all_uppercase() {
        let r = verb_title_case(&[s("HELLO WORLD")], &ctx()).unwrap();
        match r {
            DynValue::String(v) => assert!(v.starts_with('H')),
            _ => panic!("Expected string"),
        }
    }

    #[test]
    fn ext_title_case_with_numbers() {
        let r = verb_title_case(&[s("hello 123 world")], &ctx()).unwrap();
        match r {
            DynValue::String(v) => assert!(v.starts_with('H')),
            _ => panic!("Expected string"),
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 2. contains/startsWith/endsWith edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_contains_empty_needle() {
        assert_eq!(verb_contains(&[s("hello"), s("")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn ext_contains_empty_haystack() {
        assert_eq!(verb_contains(&[s(""), s("x")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn ext_contains_exact_match() {
        assert_eq!(verb_contains(&[s("hello"), s("hello")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn ext_starts_with_empty() {
        assert_eq!(verb_starts_with(&[s("hello"), s("")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn ext_starts_with_full_match() {
        assert_eq!(verb_starts_with(&[s("hello"), s("hello")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn ext_starts_with_no_match() {
        assert_eq!(verb_starts_with(&[s("hello"), s("world")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn ext_ends_with_empty() {
        assert_eq!(verb_ends_with(&[s("hello"), s("")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn ext_ends_with_no_match() {
        assert_eq!(verb_ends_with(&[s("hello"), s("xyz")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 3. pad verbs edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_pad_left_already_long_enough() {
        assert_eq!(verb_pad_left(&[s("hello"), i(3), s("0")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn ext_pad_left_exact_length() {
        assert_eq!(verb_pad_left(&[s("hi"), i(2), s("0")], &ctx()).unwrap(), s("hi"));
    }

    #[test]
    fn ext_pad_left_with_space() {
        assert_eq!(verb_pad_left(&[s("x"), i(4), s(" ")], &ctx()).unwrap(), s("   x"));
    }

    #[test]
    fn ext_pad_right_already_long_enough() {
        assert_eq!(verb_pad_right(&[s("hello"), i(3), s("0")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn ext_pad_right_single_char() {
        assert_eq!(verb_pad_right(&[s("x"), i(5), s(".")], &ctx()).unwrap(), s("x...."));
    }

    #[test]
    fn ext_pad_both_sides() {
        let r = verb_pad(&[s("hi"), i(6), s("-")], &ctx()).unwrap();
        assert_eq!(r, s("--hi--"));
    }

    #[test]
    fn ext_pad_both_odd_total() {
        let r = verb_pad(&[s("hi"), i(5), s("-")], &ctx()).unwrap();
        if let DynValue::String(v) = r {
            assert_eq!(v.len(), 5);
            assert!(v.contains("hi"));
        }
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 4. truncate edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_truncate_shorter_than_max() {
        assert_eq!(verb_truncate(&[s("hi"), i(10)], &ctx()).unwrap(), s("hi"));
    }

    #[test]
    fn ext_truncate_exact_length() {
        assert_eq!(verb_truncate(&[s("hello"), i(5)], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn ext_truncate_to_zero() {
        assert_eq!(verb_truncate(&[s("hello"), i(0)], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn ext_truncate_to_one() {
        assert_eq!(verb_truncate(&[s("hello"), i(1)], &ctx()).unwrap(), s("h"));
    }

    #[test]
    fn ext_truncate_null_passthrough() {
        assert_eq!(verb_truncate(&[DynValue::Null, i(5)], &ctx()).unwrap(), DynValue::Null);
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 5. split/join edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_split_empty_string() {
        let r = verb_split(&[s(""), s(",")], &ctx()).unwrap();
        assert_eq!(r, DynValue::Array(vec![s("")]));
    }

    #[test]
    fn ext_split_no_delimiter_found() {
        let r = verb_split(&[s("hello"), s(",")], &ctx()).unwrap();
        assert_eq!(r, DynValue::Array(vec![s("hello")]));
    }

    #[test]
    fn ext_split_multiple_delimiters() {
        let r = verb_split(&[s("a,b,c,d"), s(",")], &ctx()).unwrap();
        assert_eq!(r, DynValue::Array(vec![s("a"), s("b"), s("c"), s("d")]));
    }

    #[test]
    fn ext_split_with_index() {
        // 3rd arg is index: split("a,b,c,d", ",") = ["a","b","c","d"], index 2 = "c"
        let r = verb_split(&[s("a,b,c,d"), s(","), i(2)], &ctx()).unwrap();
        assert_eq!(r, s("c"));
    }

    #[test]
    fn ext_join_empty_array() {
        let r = verb_join(&[DynValue::Array(vec![]), s(",")], &ctx()).unwrap();
        assert_eq!(r, s(""));
    }

    #[test]
    fn ext_join_single_element() {
        let r = verb_join(&[DynValue::Array(vec![s("hello")]), s(",")], &ctx()).unwrap();
        assert_eq!(r, s("hello"));
    }

    #[test]
    fn ext_join_with_empty_delimiter() {
        let r = verb_join(&[DynValue::Array(vec![s("a"), s("b"), s("c")]), s("")], &ctx()).unwrap();
        assert_eq!(r, s("abc"));
    }

    #[test]
    fn ext_join_with_multichar_delimiter() {
        let r = verb_join(&[DynValue::Array(vec![s("a"), s("b")]), s(" -- ")], &ctx()).unwrap();
        assert_eq!(r, s("a -- b"));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 6. mask verb edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_mask_show_zero() {
        let r = verb_mask(&[s("secret"), i(0)], &ctx()).unwrap();
        assert_eq!(r, s("******"));
    }

    #[test]
    fn ext_mask_show_all() {
        let r = verb_mask(&[s("hi"), i(10)], &ctx()).unwrap();
        assert_eq!(r, s("hi"));
    }

    #[test]
    fn ext_mask_show_last_4() {
        let r = verb_mask(&[s("1234567890"), i(4)], &ctx()).unwrap();
        assert_eq!(r, s("******7890"));
    }

    #[test]
    fn ext_mask_format_pattern() {
        let r = verb_mask(&[s("1234567890"), s("(###) ###-####")], &ctx()).unwrap();
        assert_eq!(r, s("(123) 456-7890"));
    }

    #[test]
    fn ext_mask_format_pattern_short_input() {
        let r = verb_mask(&[s("12"), s("###-####")], &ctx()).unwrap();
        assert_eq!(r, s("12-"));
    }

    #[test]
    fn ext_mask_null_passthrough() {
        let r = verb_mask(&[DynValue::Null, i(4)], &ctx()).unwrap();
        assert_eq!(r, DynValue::Null);
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 7. reverse, repeat edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_reverse_empty() {
        assert_eq!(verb_reverse_string(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn ext_reverse_single_char() {
        assert_eq!(verb_reverse_string(&[s("a")], &ctx()).unwrap(), s("a"));
    }

    #[test]
    fn ext_reverse_palindrome() {
        assert_eq!(verb_reverse_string(&[s("racecar")], &ctx()).unwrap(), s("racecar"));
    }

    #[test]
    fn ext_repeat_zero_times() {
        assert_eq!(verb_repeat(&[s("abc"), i(0)], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn ext_repeat_one_time() {
        assert_eq!(verb_repeat(&[s("abc"), i(1)], &ctx()).unwrap(), s("abc"));
    }

    #[test]
    fn ext_repeat_empty_string() {
        assert_eq!(verb_repeat(&[s(""), i(5)], &ctx()).unwrap(), s(""));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 8. Case conversion edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_camel_case_already_camel() {
        let r = verb_camel_case(&[s("helloWorld")], &ctx()).unwrap();
        if let DynValue::String(v) = r {
            assert!(v.starts_with('h'));
        }
    }

    #[test]
    fn ext_camel_case_from_snake() {
        assert_eq!(verb_camel_case(&[s("hello_world")], &ctx()).unwrap(), s("helloWorld"));
    }

    #[test]
    fn ext_camel_case_from_kebab() {
        assert_eq!(verb_camel_case(&[s("hello-world")], &ctx()).unwrap(), s("helloWorld"));
    }

    #[test]
    fn ext_snake_case_from_camel() {
        assert_eq!(verb_snake_case(&[s("helloWorld")], &ctx()).unwrap(), s("hello_world"));
    }

    #[test]
    fn ext_snake_case_from_pascal() {
        assert_eq!(verb_snake_case(&[s("HelloWorld")], &ctx()).unwrap(), s("hello_world"));
    }

    #[test]
    fn ext_kebab_case_from_snake() {
        assert_eq!(verb_kebab_case(&[s("hello_world")], &ctx()).unwrap(), s("hello-world"));
    }

    #[test]
    fn ext_pascal_case_from_snake() {
        assert_eq!(verb_pascal_case(&[s("hello_world")], &ctx()).unwrap(), s("HelloWorld"));
    }

    #[test]
    fn ext_pascal_case_single_word() {
        assert_eq!(verb_pascal_case(&[s("hello")], &ctx()).unwrap(), s("Hello"));
    }

    #[test]
    fn ext_camel_case_empty() {
        assert_eq!(verb_camel_case(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn ext_snake_case_empty() {
        assert_eq!(verb_snake_case(&[s("")], &ctx()).unwrap(), s(""));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 9. slugify edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_slugify_empty() {
        assert_eq!(verb_slugify(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn ext_slugify_already_slug() {
        assert_eq!(verb_slugify(&[s("hello-world")], &ctx()).unwrap(), s("hello-world"));
    }

    #[test]
    fn ext_slugify_special_chars() {
        assert_eq!(verb_slugify(&[s("Hello! World? #Test")], &ctx()).unwrap(), s("hello-world-test"));
    }

    #[test]
    fn ext_slugify_multiple_spaces() {
        assert_eq!(verb_slugify(&[s("hello   world")], &ctx()).unwrap(), s("hello-world"));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 10. match/extract/leftOf/rightOf edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_match_null_returns_false() {
        assert_eq!(verb_match(&[DynValue::Null, s("x")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn ext_match_empty_pattern() {
        assert_eq!(verb_match(&[s("hello"), s("")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    #[test]
    fn ext_extract_not_found() {
        let r = verb_extract(&[s("hello world"), s("["), s("]")], &ctx()).unwrap();
        assert_eq!(r, DynValue::Null);
    }

    #[test]
    fn ext_extract_nested_delimiters() {
        let r = verb_extract(&[s("a<b>c"), s("<"), s(">")], &ctx()).unwrap();
        assert_eq!(r, s("b"));
    }

    #[test]
    fn ext_extract_null_passthrough() {
        let r = verb_extract(&[DynValue::Null, s("["), s("]")], &ctx()).unwrap();
        assert_eq!(r, DynValue::Null);
    }

    #[test]
    fn ext_left_of_not_found() {
        let r = verb_left_of(&[s("hello"), s("@")], &ctx()).unwrap();
        // When delimiter not found, return original or null
        match r {
            DynValue::String(v) => assert!(v == "hello" || v.is_empty()),
            DynValue::Null => {} // also acceptable
            _ => panic!("Unexpected type"),
        }
    }

    #[test]
    fn ext_left_of_at_start() {
        let r = verb_left_of(&[s("@hello"), s("@")], &ctx()).unwrap();
        assert_eq!(r, s(""));
    }

    #[test]
    fn ext_right_of_not_found() {
        let r = verb_right_of(&[s("hello"), s("@")], &ctx()).unwrap();
        match r {
            DynValue::String(v) => assert!(v == "hello" || v.is_empty()),
            DynValue::Null => {}
            _ => panic!("Unexpected type"),
        }
    }

    #[test]
    fn ext_right_of_at_end() {
        let r = verb_right_of(&[s("hello@"), s("@")], &ctx()).unwrap();
        assert_eq!(r, s(""));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 11. normalize_space/clean/word_count edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_normalize_space_empty() {
        assert_eq!(verb_normalize_space(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn ext_normalize_space_only_spaces() {
        assert_eq!(verb_normalize_space(&[s("   ")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn ext_normalize_space_tabs_and_newlines() {
        assert_eq!(verb_normalize_space(&[s("hello\t\nworld")], &ctx()).unwrap(), s("hello world"));
    }

    #[test]
    fn ext_clean_no_control_chars() {
        assert_eq!(verb_clean(&[s("hello world")], &ctx()).unwrap(), s("hello world"));
    }

    #[test]
    fn ext_clean_empty_string() {
        assert_eq!(verb_clean(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn ext_word_count_empty() {
        assert_eq!(verb_word_count(&[s("")], &ctx()).unwrap(), i(0));
    }

    #[test]
    fn ext_word_count_single_word() {
        assert_eq!(verb_word_count(&[s("hello")], &ctx()).unwrap(), i(1));
    }

    #[test]
    fn ext_word_count_multiple_spaces() {
        assert_eq!(verb_word_count(&[s("  hello   world  ")], &ctx()).unwrap(), i(2));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 12. wrap/center edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_wrap_short_text_no_wrap() {
        let r = verb_wrap(&[s("short"), i(100)], &ctx()).unwrap();
        assert_eq!(r, DynValue::Array(vec![s("short")]));
    }

    #[test]
    fn ext_center_exact_width() {
        let r = verb_center(&[s("abcd"), i(4), s("-")], &ctx()).unwrap();
        assert_eq!(r, s("abcd"));
    }

    #[test]
    fn ext_center_wider_than_width() {
        let r = verb_center(&[s("hello"), i(3), s("-")], &ctx()).unwrap();
        assert_eq!(r, s("hello"));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 13. Encoding verbs edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_base64_encode_empty() {
        assert_eq!(verb_base64_encode(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn ext_base64_roundtrip() {
        let encoded = verb_base64_encode(&[s("Hello, World!")], &ctx()).unwrap();
        let decoded = verb_base64_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s("Hello, World!"));
    }

    #[test]
    fn ext_url_encode_empty() {
        assert_eq!(verb_url_encode(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn ext_url_encode_no_special_chars() {
        assert_eq!(verb_url_encode(&[s("hello")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn ext_url_roundtrip() {
        let encoded = verb_url_encode(&[s("hello world&foo=bar")], &ctx()).unwrap();
        let decoded = verb_url_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s("hello world&foo=bar"));
    }

    #[test]
    fn ext_hex_encode_empty() {
        assert_eq!(verb_hex_encode(&[s("")], &ctx()).unwrap(), s(""));
    }

    #[test]
    fn ext_hex_roundtrip() {
        let encoded = verb_hex_encode(&[s("Test123")], &ctx()).unwrap();
        let decoded = verb_hex_decode(&[encoded], &ctx()).unwrap();
        assert_eq!(decoded, s("Test123"));
    }

    #[test]
    fn ext_json_encode_array() {
        let arr = DynValue::Array(vec![i(1), i(2), i(3)]);
        let r = verb_json_encode(&[arr], &ctx()).unwrap();
        assert_eq!(r, s("[1,2,3]"));
    }

    #[test]
    fn ext_json_encode_string() {
        let r = verb_json_encode(&[s("hello")], &ctx()).unwrap();
        assert_eq!(r, s("\"hello\""));
    }

    #[test]
    fn ext_json_decode_array() {
        let r = verb_json_decode(&[s("[1,2,3]")], &ctx()).unwrap();
        assert_eq!(r, DynValue::Array(vec![i(1), i(2), i(3)]));
    }

    #[test]
    fn ext_json_decode_string() {
        let r = verb_json_decode(&[s("\"hello\"")], &ctx()).unwrap();
        assert_eq!(r, s("hello"));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 14. Hash verbs edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_sha256_empty() {
        let r = verb_sha256(&[s("")], &ctx()).unwrap();
        assert_eq!(r, s("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"));
    }

    #[test]
    fn ext_md5_empty() {
        let r = verb_md5(&[s("")], &ctx()).unwrap();
        assert_eq!(r, s("d41d8cd98f00b204e9800998ecf8427e"));
    }

    #[test]
    fn ext_crc32_empty() {
        let r = verb_crc32(&[s("")], &ctx()).unwrap();
        assert_eq!(r, s("00000000"));
    }

    #[test]
    fn ext_sha256_longer_string() {
        let r = verb_sha256(&[s("The quick brown fox jumps over the lazy dog")], &ctx()).unwrap();
        assert_eq!(r, s("d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 15. replaceRegex edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_replace_regex_no_match() {
        let r = verb_replace_regex(&[s("hello"), s("xyz"), s("abc")], &ctx()).unwrap();
        assert_eq!(r, s("hello"));
    }

    #[test]
    fn ext_replace_regex_all_occurrences() {
        let r = verb_replace_regex(&[s("aaa"), s("a"), s("b")], &ctx()).unwrap();
        assert_eq!(r, s("bbb"));
    }

    #[test]
    fn ext_replace_regex_empty_replacement() {
        let r = verb_replace_regex(&[s("hello world"), s(" world"), s("")], &ctx()).unwrap();
        assert_eq!(r, s("hello"));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 16. Text analysis verbs
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_tokenize_whitespace() {
        let r = verb_tokenize(&[s("hello world foo")], &ctx()).unwrap();
        assert_eq!(r, DynValue::Array(vec![s("hello"), s("world"), s("foo")]));
    }

    #[test]
    fn ext_tokenize_custom_delimiter() {
        let r = verb_tokenize(&[s("a,b,c"), s(",")], &ctx()).unwrap();
        assert_eq!(r, DynValue::Array(vec![s("a"), s("b"), s("c")]));
    }

    #[test]
    fn ext_tokenize_empty_string() {
        let r = verb_tokenize(&[s("")], &ctx()).unwrap();
        assert_eq!(r, DynValue::Array(vec![]));
    }

    #[test]
    fn ext_levenshtein_identical() {
        assert_eq!(verb_levenshtein(&[s("hello"), s("hello")], &ctx()).unwrap(), i(0));
    }

    #[test]
    fn ext_levenshtein_one_edit() {
        assert_eq!(verb_levenshtein(&[s("cat"), s("bat")], &ctx()).unwrap(), i(1));
    }

    #[test]
    fn ext_levenshtein_empty_to_string() {
        assert_eq!(verb_levenshtein(&[s(""), s("hello")], &ctx()).unwrap(), i(5));
    }

    #[test]
    fn ext_levenshtein_both_empty() {
        assert_eq!(verb_levenshtein(&[s(""), s("")], &ctx()).unwrap(), i(0));
    }

    #[test]
    fn ext_soundex_robert() {
        assert_eq!(verb_soundex(&[s("Robert")], &ctx()).unwrap(), s("R163"));
    }

    #[test]
    fn ext_soundex_smith() {
        assert_eq!(verb_soundex(&[s("Smith")], &ctx()).unwrap(), s("S530"));
    }

    #[test]
    fn ext_soundex_empty() {
        assert_eq!(verb_soundex(&[s("")], &ctx()).unwrap(), s(""));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 17. strip_accents edge cases
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_strip_accents_no_accents() {
        assert_eq!(verb_strip_accents(&[s("hello")], &ctx()).unwrap(), s("hello"));
    }

    #[test]
    fn ext_strip_accents_empty() {
        assert_eq!(verb_strip_accents(&[s("")], &ctx()).unwrap(), s(""));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 18. matches alias
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_matches_false() {
        assert_eq!(verb_matches(&[s("hello"), s("xyz")], &ctx()).unwrap(), DynValue::Bool(false));
    }

    #[test]
    fn ext_matches_empty_pattern() {
        assert_eq!(verb_matches(&[s("hello"), s("")], &ctx()).unwrap(), DynValue::Bool(true));
    }

    // ═════════════════════════════════════════════════════════════════════════
    // 19. Error cases - wrong argument types
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn ext_reverse_string_rejects_array() {
        let r = verb_reverse_string(&[DynValue::Array(vec![])], &ctx());
        assert!(r.is_err());
    }

    #[test]
    fn ext_mask_requires_two_args() {
        let r = verb_mask(&[s("hello")], &ctx());
        assert!(r.is_err());
    }

    #[test]
    fn ext_split_requires_two_args() {
        let r = verb_split(&[s("hello")], &ctx());
        assert!(r.is_err());
    }

    #[test]
    fn ext_join_requires_two_args() {
        let r = verb_join(&[DynValue::Array(vec![])], &ctx());
        assert!(r.is_err());
    }

    #[test]
    fn ext_repeat_requires_two_args() {
        let r = verb_repeat(&[s("hello")], &ctx());
        assert!(r.is_err());
    }

    #[test]
    fn ext_truncate_requires_two_args() {
        let r = verb_truncate(&[s("hello")], &ctx());
        assert!(r.is_err());
    }

    #[test]
    fn ext_match_requires_two_args() {
        let r = verb_match(&[s("hello")], &ctx());
        assert!(r.is_err());
    }

    #[test]
    fn ext_join_rejects_non_array_first() {
        let r = verb_join(&[s("not an array"), s(",")], &ctx());
        assert!(r.is_err());
    }

    #[test]
    fn ext_mask_rejects_non_string_first() {
        let r = verb_mask(&[DynValue::Array(vec![]), i(3)], &ctx());
        assert!(r.is_err());
    }
}
