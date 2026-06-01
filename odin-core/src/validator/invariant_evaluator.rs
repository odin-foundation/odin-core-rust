//! Invariant expression evaluation.
//!
//! Recursive-descent evaluator over the invariant grammar:
//!   expression     = logic_or
//!   logic_or       = logic_and , { "||" , logic_and }
//!   logic_and      = equality , { "&&" , equality }
//!   equality       = comparison , { ( "==" | "!=" | "=" ) , comparison }
//!   comparison     = additive , { ( ">" | "<" | ">=" | "<=" ) , additive }
//!   additive       = multiplicative , { ( "+" | "-" ) , multiplicative }
//!   multiplicative = unary , { ( "*" | "/" | "%" ) , unary }
//!   unary          = [ "!" ] , primary
//!   primary        = path | number | string | "(" , expression , ")"

use crate::types::values::OdinValue;

const EPSILON: f64 = 1e-9;

/// A resolved operand value.
#[derive(Clone, Debug)]
enum Operand {
    Number(f64),
    Str(String),
    Bool(bool),
    Null,
}

/// Token kinds produced by the lexer.
#[derive(Clone, Debug, PartialEq)]
enum Token {
    Op(String),
    Number(String),
    Str(String),
    Ident(String),
    LParen,
    RParen,
}

/// Outcome of evaluating an invariant expression.
pub struct InvariantResult {
    /// `Some(true/false)` when fully evaluable; `None` when an operand field is absent.
    pub value: Option<bool>,
    /// True if any referenced field is present but null.
    pub null_operand: bool,
}

/// Tokenize an invariant expression. Returns `Err` on unrecognized input.
fn tokenize(expr: &str) -> Result<Vec<Token>, ()> {
    let bytes: Vec<char> = expr.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == ' ' || c == '\t' {
            i += 1;
            continue;
        }
        if c == '(' {
            tokens.push(Token::LParen);
            i += 1;
            continue;
        }
        if c == ')' {
            tokens.push(Token::RParen);
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            let quote = c;
            let mut j = i + 1;
            let mut text = String::new();
            while j < bytes.len() && bytes[j] != quote {
                text.push(bytes[j]);
                j += 1;
            }
            if j >= bytes.len() {
                return Err(()); // unterminated string
            }
            tokens.push(Token::Str(text));
            i = j + 1;
            continue;
        }
        // Multi-char operators.
        if i + 1 < bytes.len() {
            let two: String = [c, bytes[i + 1]].iter().collect();
            if matches!(two.as_str(), "==" | "!=" | ">=" | "<=" | "&&" | "||") {
                tokens.push(Token::Op(two));
                i += 2;
                continue;
            }
        }
        if matches!(c, '+' | '-' | '*' | '/' | '%' | '>' | '<' | '=' | '!') {
            tokens.push(Token::Op(c.to_string()));
            i += 1;
            continue;
        }
        if c.is_ascii_digit() {
            let mut j = i;
            while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == '.') {
                j += 1;
            }
            tokens.push(Token::Number(bytes[i..j].iter().collect()));
            i = j;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let mut j = i;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == '_' || bytes[j] == '.')
            {
                j += 1;
            }
            tokens.push(Token::Ident(bytes[i..j].iter().collect()));
            i = j;
            continue;
        }
        return Err(());
    }
    Ok(tokens)
}

/// Parse and evaluate an invariant expression. `resolve` returns the document
/// value at a field name, or `None` if absent.
pub fn evaluate_invariant<F>(expr: &str, resolve: F) -> Result<InvariantResult, ()>
where
    F: Fn(&str) -> Option<OdinValue>,
{
    let tokens = tokenize(expr)?;
    let mut parser = Parser {
        tokens,
        pos: 0,
        resolve: &resolve,
        absent_operand: false,
        null_operand: false,
    };

    let final_val = parser.parse_expression()?;
    if parser.pos != parser.tokens.len() {
        return Err(()); // trailing tokens
    }

    let value = if parser.null_operand {
        Some(false)
    } else if parser.absent_operand {
        None
    } else {
        Some(to_bool(&final_val))
    };

    Ok(InvariantResult { value, null_operand: parser.null_operand })
}

struct Parser<'a, F: Fn(&str) -> Option<OdinValue>> {
    tokens: Vec<Token>,
    pos: usize,
    resolve: &'a F,
    absent_operand: bool,
    null_operand: bool,
}

impl<'a, F: Fn(&str) -> Option<OdinValue>> Parser<'a, F> {
    fn peek_op(&self) -> Option<&str> {
        match self.tokens.get(self.pos) {
            Some(Token::Op(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn parse_expression(&mut self) -> Result<Operand, ()> {
        self.parse_logic_or()
    }

    fn parse_logic_or(&mut self) -> Result<Operand, ()> {
        let mut left = self.parse_logic_and()?;
        while self.peek_op() == Some("||") {
            self.next();
            let right = self.parse_logic_and()?;
            left = Operand::Bool(to_bool(&left) || to_bool(&right));
        }
        Ok(left)
    }

    fn parse_logic_and(&mut self) -> Result<Operand, ()> {
        let mut left = self.parse_equality()?;
        while self.peek_op() == Some("&&") {
            self.next();
            let right = self.parse_equality()?;
            left = Operand::Bool(to_bool(&left) && to_bool(&right));
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Operand, ()> {
        let mut left = self.parse_comparison()?;
        while matches!(self.peek_op(), Some("==") | Some("!=") | Some("=")) {
            let op = match self.next() {
                Some(Token::Op(s)) => s,
                _ => unreachable!(),
            };
            let right = self.parse_comparison()?;
            let eq = loose_equals(&left, &right);
            left = Operand::Bool(if op == "!=" { !eq } else { eq });
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Operand, ()> {
        let mut left = self.parse_additive()?;
        while matches!(self.peek_op(), Some(">") | Some("<") | Some(">=") | Some("<=")) {
            let op = match self.next() {
                Some(Token::Op(s)) => s,
                _ => unreachable!(),
            };
            let right = self.parse_additive()?;
            left = Operand::Bool(compare(&left, &op, &right));
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Operand, ()> {
        let mut left = self.parse_multiplicative()?;
        while matches!(self.peek_op(), Some("+") | Some("-")) {
            let op = match self.next() {
                Some(Token::Op(s)) => s,
                _ => unreachable!(),
            };
            let right = self.parse_multiplicative()?;
            left = match (to_num(&left), to_num(&right)) {
                (Some(l), Some(r)) => Operand::Number(if op == "+" { l + r } else { l - r }),
                _ => Operand::Number(f64::NAN),
            };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Operand, ()> {
        let mut left = self.parse_unary()?;
        while matches!(self.peek_op(), Some("*") | Some("/") | Some("%")) {
            let op = match self.next() {
                Some(Token::Op(s)) => s,
                _ => unreachable!(),
            };
            let right = self.parse_unary()?;
            left = match (to_num(&left), to_num(&right)) {
                (Some(l), Some(r)) => {
                    let v = match op.as_str() {
                        "*" => l * r,
                        "/" => if r == 0.0 { f64::NAN } else { l / r },
                        _ => if r == 0.0 { f64::NAN } else { l % r },
                    };
                    Operand::Number(v)
                }
                _ => Operand::Number(f64::NAN),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Operand, ()> {
        if self.peek_op() == Some("!") {
            self.next();
            let operand = self.parse_unary()?;
            return Ok(Operand::Bool(!to_bool(&operand)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Operand, ()> {
        let tok = self.next().ok_or(())?;
        match tok {
            Token::LParen => {
                let inner = self.parse_expression()?;
                match self.next() {
                    Some(Token::RParen) => Ok(inner),
                    _ => Err(()),
                }
            }
            Token::Number(text) => text.parse::<f64>().map(Operand::Number).map_err(|_| ()),
            Token::Str(text) => Ok(Operand::Str(text)),
            Token::Ident(text) => {
                if text == "true" {
                    return Ok(Operand::Bool(true));
                }
                if text == "false" {
                    return Ok(Operand::Bool(false));
                }
                match (self.resolve)(&text) {
                    None => {
                        self.absent_operand = true;
                        Ok(Operand::Number(f64::NAN))
                    }
                    Some(OdinValue::Null { .. }) => {
                        self.null_operand = true;
                        Ok(Operand::Null)
                    }
                    Some(value) => Ok(operand_from_value(&value)),
                }
            }
            _ => Err(()),
        }
    }
}

/// Extract a comparable operand from an `OdinValue`.
fn operand_from_value(value: &OdinValue) -> Operand {
    match value {
        OdinValue::Number { value: v, .. }
        | OdinValue::Currency { value: v, .. }
        | OdinValue::Percent { value: v, .. } => Operand::Number(*v),
        OdinValue::Integer { value: v, .. } => Operand::Number(*v as f64),
        OdinValue::String { value: v, .. } => Operand::Str(v.clone()),
        OdinValue::Boolean { value: v, .. } => Operand::Bool(*v),
        OdinValue::Date { .. } | OdinValue::Timestamp { .. } => {
            Operand::Number(temporal_ms(value).unwrap_or(f64::NAN))
        }
        _ => Operand::Number(f64::NAN),
    }
}

/// Millisecond key for a temporal value, mirroring numeric comparison.
fn temporal_ms(value: &OdinValue) -> Option<f64> {
    match value {
        OdinValue::Date { year, month, day, .. } => {
            Some((days_from_civil(*year as i64, *month as i64, *day as i64) * 86_400_000) as f64)
        }
        OdinValue::Timestamp { epoch_ms, .. } => Some(*epoch_ms as f64),
        _ => None,
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn to_num(v: &Operand) -> Option<f64> {
    match v {
        Operand::Number(n) => if n.is_nan() { None } else { Some(*n) },
        Operand::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn to_bool(v: &Operand) -> bool {
    match v {
        Operand::Bool(b) => *b,
        Operand::Number(n) => !n.is_nan() && *n != 0.0,
        Operand::Str(s) => !s.is_empty(),
        Operand::Null => false,
    }
}

fn loose_equals(a: &Operand, b: &Operand) -> bool {
    match (a, b) {
        (Operand::Number(x), Operand::Number(y)) => (x - y).abs() < EPSILON,
        (Operand::Str(x), Operand::Str(y)) => x == y,
        (Operand::Bool(x), Operand::Bool(y)) => x == y,
        (Operand::Null, Operand::Null) => true,
        _ => match (to_num(a), to_num(b)) {
            (Some(x), Some(y)) => (x - y).abs() < EPSILON,
            _ => false,
        },
    }
}

fn compare(a: &Operand, op: &str, b: &Operand) -> bool {
    if let (Some(x), Some(y)) = (to_num(a), to_num(b)) {
        return match op {
            ">" => x > y,
            "<" => x < y,
            ">=" => x >= y,
            "<=" => x <= y,
            _ => false,
        };
    }
    if let (Operand::Str(x), Operand::Str(y)) = (a, b) {
        return match op {
            ">" => x > y,
            "<" => x < y,
            ">=" => x >= y,
            "<=" => x <= y,
            _ => false,
        };
    }
    false
}
