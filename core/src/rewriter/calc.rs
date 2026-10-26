//! 電卓（ADR 0002 §12.2）。`1+2=` -> `3`、`1+2=3`。

use super::{RewriteCandidate, RewriteInput, Rewriter};

// 書式ID（固定）
const FORMAT_RESULT: u16 = 1; // 3
const FORMAT_EXPRESSION: u16 = 2; // 1+2=3

const TRIGGER: char = '=';
const OPERATORS: [char; 4] = ['+', '-', '*', '/'];
/// 式の長さの上限（文字）。病的な入力で重くしない
const MAX_EXPRESSION_CHARS: usize = 128;
/// 括弧の入れ子の上限。再帰でスタックを使い切らないため
const MAX_NESTING: usize = 32;
/// 小数の結果を丸める桁数
const FLOAT_DECIMALS: usize = 10;
/// これ以上の絶対値の小数は、桁が足りなくなるので出さない
const FLOAT_ABS_LIMIT: f64 = 1e15;

#[derive(Clone, Copy, Debug, Default)]
pub struct CalculatorRewriter;

impl CalculatorRewriter {
    pub fn new() -> Self {
        Self
    }
}

impl Rewriter for CalculatorRewriter {
    fn name(&self) -> &'static str {
        "calc"
    }

    fn rewrite(&self, input: &RewriteInput<'_>, out: &mut Vec<RewriteCandidate>) {
        let Some(expr) = input.reading.strip_suffix(TRIGGER) else {
            return;
        };
        if expr.chars().count() > MAX_EXPRESSION_CHARS || !expr.contains(OPERATORS) {
            return;
        }
        let Some(value) = evaluate(expr).and_then(Value::display) else {
            return;
        };
        out.push(RewriteCandidate::new(value.clone(), FORMAT_RESULT));
        out.push(RewriteCandidate::new(
            format!("{expr}={value}"),
            FORMAT_EXPRESSION,
        ));
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Value {
    Int(i64),
    Float(f64),
}

impl Value {
    fn as_f64(self) -> f64 {
        match self {
            Value::Int(i) => i as f64,
            Value::Float(f) => f,
        }
    }

    fn display(self) -> Option<String> {
        match self {
            Value::Int(i) => Some(i.to_string()),
            Value::Float(f) => {
                if !f.is_finite() || f.abs() >= FLOAT_ABS_LIMIT {
                    return None;
                }
                let s = format!("{f:.FLOAT_DECIMALS$}");
                let s = s.trim_end_matches('0').trim_end_matches('.');
                Some(if s == "-0" || s.is_empty() { "0" } else { s }.to_owned())
            }
        }
    }

    /// 結果が有限の小数でなければ None
    fn float(f: f64) -> Option<Value> {
        f.is_finite().then_some(Value::Float(f))
    }

    fn add(self, rhs: Value) -> Option<Value> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => a.checked_add(b).map(Value::Int),
            _ => Value::float(self.as_f64() + rhs.as_f64()),
        }
    }

    fn sub(self, rhs: Value) -> Option<Value> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => a.checked_sub(b).map(Value::Int),
            _ => Value::float(self.as_f64() - rhs.as_f64()),
        }
    }

    fn mul(self, rhs: Value) -> Option<Value> {
        match (self, rhs) {
            (Value::Int(a), Value::Int(b)) => a.checked_mul(b).map(Value::Int),
            _ => Value::float(self.as_f64() * rhs.as_f64()),
        }
    }

    /// 0除算は None。整数同士で割り切れなければ小数にする。
    fn div(self, rhs: Value) -> Option<Value> {
        match (self, rhs) {
            (Value::Int(_), Value::Int(0)) => None,
            (Value::Int(a), Value::Int(b)) => match a.checked_rem(b) {
                Some(0) => a.checked_div(b).map(Value::Int),
                Some(_) => Value::float(a as f64 / b as f64),
                None => None, // i64::MIN / -1
            },
            _ => {
                let d = rhs.as_f64();
                if d == 0.0 {
                    None
                } else {
                    Value::float(self.as_f64() / d)
                }
            }
        }
    }

    fn neg(self) -> Option<Value> {
        match self {
            Value::Int(a) => a.checked_neg().map(Value::Int),
            Value::Float(f) => Some(Value::Float(-f)),
        }
    }
}

/// 式を評価する。文法が不正、0除算、桁あふれのどれでも None。
fn evaluate(expr: &str) -> Option<Value> {
    let mut p = Parser {
        src: expr.as_bytes(),
        pos: 0,
        depth: 0,
    };
    let v = p.expr()?;
    (p.pos == p.src.len()).then_some(v)
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    // expr := term (('+' | '-') term)*
    fn expr(&mut self) -> Option<Value> {
        let mut acc = self.term()?;
        loop {
            if self.eat(b'+') {
                acc = acc.add(self.term()?)?;
            } else if self.eat(b'-') {
                acc = acc.sub(self.term()?)?;
            } else {
                return Some(acc);
            }
        }
    }

    // term := unary (('*' | '/') unary)*
    fn term(&mut self) -> Option<Value> {
        let mut acc = self.unary()?;
        loop {
            if self.eat(b'*') {
                acc = acc.mul(self.unary()?)?;
            } else if self.eat(b'/') {
                acc = acc.div(self.unary()?)?;
            } else {
                return Some(acc);
            }
        }
    }

    // unary := ('-' | '+') unary | primary
    fn unary(&mut self) -> Option<Value> {
        if self.eat(b'-') {
            self.enter()?;
            let v = self.unary().and_then(Value::neg);
            self.depth -= 1;
            v
        } else if self.eat(b'+') {
            self.enter()?;
            let v = self.unary();
            self.depth -= 1;
            v
        } else {
            self.primary()
        }
    }

    // primary := number | '(' expr ')'
    fn primary(&mut self) -> Option<Value> {
        if self.eat(b'(') {
            self.enter()?;
            let v = self.expr();
            self.depth -= 1;
            let v = v?;
            return self.eat(b')').then_some(v);
        }
        self.number()
    }

    // number := digit+ ('.' digit+)?
    fn number(&mut self) -> Option<Value> {
        let start = self.pos;
        self.digits()?;
        let is_float = self.peek() == Some(b'.');
        if is_float {
            self.pos += 1;
            self.digits()?;
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).ok()?;
        if is_float {
            text.parse().ok().and_then(Value::float)
        } else {
            text.parse().ok().map(Value::Int)
        }
    }

    /// 1桁以上の数字を読み進める。なければ None
    fn digits(&mut self) -> Option<()> {
        let start = self.pos;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
        (self.pos > start).then_some(())
    }

    fn enter(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MAX_NESTING).then_some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Context;
    use crate::rewriter::LocalDateTime;

    fn run(reading: &str) -> Vec<(u16, String)> {
        let ctx = Context::new();
        let input = RewriteInput {
            reading,
            context: &ctx,
            now: LocalDateTime {
                year: 2026,
                month: 10,
                day: 11,
                hour: 0,
                minute: 0,
                weekday: 6,
            },
        };
        let mut out = Vec::new();
        CalculatorRewriter::new().rewrite(&input, &mut out);
        out.into_iter().map(|c| (c.format, c.surface)).collect()
    }

    fn result(reading: &str) -> Option<String> {
        run(reading).into_iter().next().map(|(_, s)| s)
    }

    #[test]
    fn spec_example() {
        assert_eq!(
            run("1+2="),
            vec![(1, "3".to_owned()), (2, "1+2=3".to_owned())]
        );
    }

    #[test]
    fn precedence_and_parentheses() {
        assert_eq!(result("1+2*3=").as_deref(), Some("7"));
        assert_eq!(result("(1+2)*3=").as_deref(), Some("9"));
        assert_eq!(result("10-3-2=").as_deref(), Some("5"));
        assert_eq!(result("100/10/5=").as_deref(), Some("2"));
        assert_eq!(result("2*(3+(4-1))*2=").as_deref(), Some("24"));
    }

    #[test]
    fn unary_signs() {
        assert_eq!(result("-3+5=").as_deref(), Some("2"));
        assert_eq!(result("(-3)*(-3)=").as_deref(), Some("9"));
        assert_eq!(result("2*-3=").as_deref(), Some("-6"));
        assert_eq!(result("--3+1=").as_deref(), Some("4"));
        assert_eq!(result("+3+1=").as_deref(), Some("4"));
    }

    #[test]
    fn division_exact_and_fractional() {
        assert_eq!(result("6/3=").as_deref(), Some("2"));
        assert_eq!(result("1/2=").as_deref(), Some("0.5"));
        assert_eq!(result("1/3=").as_deref(), Some("0.3333333333"));
        assert_eq!(result("2/3=").as_deref(), Some("0.6666666667"));
        assert_eq!(result("-7/2=").as_deref(), Some("-3.5"));
    }

    #[test]
    fn decimals_do_not_leak_binary_noise() {
        assert_eq!(result("0.1+0.2=").as_deref(), Some("0.3"));
        assert_eq!(result("1.5*2=").as_deref(), Some("3"));
        assert_eq!(result("3.14*2=").as_deref(), Some("6.28"));
        assert_eq!(result("0.5-0.5=").as_deref(), Some("0"));
        assert_eq!(result("-0.5+0.5=").as_deref(), Some("0"));
    }

    #[test]
    fn expression_form_keeps_what_was_typed() {
        let v = run("(1+2)*3=");
        assert_eq!(v[1], (2, "(1+2)*3=9".to_owned()));
    }

    #[test]
    fn division_by_zero_gives_no_candidate() {
        for r in ["1/0=", "0/0=", "1/(2-2)=", "1.5/0=", "1/0.0=", "5/(1-1)+3="] {
            assert!(run(r).is_empty(), "{r}");
        }
    }

    #[test]
    fn overflow_gives_no_candidate() {
        assert!(run("9223372036854775807+1=").is_empty());
        assert!(run("9223372036854775808+1=").is_empty());
        assert!(run("9223372036854775807*2=").is_empty());
        assert!(run("-9223372036854775807-2=").is_empty());
        assert_eq!(
            result("9223372036854775807-1=").as_deref(),
            Some("9223372036854775806")
        );
        // 巨大な小数は桁が足りないので出さない
        assert!(run("99999999999999999999999.5+1=").is_empty());
    }

    #[test]
    fn i64_min_edge_cases() {
        assert_eq!(
            result("-9223372036854775807-1=").as_deref(),
            Some("-9223372036854775808")
        );
        assert!(run("(-9223372036854775807-1)/-1=").is_empty());
    }

    #[test]
    fn malformed_expressions_give_nothing() {
        for r in [
            "=", "1+=", "+=", "(1+2=", "1+2)=", "()=", "1++=", "*2=", "1 +2=", "1..2+3=", ".5+1=",
            "5.+1=", "1+2", "1+2=3", "abc=", "1,000+1=", "２+３=", "1x2=", "1+2==",
        ] {
            assert!(run(r).is_empty(), "{r:?} -> {:?}", run(r));
        }
    }

    #[test]
    fn needs_an_operator() {
        assert!(run("5=").is_empty());
        assert!(run("(5)=").is_empty());
        assert!(run("12.5=").is_empty());
    }

    #[test]
    fn nesting_and_length_limits() {
        let deep = format!("{}1{}+1=", "(".repeat(MAX_NESTING), ")".repeat(MAX_NESTING));
        assert_eq!(result(&deep).as_deref(), Some("2"));
        let too_deep = format!(
            "{}1{}+1=",
            "(".repeat(MAX_NESTING + 1),
            ")".repeat(MAX_NESTING + 1)
        );
        assert!(run(&too_deep).is_empty());
        let many_minus = format!("{}1+1=", "-".repeat(MAX_NESTING + 5));
        assert!(run(&many_minus).is_empty());
        let long = format!("{}=", "1+".repeat(MAX_EXPRESSION_CHARS));
        assert!(run(&long).is_empty());
        let huge_stack = format!("{}1=", "(".repeat(100_000));
        assert!(run(&huge_stack).is_empty());
    }

    #[test]
    fn format_ids_are_stable() {
        assert_eq!([FORMAT_RESULT, FORMAT_EXPRESSION], [1, 2]);
    }
}
