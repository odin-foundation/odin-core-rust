//! `ReDoS` (Regular Expression Denial of Service) protection.
//!
//! Validates regex patterns before compilation to detect potentially unsafe patterns.
//!
//! Note: Rust's `regex` crate uses a Thompson NFA engine which is inherently
//! resistant to catastrophic backtracking (unlike PCRE/JS engines). However,
//! we still enforce complexity limits to prevent excessive memory/time usage
//! from large or deeply nested patterns.

/// Maximum allowed length for a regex pattern.
const MAX_PATTERN_LENGTH: usize = 1024;

/// Maximum allowed nesting depth for groups.
const MAX_NESTING_DEPTH: usize = 10;

/// Maximum number of quantifiers in a pattern.
const MAX_QUANTIFIERS: usize = 20;

/// Result of `ReDoS` analysis.
#[derive(Debug, Clone)]
pub struct RedosAnalysis {
    /// Whether the pattern is considered safe.
    pub safe: bool,
    /// Description of the issue if unsafe.
    pub reason: Option<String>,
    /// Estimated complexity score (higher = more complex).
    pub complexity: usize,
}

/// Analyze a regex pattern for potential `ReDoS` vulnerabilities.
///
/// Returns a `RedosAnalysis` indicating whether the pattern is safe to use.
pub fn analyze_pattern(pattern: &str) -> RedosAnalysis {
    // Check length
    if pattern.len() > MAX_PATTERN_LENGTH {
        return RedosAnalysis {
            safe: false,
            reason: Some(format!(
                "Pattern exceeds maximum length ({} > {})",
                pattern.len(),
                MAX_PATTERN_LENGTH
            )),
            complexity: pattern.len(),
        };
    }

    // Check nesting depth
    let max_depth = count_max_nesting(pattern);
    if max_depth > MAX_NESTING_DEPTH {
        return RedosAnalysis {
            safe: false,
            reason: Some(format!(
                "Pattern nesting depth exceeds maximum ({max_depth} > {MAX_NESTING_DEPTH})"
            )),
            complexity: max_depth * 10,
        };
    }

    // Count quantifiers
    let quantifier_count = count_quantifiers(pattern);
    if quantifier_count > MAX_QUANTIFIERS {
        return RedosAnalysis {
            safe: false,
            reason: Some(format!(
                "Pattern has too many quantifiers ({quantifier_count} > {MAX_QUANTIFIERS})"
            )),
            complexity: quantifier_count * 5,
        };
    }

    // Detect nested quantifiers (a common ReDoS trigger in backtracking engines)
    if let Some(reason) = detect_nested_quantifiers(pattern) {
        return RedosAnalysis {
            safe: false, // Flag but don't block — Rust regex handles this safely
            reason: Some(reason),
            complexity: 50,
        };
    }

    let complexity = quantifier_count * 2 + max_depth * 3 + pattern.len() / 10;
    RedosAnalysis {
        safe: true,
        reason: None,
        complexity,
    }
}

/// Count maximum nesting depth of groups in a pattern.
fn count_max_nesting(pattern: &str) -> usize {
    let mut depth: usize = 0;
    let mut max_depth: usize = 0;
    let mut in_char_class = false;
    let mut escaped = false;

    for ch in pattern.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '[' && !in_char_class {
            in_char_class = true;
            continue;
        }
        if ch == ']' && in_char_class {
            in_char_class = false;
            continue;
        }
        if in_char_class {
            continue;
        }
        if ch == '(' {
            depth += 1;
            if depth > max_depth {
                max_depth = depth;
            }
        } else if ch == ')' {
            depth = depth.saturating_sub(1);
        }
    }

    max_depth
}

/// Count quantifiers (+, *, ?, {n,m}) in a pattern.
fn count_quantifiers(pattern: &str) -> usize {
    let mut count = 0;
    let mut escaped = false;
    let mut in_char_class = false;

    for ch in pattern.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '[' && !in_char_class {
            in_char_class = true;
            continue;
        }
        if ch == ']' && in_char_class {
            in_char_class = false;
            continue;
        }
        if in_char_class {
            continue;
        }
        if matches!(ch, '+' | '*' | '?') {
            count += 1;
        }
        if ch == '{' {
            count += 1;
        }
    }

    count
}

/// Detect nested quantifiers (e.g., `(a+)+`, `(a*)*`, `(a+b*)+`).
/// These cause catastrophic backtracking in PCRE/JS but are safe in Rust regex.
fn detect_nested_quantifiers(pattern: &str) -> Option<String> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    let mut group_has_quantifier = Vec::new();
    let mut in_char_class = false;

    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '[' && !in_char_class {
            in_char_class = true;
            i += 1;
            continue;
        }
        if chars[i] == ']' && in_char_class {
            in_char_class = false;
            i += 1;
            continue;
        }
        if in_char_class {
            i += 1;
            continue;
        }

        if chars[i] == '(' {
            group_has_quantifier.push(false);
        } else if chars[i] == ')' {
            let inner_has_quant = group_has_quantifier.pop().unwrap_or(false);
            // Check if this group is followed by a quantifier
            if inner_has_quant {
                let next = chars.get(i + 1);
                if matches!(next, Some('+' | '*' | '{')) {
                    return Some("Pattern contains nested quantifiers (e.g., (a+)+) — safe in Rust regex but flagged for cross-SDK compatibility".to_string());
                }
            }
        } else if matches!(chars[i], '+' | '*' | '?' | '{') {
            if let Some(last) = group_has_quantifier.last_mut() {
                *last = true;
            }
        }

        i += 1;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_pattern() {
        let result = analyze_pattern(r"^\d{3}-\d{2}-\d{4}$");
        assert!(result.safe);
    }

    #[test]
    fn test_pattern_too_long() {
        let long = "a".repeat(MAX_PATTERN_LENGTH + 1);
        let result = analyze_pattern(&long);
        assert!(!result.safe);
        assert!(result.reason.unwrap().contains("maximum length"));
    }

    #[test]
    fn test_excessive_nesting() {
        let nested = "(".repeat(15) + "a" + &")".repeat(15);
        let result = analyze_pattern(&nested);
        assert!(!result.safe);
        assert!(result.reason.unwrap().contains("nesting depth"));
    }

    #[test]
    fn test_too_many_quantifiers() {
        let many_quants = (0..25).map(|_| "a+").collect::<String>();
        let result = analyze_pattern(&many_quants);
        assert!(!result.safe);
        assert!(result.reason.unwrap().contains("quantifiers"));
    }

    #[test]
    fn test_nested_quantifiers_detected() {
        let result = analyze_pattern(r"(a+)+");
        assert!(!result.safe);
        assert!(result.reason.unwrap().contains("nested quantifiers"));
    }

    #[test]
    fn test_normal_quantifiers_ok() {
        let result = analyze_pattern(r"\d+\.\d+");
        assert!(result.safe);
    }

    #[test]
    fn test_escaped_chars_not_counted() {
        let result = analyze_pattern(r"\(\)\+\*");
        assert!(result.safe);
        assert_eq!(result.complexity, 0); // All escaped, no real quantifiers
    }

    #[test]
    fn test_char_class_not_counted() {
        let result = analyze_pattern(r"[+*?{}]+");
        assert!(result.safe); // Quantifiers inside [] don't count
    }
}
