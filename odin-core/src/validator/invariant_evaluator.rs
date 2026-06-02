//! Invariant expression evaluation.
//!
//! Recursive-descent parser over the invariant grammar:
//!   expression     = logic_or
//!   logic_or       = logic_and , { "||" , logic_and }
//!   logic_and      = equality , { "&&" , equality }
//!   equality       = comparison , { ( "==" | "!=" | "=" ) , comparison }
//!   comparison     = additive , { ( ">" | "<" | ">=" | "<=" ) , additive }
//!   additive       = multiplicative , { ( "+" | "-" ) , multiplicative }
//!   multiplicative = unary , { ( "*" | "/" | "%" ) , unary }
//!   unary          = [ "!" ] , primary
//!   primary        = path | number | string | "(" , expression , ")"
//!
//! An expression is parsed to an AST once and cached by its source string; each
//! document validation evaluates the cached AST against that document's values.

use std::borrow::Cow;
use std::sync::Arc;

use crate::types::values::OdinValue;

const EPSILON: f64 = 1e-9;

/// A resolved operand value. String operands borrow from the document value or
/// the parsed node, avoiding a clone on every evaluation.
#[derive(Clone, Debug)]
enum Operand<'a> {
    Number(f64),
    Str(Cow<'a, str>),
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

/// A parsed, document-independent invariant expression node.
#[derive(Clone, Debug)]
pub(crate) enum Node {
    Number(f64),
    Str(String),
    Bool(bool),
    Field(String),
    Not(Box<Node>),
    Logic { op: LogicOp, left: Box<Node>, right: Box<Node> },
    Equality { negate: bool, left: Box<Node>, right: Box<Node> },
    Compare { op: CmpOp, left: Box<Node>, right: Box<Node> },
    Additive { subtract: bool, left: Box<Node>, right: Box<Node> },
    Multiplicative { op: MulOp, left: Box<Node>, right: Box<Node> },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum LogicOp { And, Or }

#[derive(Clone, Copy, Debug)]
pub(crate) enum CmpOp { Gt, Lt, Gte, Lte }

#[derive(Clone, Copy, Debug)]
pub(crate) enum MulOp { Mul, Div, Rem }

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

/// A parse result: the prepared AST, or `Err` for a malformed expression.
pub(crate) type AstEntry = Result<Arc<Node>, ()>;

/// Parse an invariant expression into a prepared AST, stored on the per-schema
/// memo so evaluation needs no global lock. Returns `Err` on malformed input.
pub(crate) fn parse_invariant(expr: &str) -> AstEntry {
    parse_to_ast(expr).map(Arc::new)
}

/// Parse an invariant expression to an AST. Returns `Err` on malformed input.
fn parse_to_ast(expr: &str) -> Result<Node, ()> {
    let tokens = tokenize(expr)?;
    let mut parser = AstParser { tokens, pos: 0 };
    let ast = parser.parse_expression()?;
    if parser.pos != parser.tokens.len() {
        return Err(()); // trailing tokens
    }
    Ok(ast)
}

/// Evaluate a prepared invariant AST against a document. `resolve` returns a
/// borrowed document value at a field name, or `None` if absent.
pub(crate) fn evaluate_ast<'a, F>(ast: &'a Node, resolve: F) -> InvariantResult
where
    F: Fn(&str) -> Option<&'a OdinValue>,
{
    let mut state = EvalState {
        resolve: &resolve,
        absent_operand: false,
        null_operand: false,
    };
    let final_val = eval_node(ast, &mut state);

    let value = if state.null_operand {
        Some(false)
    } else if state.absent_operand {
        None
    } else {
        Some(to_bool(&final_val))
    };

    InvariantResult { value, null_operand: state.null_operand }
}

struct AstParser {
    tokens: Vec<Token>,
    pos: usize,
}

impl AstParser {
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

    fn parse_expression(&mut self) -> Result<Node, ()> {
        self.parse_logic_or()
    }

    fn parse_logic_or(&mut self) -> Result<Node, ()> {
        let mut left = self.parse_logic_and()?;
        while self.peek_op() == Some("||") {
            self.next();
            let right = self.parse_logic_and()?;
            left = Node::Logic { op: LogicOp::Or, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_logic_and(&mut self) -> Result<Node, ()> {
        let mut left = self.parse_equality()?;
        while self.peek_op() == Some("&&") {
            self.next();
            let right = self.parse_equality()?;
            left = Node::Logic { op: LogicOp::And, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Node, ()> {
        let mut left = self.parse_comparison()?;
        while matches!(self.peek_op(), Some("==") | Some("!=") | Some("=")) {
            let op = match self.next() {
                Some(Token::Op(s)) => s,
                _ => unreachable!(),
            };
            let right = self.parse_comparison()?;
            left = Node::Equality {
                negate: op == "!=",
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Node, ()> {
        let mut left = self.parse_additive()?;
        while matches!(self.peek_op(), Some(">") | Some("<") | Some(">=") | Some("<=")) {
            let op = match self.next() {
                Some(Token::Op(s)) => s,
                _ => unreachable!(),
            };
            let cmp = match op.as_str() {
                ">" => CmpOp::Gt,
                "<" => CmpOp::Lt,
                ">=" => CmpOp::Gte,
                _ => CmpOp::Lte,
            };
            let right = self.parse_additive()?;
            left = Node::Compare { op: cmp, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Node, ()> {
        let mut left = self.parse_multiplicative()?;
        while matches!(self.peek_op(), Some("+") | Some("-")) {
            let op = match self.next() {
                Some(Token::Op(s)) => s,
                _ => unreachable!(),
            };
            let right = self.parse_multiplicative()?;
            left = Node::Additive {
                subtract: op == "-",
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Node, ()> {
        let mut left = self.parse_unary()?;
        while matches!(self.peek_op(), Some("*") | Some("/") | Some("%")) {
            let op = match self.next() {
                Some(Token::Op(s)) => s,
                _ => unreachable!(),
            };
            let mul = match op.as_str() {
                "*" => MulOp::Mul,
                "/" => MulOp::Div,
                _ => MulOp::Rem,
            };
            let right = self.parse_unary()?;
            left = Node::Multiplicative { op: mul, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Node, ()> {
        if self.peek_op() == Some("!") {
            self.next();
            let operand = self.parse_unary()?;
            return Ok(Node::Not(Box::new(operand)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Node, ()> {
        let tok = self.next().ok_or(())?;
        match tok {
            Token::LParen => {
                let inner = self.parse_expression()?;
                match self.next() {
                    Some(Token::RParen) => Ok(inner),
                    _ => Err(()),
                }
            }
            Token::Number(text) => text.parse::<f64>().map(Node::Number).map_err(|_| ()),
            Token::Str(text) => Ok(Node::Str(text)),
            Token::Ident(text) => {
                if text == "true" {
                    return Ok(Node::Bool(true));
                }
                if text == "false" {
                    return Ok(Node::Bool(false));
                }
                Ok(Node::Field(text))
            }
            _ => Err(()),
        }
    }
}

/// Per-evaluation state tracking absent/null operands.
struct EvalState<'a, 'b, F: Fn(&str) -> Option<&'a OdinValue>> {
    resolve: &'b F,
    absent_operand: bool,
    null_operand: bool,
}

fn eval_node<'a, 'b, F>(node: &'a Node, state: &mut EvalState<'a, 'b, F>) -> Operand<'a>
where
    F: Fn(&str) -> Option<&'a OdinValue>,
{
    match node {
        Node::Number(n) => Operand::Number(*n),
        Node::Str(s) => Operand::Str(Cow::Borrowed(s.as_str())),
        Node::Bool(b) => Operand::Bool(*b),
        Node::Field(name) => match (state.resolve)(name) {
            None => {
                state.absent_operand = true;
                Operand::Number(f64::NAN)
            }
            Some(OdinValue::Null { .. }) => {
                state.null_operand = true;
                Operand::Null
            }
            Some(value) => operand_from_value(value),
        },
        Node::Not(operand) => Operand::Bool(!to_bool(&eval_node(operand, state))),
        Node::Logic { op, left, right } => {
            let l = to_bool(&eval_node(left, state));
            let r = to_bool(&eval_node(right, state));
            Operand::Bool(match op {
                LogicOp::Or => l || r,
                LogicOp::And => l && r,
            })
        }
        Node::Equality { negate, left, right } => {
            let l = eval_node(left, state);
            let r = eval_node(right, state);
            let eq = loose_equals(&l, &r);
            Operand::Bool(if *negate { !eq } else { eq })
        }
        Node::Compare { op, left, right } => {
            let l = eval_node(left, state);
            let r = eval_node(right, state);
            Operand::Bool(compare(&l, *op, &r))
        }
        Node::Additive { subtract, left, right } => {
            let l = eval_node(left, state);
            let r = eval_node(right, state);
            match (to_num(&l), to_num(&r)) {
                (Some(a), Some(b)) => Operand::Number(if *subtract { a - b } else { a + b }),
                _ => Operand::Number(f64::NAN),
            }
        }
        Node::Multiplicative { op, left, right } => {
            let l = eval_node(left, state);
            let r = eval_node(right, state);
            match (to_num(&l), to_num(&r)) {
                (Some(a), Some(b)) => {
                    let v = match op {
                        MulOp::Mul => a * b,
                        MulOp::Div => if b == 0.0 { f64::NAN } else { a / b },
                        MulOp::Rem => if b == 0.0 { f64::NAN } else { a % b },
                    };
                    Operand::Number(v)
                }
                _ => Operand::Number(f64::NAN),
            }
        }
    }
}

/// Extract a comparable operand from an `OdinValue`, borrowing string contents.
fn operand_from_value(value: &OdinValue) -> Operand<'_> {
    match value {
        OdinValue::Number { value: v, .. }
        | OdinValue::Currency { value: v, .. }
        | OdinValue::Percent { value: v, .. } => Operand::Number(*v),
        OdinValue::Integer { value: v, .. } => Operand::Number(*v as f64),
        OdinValue::String { value: v, .. } => Operand::Str(Cow::Borrowed(v.as_str())),
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

fn compare(a: &Operand, op: CmpOp, b: &Operand) -> bool {
    if let (Some(x), Some(y)) = (to_num(a), to_num(b)) {
        return match op {
            CmpOp::Gt => x > y,
            CmpOp::Lt => x < y,
            CmpOp::Gte => x >= y,
            CmpOp::Lte => x <= y,
        };
    }
    if let (Operand::Str(x), Operand::Str(y)) = (a, b) {
        return match op {
            CmpOp::Gt => x > y,
            CmpOp::Lt => x < y,
            CmpOp::Gte => x >= y,
            CmpOp::Lte => x <= y,
        };
    }
    false
}
