//! What the calculator works out: an expression as typed ("2+3×4",
//! "√2", "sin(30)", "200+10%"), read in the usual order (powers first, then
//! × and ÷, then + and −, left to right), with a few calculator customs:
//! a number written against a bracket or a constant multiplies it ("2π",
//! "3(4+1)"); "a + b%" adds b percent of a, "a × b%" takes b percent; "!"
//! is the factorial. Brackets left open at the end are closed.

use crate::dec::{self, CalcError, Dec, Result};

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(Dec),
    Plus,
    Minus,
    Times,
    Divide,
    Power,
    Open,
    Close,
    Percent,
    Fact,
    Root,
    Func(Func),
    Pi,
    E,
    Ans,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Func {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Ln,
    Log,
}

impl Func {
    pub fn name(self) -> &'static str {
        match self {
            Func::Sin => "sin",
            Func::Cos => "cos",
            Func::Tan => "tan",
            Func::Asin => "asin",
            Func::Acos => "acos",
            Func::Atan => "atan",
            Func::Ln => "ln",
            Func::Log => "log",
        }
    }
}

const FUNCS: [Func; 8] = [Func::Asin, Func::Acos, Func::Atan, Func::Sin, Func::Cos, Func::Tan, Func::Ln, Func::Log];

fn tokens(s: &str) -> Result<Vec<Tok>> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' => i += 1,
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                out.push(Tok::Num(Dec::parse(&text).ok_or(CalcError::Syntax)?));
            }
            '+' => {
                out.push(Tok::Plus);
                i += 1;
            }
            '-' | '−' => {
                out.push(Tok::Minus);
                i += 1;
            }
            '*' | '×' => {
                out.push(Tok::Times);
                i += 1;
            }
            '/' | '÷' => {
                out.push(Tok::Divide);
                i += 1;
            }
            '^' => {
                out.push(Tok::Power);
                i += 1;
            }
            '(' => {
                out.push(Tok::Open);
                i += 1;
            }
            ')' => {
                out.push(Tok::Close);
                i += 1;
            }
            '%' => {
                out.push(Tok::Percent);
                i += 1;
            }
            '!' => {
                out.push(Tok::Fact);
                i += 1;
            }
            '√' => {
                out.push(Tok::Root);
                i += 1;
            }
            'π' => {
                out.push(Tok::Pi);
                i += 1;
            }
            _ => {
                let rest: String = chars[i..].iter().collect();
                if let Some(f) = FUNCS.iter().find(|f| rest.starts_with(f.name())) {
                    out.push(Tok::Func(*f));
                    i += f.name().chars().count();
                } else if rest.starts_with("Ans") {
                    out.push(Tok::Ans);
                    i += 3;
                } else if rest.starts_with("pi") {
                    out.push(Tok::Pi);
                    i += 2;
                } else if c == 'e' {
                    out.push(Tok::E);
                    i += 1;
                } else {
                    return Err(CalcError::Syntax);
                }
            }
        }
    }
    Ok(out)
}

/// What the expression is worked out with besides itself.
#[derive(Debug, Clone, Default)]
pub struct Context {
    /// Angles in degrees (else radians).
    pub degrees: bool,
    /// The last answer, for "Ans".
    pub ans: Option<Dec>,
}

struct Parser<'a> {
    toks: Vec<Tok>,
    at: usize,
    cx: &'a Context,
}

/// A value, and whether it was written as a percentage ("10%").
struct Val {
    v: Dec,
    percent: bool,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.at).cloned();
        self.at += 1;
        t
    }

    /// sum = product (('+' | '−') product)*
    fn sum(&mut self) -> Result<Val> {
        let mut left = self.product()?;
        while let Some(op) = self.peek().cloned().filter(|t| matches!(t, Tok::Plus | Tok::Minus)) {
            self.at += 1;
            let right = self.product()?;
            // a + b% adds b percent of a.
            let r = if right.percent { left.v.mul(&right.v)? } else { right.v };
            left = Val { v: if op == Tok::Plus { left.v.add(&r)? } else { left.v.sub(&r)? }, percent: false };
        }
        Ok(left)
    }

    /// product = power (('×' | '÷' | a value written against it) power)*
    fn product(&mut self) -> Result<Val> {
        let mut left = self.power()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Times) => Some(true),
                Some(Tok::Divide) => Some(false),
                // "2π", "3(4)", "2√9": a value against a value multiplies.
                Some(Tok::Open | Tok::Pi | Tok::E | Tok::Ans | Tok::Func(_) | Tok::Root | Tok::Num(_)) => None,
                _ => break,
            };
            if op.is_some() {
                self.at += 1;
            }
            let right = self.power()?;
            // "50 × 10%" takes 10 percent of 50: the percent is already a fraction.
            left = Val { v: if op == Some(false) { left.v.div(&right.v)? } else { left.v.mul(&right.v)? }, percent: false };
        }
        Ok(left)
    }

    /// power = unary ('^' power)?   (right to left: 2^3^2 is 2^9)
    fn power(&mut self) -> Result<Val> {
        let base = self.unary()?;
        if self.peek() == Some(&Tok::Power) {
            self.at += 1;
            let e = self.power()?;
            return Ok(Val { v: dec::pow(&base.v, &e.v)?, percent: false });
        }
        Ok(base)
    }

    /// unary = '−' unary | '√' unary | postfix
    fn unary(&mut self) -> Result<Val> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.at += 1;
                // −2^2 is −4, as written on paper.
                let v = self.power()?;
                Ok(Val { v: v.v.neg(), percent: v.percent })
            }
            Some(Tok::Plus) => {
                self.at += 1;
                self.unary()
            }
            Some(Tok::Root) => {
                self.at += 1;
                let v = self.unary()?;
                Ok(Val { v: dec::sqrt(&v.v)?, percent: false })
            }
            _ => self.postfix(),
        }
    }

    /// postfix = primary ('!' | '%')*
    fn postfix(&mut self) -> Result<Val> {
        let mut v = self.primary()?;
        loop {
            match self.peek() {
                Some(Tok::Fact) => {
                    self.at += 1;
                    v = Val { v: dec::factorial(&v.v)?, percent: false };
                }
                Some(Tok::Percent) => {
                    self.at += 1;
                    v = Val { v: v.v.div(&Dec::from_i64(100))?, percent: true };
                }
                _ => return Ok(v),
            }
        }
    }

    fn primary(&mut self) -> Result<Val> {
        let plain = |v: Dec| Ok(Val { v, percent: false });
        match self.next() {
            Some(Tok::Num(n)) => plain(n),
            Some(Tok::Pi) => plain(dec::pi().round(dec::DIGITS)),
            Some(Tok::E) => plain(dec::e()),
            Some(Tok::Ans) => plain(self.cx.ans.clone().unwrap_or_else(Dec::zero)),
            Some(Tok::Open) => {
                let v = self.sum()?;
                // A bracket left open at the end closes there.
                match self.peek() {
                    Some(Tok::Close) => self.at += 1,
                    None => {}
                    _ => return Err(CalcError::Syntax),
                }
                plain(v.v)
            }
            Some(Tok::Func(f)) => {
                // "sin 30" or "sin(30)": the function takes what follows.
                let x = self.unary()?.v;
                let deg = self.cx.degrees;
                plain(match f {
                    Func::Sin => dec::sin(&x, deg)?,
                    Func::Cos => dec::cos(&x, deg)?,
                    Func::Tan => dec::tan(&x, deg)?,
                    Func::Asin => dec::asin(&x, deg)?,
                    Func::Acos => dec::acos(&x, deg)?,
                    Func::Atan => dec::atan(&x, deg)?,
                    Func::Ln => dec::ln(&x)?,
                    Func::Log => dec::log10(&x)?,
                })
            }
            _ => Err(CalcError::Syntax),
        }
    }
}

/// Works out an expression.
pub fn eval(s: &str, cx: &Context) -> Result<Dec> {
    let toks = tokens(s)?;
    if toks.is_empty() {
        return Err(CalcError::Syntax);
    }
    let mut p = Parser { toks, at: 0, cx };
    let v = p.sum()?;
    if p.at != p.toks.len() {
        return Err(CalcError::Syntax);
    }
    Ok(v.v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> String {
        let cx = Context { degrees: true, ans: Some(Dec::from_i64(42)) };
        eval(s, &cx).map(|v| v.show(16)).unwrap_or_else(|e| format!("{e:?}"))
    }

    #[test]
    fn the_usual_order() {
        assert_eq!(ev("2+3×4"), "14");
        assert_eq!(ev("(2+3)×4"), "20");
        assert_eq!(ev("10−4−3"), "3");
        assert_eq!(ev("100÷10÷2"), "5");
        assert_eq!(ev("2^3^2"), "512");
        assert_eq!(ev("−2^2"), "-4");
        assert_eq!(ev("2×−3"), "-6");
        assert_eq!(ev("0.1+0.2"), "0.3");
        assert_eq!(ev("1÷3×3"), "1");
    }

    #[test]
    fn calculator_customs() {
        assert_eq!(ev("200+10%"), "220");
        assert_eq!(ev("200−10%"), "180");
        assert_eq!(ev("50×10%"), "5");
        assert_eq!(ev("10%"), "0.1");
        assert_eq!(ev("2π"), "6.283185307179586");
        assert_eq!(ev("3(4+1)"), "15");
        assert_eq!(ev("(1+2"), "3");
        assert_eq!(ev("5!"), "120");
        assert_eq!(ev("√16+1"), "5");
        assert_eq!(ev("2√9"), "6");
        assert_eq!(ev("Ans+1"), "43");
    }

    #[test]
    fn functions_in_degrees() {
        assert_eq!(ev("sin(30)"), "0.5");
        assert_eq!(ev("sin 30"), "0.5");
        assert_eq!(ev("cos(60)+sin(30)"), "1");
        assert_eq!(ev("log(1000)"), "3");
        assert_eq!(ev("ln(e)"), "1");
        assert_eq!(ev("asin(1)"), "90");
    }

    #[test]
    fn what_cannot_be_worked_out_says_why() {
        assert_eq!(ev("1÷0"), "DivideByZero");
        assert_eq!(ev("√(−4)"), "Domain");
        assert_eq!(ev("2+"), "Syntax");
        assert_eq!(ev("()"), "Syntax");
        assert_eq!(ev("2)"), "Syntax");
        assert_eq!(ev(""), "Syntax");
        assert_eq!(ev("1.2.3"), "Syntax");
    }
}
