//! `%expr` formula macro.
//!
//! Compiles an infix arithmetic formula string into a tree of existing transform
//! verbs at parse time. There is no runtime evaluator: the result is an ordinary
//! verb expression, so the arithmetic runs through the deterministic verbs (add,
//! subtract, multiply, divide, mod, negate, pow, and a whitelist of numeric
//! functions). Variables resolve under an explicit bindings object passed as the
//! second argument: in `%expr "a + b" @.vars`, the name `a` reads `@.vars.a`.
//!
//! Precedence, high to low:
//!   1. parentheses, function call
//!   2. `^` power (right-associative)
//!   3. unary `-` / `+` (looser than `^`, so `-2^2 = -(2^2) = -4`; `(-2)^2 = 4`)
//!   4. `* / %` (left-associative)
//!   5. `+ -` (left-associative)

use crate::types::transform::{VerbArg, VerbCall};
use crate::types::values::OdinValues;

/// Error compiling a `%expr` formula; carries the T015 code.
#[derive(Debug)]
pub struct ExprError(pub String);

impl std::fmt::Display for ExprError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[T015] Invalid %expr formula: {}", self.0)
    }
}

/// Binary operator -> verb.
fn binary_op(op: char) -> &'static str {
    match op {
        '+' => "add",
        '-' => "subtract",
        '*' => "multiply",
        '/' => "divide",
        '%' => "mod",
        _ => unreachable!(),
    }
}

/// Whitelisted function -> (verb, min args, max args). Infinity is i32::MAX.
fn function(name: &str) -> Option<(&'static str, usize, usize)> {
    Some(match name {
        "abs" => ("abs", 1, 1),
        "floor" => ("floor", 1, 1),
        "ceil" => ("ceil", 1, 1),
        "trunc" => ("trunc", 1, 1),
        "sqrt" => ("sqrt", 1, 1),
        "round" => ("round", 1, 2),
        "pow" => ("pow", 2, 2),
        "min" => ("minOf", 1, usize::MAX),
        "max" => ("maxOf", 1, usize::MAX),
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(String, bool),
    Ident(String),
    Op(char),
    LParen,
    RParen,
    Comma,
}

fn tokenize(src: &str) -> Result<Vec<Tok>, ExprError> {
    let chars: Vec<char> = src.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            let mut is_float = false;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i < chars.len() && chars[i] == '.' {
                is_float = true;
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                is_float = true;
                i += 1;
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    i += 1;
                }
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            tokens.push(Tok::Num(chars[start..i].iter().collect(), is_float));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '.')
            {
                i += 1;
            }
            tokens.push(Tok::Ident(chars[start..i].iter().collect()));
            continue;
        }
        match c {
            '(' => tokens.push(Tok::LParen),
            ')' => tokens.push(Tok::RParen),
            ',' => tokens.push(Tok::Comma),
            '+' | '-' | '*' | '/' | '%' | '^' => tokens.push(Tok::Op(c)),
            _ => return Err(ExprError(format!("unexpected character '{c}'"))),
        }
        i += 1;
    }
    Ok(tokens)
}

fn literal(text: &str, is_float: bool) -> VerbArg {
    if is_float {
        VerbArg::Literal(OdinValues::number(text.parse::<f64>().unwrap_or(0.0)))
    } else {
        VerbArg::Literal(OdinValues::integer(text.parse::<i64>().unwrap_or(0)))
    }
}

fn verb_node(verb: &str, args: Vec<VerbArg>) -> VerbArg {
    VerbArg::Verb(VerbCall { verb: verb.to_string(), is_custom: false, args })
}

struct Parser<'a> {
    tokens: Vec<Tok>,
    pos: usize,
    binding_path: Option<&'a str>,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        t
    }

    fn parse(&mut self) -> Result<VerbArg, ExprError> {
        if self.tokens.is_empty() {
            return Err(ExprError("empty formula".to_string()));
        }
        let expr = self.parse_additive()?;
        if self.pos < self.tokens.len() {
            return Err(ExprError(format!("unexpected token '{}'", tok_text(&self.tokens[self.pos]))));
        }
        Ok(expr)
    }

    fn parse_additive(&mut self) -> Result<VerbArg, ExprError> {
        let mut left = self.parse_multiplicative()?;
        while let Some(Tok::Op(op @ ('+' | '-'))) = self.peek().cloned() {
            self.next();
            let right = self.parse_multiplicative()?;
            left = verb_node(binary_op(op), vec![left, right]);
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<VerbArg, ExprError> {
        let mut left = self.parse_unary()?;
        while let Some(Tok::Op(op @ ('*' | '/' | '%'))) = self.peek().cloned() {
            self.next();
            let right = self.parse_unary()?;
            left = verb_node(binary_op(op), vec![left, right]);
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<VerbArg, ExprError> {
        if let Some(Tok::Op(op @ ('-' | '+'))) = self.peek().cloned() {
            self.next();
            let operand = self.parse_unary()?;
            return Ok(if op == '-' { verb_node("negate", vec![operand]) } else { operand });
        }
        self.parse_power()
    }

    fn parse_power(&mut self) -> Result<VerbArg, ExprError> {
        let base = self.parse_primary()?;
        if let Some(Tok::Op('^')) = self.peek() {
            self.next();
            let exponent = self.parse_unary()?;
            return Ok(verb_node("pow", vec![base, exponent]));
        }
        Ok(base)
    }

    fn parse_primary(&mut self) -> Result<VerbArg, ExprError> {
        let t = self.next().ok_or_else(|| ExprError("unexpected end of formula".to_string()))?;
        match t {
            Tok::Num(text, is_float) => Ok(literal(&text, is_float)),
            Tok::LParen => {
                let inner = self.parse_additive()?;
                match self.next() {
                    Some(Tok::RParen) => Ok(inner),
                    _ => Err(ExprError("missing closing parenthesis".to_string())),
                }
            }
            Tok::Ident(name) => {
                if let Some(Tok::LParen) = self.peek() {
                    self.parse_call(&name)
                } else {
                    match self.binding_path {
                        Some(path) => Ok(VerbArg::Reference(format!("{path}.{name}"), Vec::new())),
                        None => Err(ExprError(format!(
                            "variable '{name}' requires a bindings object, e.g. %expr \"...\" @.vars"
                        ))),
                    }
                }
            }
            other => Err(ExprError(format!("unexpected token '{}'", tok_text(&other)))),
        }
    }

    fn parse_call(&mut self, name: &str) -> Result<VerbArg, ExprError> {
        let (verb, min, max) = function(name)
            .ok_or_else(|| ExprError(format!("unknown function '{name}'")))?;
        self.next(); // consume '('
        let mut args = Vec::new();
        if self.peek() != Some(&Tok::RParen) {
            args.push(self.parse_additive()?);
            while self.peek() == Some(&Tok::Comma) {
                self.next();
                args.push(self.parse_additive()?);
            }
        }
        match self.next() {
            Some(Tok::RParen) => {}
            _ => return Err(ExprError(format!("missing ) after {name}("))),
        }
        if args.len() < min || args.len() > max {
            let bounds = if min == max { min.to_string() } else { format!("{min}-{max}") };
            return Err(ExprError(format!(
                "{name}() takes {bounds} arguments, got {}", args.len()
            )));
        }
        if name == "round" && args.len() == 1 {
            args.push(VerbArg::Literal(OdinValues::integer(0)));
        }
        Ok(verb_node(verb, args))
    }
}

fn tok_text(t: &Tok) -> String {
    match t {
        Tok::Num(s, _) | Tok::Ident(s) => s.clone(),
        Tok::Op(c) => c.to_string(),
        Tok::LParen => "(".to_string(),
        Tok::RParen => ")".to_string(),
        Tok::Comma => ",".to_string(),
    }
}

/// Compile a formula into a verb-argument tree. `binding_path` is the path of
/// the bindings object (e.g. `.vars`); a variable used without it is an error.
pub fn compile_expr(formula: &str, binding_path: Option<&str>) -> Result<VerbArg, ExprError> {
    let tokens = tokenize(formula)?;
    Parser { tokens, pos: 0, binding_path }.parse()
}
