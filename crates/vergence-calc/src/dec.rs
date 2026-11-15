//! Exact decimal numbers for the calculator (tools milestone 1): a number is
//! a sign, decimal digits and a power of ten, so 0.1 is exactly one tenth
//! and 0.1 + 0.2 is 0.3. Results are rounded to [`DIGITS`] significant
//! digits, half to even (as IEEE decimal128 keeps 34); the scientific
//! functions are worked out with guard digits ([`WORK`]) and then rounded,
//! so what they show is right to the last digit shown. Our own code, digit
//! by digit: a calculator's numbers are short, and plain long arithmetic is
//! fast enough and easy to check.

use std::cmp::Ordering;

/// Significant digits a result keeps.
pub const DIGITS: usize = 34;
/// Digits the functions work with before rounding to [`DIGITS`].
pub const WORK: usize = DIGITS + 12;

/// Why a calculation has no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalcError {
    DivideByZero,
    /// Outside a function's domain (the root of a negative, the logarithm of 0 …).
    Domain,
    /// Too large to work out (or to show).
    TooLarge,
    Syntax,
}

impl std::fmt::Display for CalcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CalcError::DivideByZero => "Can't divide by zero",
            CalcError::Domain => "Not a real number",
            CalcError::TooLarge => "Too large",
            CalcError::Syntax => "Not a complete calculation",
        })
    }
}

pub type Result<T> = std::result::Result<T, CalcError>;

/// A decimal number: `coef × 10^exp`, the coefficient's digits least
/// significant first, none of them zero at its top; zero has no digits.
#[derive(Clone, Debug)]
pub struct Dec {
    neg: bool,
    coef: Vec<u8>,
    exp: i64,
}

/// The largest power of ten a number may reach before it is too large.
const MAX_EXP: i64 = 100_000;

// ---- digit vectors, least significant first ----

fn trim(v: &mut Vec<u8>) {
    while v.last() == Some(&0) {
        v.pop();
    }
}

fn cmp_mag(a: &[u8], b: &[u8]) -> Ordering {
    if a.len() != b.len() {
        return a.len().cmp(&b.len());
    }
    for i in (0..a.len()).rev() {
        match a[i].cmp(&b[i]) {
            Ordering::Equal => continue,
            o => return o,
        }
    }
    Ordering::Equal
}

fn add_mag(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
    let mut carry = 0u8;
    for i in 0..a.len().max(b.len()) {
        let s = a.get(i).copied().unwrap_or(0) + b.get(i).copied().unwrap_or(0) + carry;
        out.push(s % 10);
        carry = s / 10;
    }
    if carry > 0 {
        out.push(carry);
    }
    out
}

/// a - b, a at least b.
fn sub_mag(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(a.len());
    let mut borrow = 0i8;
    for i in 0..a.len() {
        let mut d = a[i] as i8 - b.get(i).copied().unwrap_or(0) as i8 - borrow;
        borrow = if d < 0 {
            d += 10;
            1
        } else {
            0
        };
        out.push(d as u8);
    }
    trim(&mut out);
    out
}

fn mul_mag(a: &[u8], b: &[u8]) -> Vec<u8> {
    if a.is_empty() || b.is_empty() {
        return Vec::new();
    }
    let mut acc = vec![0u32; a.len() + b.len()];
    for (i, &x) in a.iter().enumerate() {
        for (j, &y) in b.iter().enumerate() {
            acc[i + j] += x as u32 * y as u32;
        }
    }
    let mut out = Vec::with_capacity(acc.len());
    let mut carry = 0u32;
    for v in acc {
        let s = v + carry;
        out.push((s % 10) as u8);
        carry = s / 10;
    }
    while carry > 0 {
        out.push((carry % 10) as u8);
        carry /= 10;
    }
    trim(&mut out);
    out
}

/// a / b and its remainder, b not zero.
fn divmod_mag(a: &[u8], b: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let mut q = vec![0u8; a.len()];
    let mut r: Vec<u8> = Vec::new();
    for i in (0..a.len()).rev() {
        r.insert(0, a[i]);
        trim(&mut r);
        let mut d = 0u8;
        while cmp_mag(&r, b) != Ordering::Less {
            r = sub_mag(&r, b);
            d += 1;
        }
        q[i] = d;
    }
    trim(&mut q);
    (q, r)
}

/// `v × 10^n`.
fn shift(v: &[u8], n: usize) -> Vec<u8> {
    if v.is_empty() {
        return Vec::new();
    }
    let mut out = vec![0u8; n];
    out.extend_from_slice(v);
    out
}

impl Dec {
    pub fn zero() -> Dec {
        Dec { neg: false, coef: Vec::new(), exp: 0 }
    }

    pub fn from_i64(n: i64) -> Dec {
        let mut v = Vec::new();
        let mut m = n.unsigned_abs();
        while m > 0 {
            v.push((m % 10) as u8);
            m /= 10;
        }
        Dec { neg: n < 0, coef: v, exp: 0 }.norm()
    }

    /// A number as written: digits, at most one point, an optional exponent
    /// ("1.5e-3"); a leading minus.
    pub fn parse(s: &str) -> Option<Dec> {
        let s = s.trim();
        let (neg, s) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let (mant, exp) = match s.find(['e', 'E']) {
            Some(i) => (&s[..i], s[i + 1..].parse::<i64>().ok()?),
            None => (s, 0),
        };
        if mant.is_empty() || mant.chars().filter(|c| *c == '.').count() > 1 {
            return None;
        }
        let mut digits = Vec::new();
        let mut after = 0i64;
        let mut seen_point = false;
        for c in mant.chars() {
            match c {
                '.' => seen_point = true,
                '0'..='9' => {
                    digits.push(c as u8 - b'0');
                    if seen_point {
                        after += 1;
                    }
                }
                _ => return None,
            }
        }
        if digits.is_empty() || exp.abs() > MAX_EXP {
            return None;
        }
        digits.reverse();
        Some(Dec { neg, coef: digits, exp: exp - after }.norm().round(DIGITS))
    }

    /// Leading zeros away, trailing zeros into the exponent; no negative zero.
    fn norm(mut self) -> Dec {
        trim(&mut self.coef);
        let zeros = self.coef.iter().take_while(|d| **d == 0).count();
        if zeros > 0 {
            self.coef.drain(..zeros);
            self.exp += zeros as i64;
        }
        if self.coef.is_empty() {
            self.neg = false;
            self.exp = 0;
        }
        self
    }

    pub fn is_zero(&self) -> bool {
        self.coef.is_empty()
    }

    pub fn is_neg(&self) -> bool {
        self.neg
    }

    /// The power of ten of its first digit (123 → 2, 0.05 → -2).
    fn magnitude(&self) -> i64 {
        self.exp + self.coef.len() as i64 - 1
    }

    /// Rounded to `digits` significant digits, half to even.
    pub fn round(self, digits: usize) -> Dec {
        if self.coef.len() <= digits {
            return self;
        }
        let drop = self.coef.len() - digits;
        let first = self.coef[drop - 1];
        let rest_zero = self.coef[..drop - 1].iter().all(|d| *d == 0);
        let mut kept = self.coef[drop..].to_vec();
        let up = first > 5 || (first == 5 && (!rest_zero || kept[0] % 2 == 1));
        if up {
            kept = add_mag(&kept, &[1]);
        }
        Dec { neg: self.neg, coef: kept, exp: self.exp + drop as i64 }.norm()
    }

    fn check(self) -> Result<Dec> {
        if !self.is_zero() && self.magnitude() > MAX_EXP {
            return Err(CalcError::TooLarge);
        }
        // Too small to tell from zero.
        if !self.is_zero() && self.magnitude() < -MAX_EXP {
            return Ok(Dec::zero());
        }
        Ok(self)
    }

    pub fn neg(&self) -> Dec {
        Dec { neg: !self.neg, ..self.clone() }.norm()
    }

    pub fn abs(&self) -> Dec {
        Dec { neg: false, ..self.clone() }
    }

    fn add_p(&self, b: &Dec, p: usize) -> Dec {
        if self.is_zero() {
            return b.clone().round(p);
        }
        if b.is_zero() {
            return self.clone().round(p);
        }
        // A number too small to reach the other's last kept digit changes nothing but the rounding.
        let gap = (self.magnitude() - b.magnitude()).abs();
        if gap > p as i64 + 3 {
            let (big, small) = if self.magnitude() > b.magnitude() { (self, b) } else { (b, self) };
            let nudge = Dec { neg: small.neg, coef: vec![1], exp: big.magnitude() - p as i64 - 2 };
            return big.add_p(&nudge, p);
        }
        let exp = self.exp.min(b.exp);
        let a = shift(&self.coef, (self.exp - exp) as usize);
        let c = shift(&b.coef, (b.exp - exp) as usize);
        let out = if self.neg == b.neg {
            Dec { neg: self.neg, coef: add_mag(&a, &c), exp }
        } else {
            match cmp_mag(&a, &c) {
                Ordering::Equal => Dec::zero(),
                Ordering::Greater => Dec { neg: self.neg, coef: sub_mag(&a, &c), exp },
                Ordering::Less => Dec { neg: b.neg, coef: sub_mag(&c, &a), exp },
            }
        };
        out.norm().round(p)
    }

    fn mul_p(&self, b: &Dec, p: usize) -> Dec {
        Dec { neg: self.neg != b.neg, coef: mul_mag(&self.coef, &b.coef), exp: self.exp + b.exp }.norm().round(p)
    }

    fn div_p(&self, b: &Dec, p: usize) -> Result<Dec> {
        if b.is_zero() {
            return Err(CalcError::DivideByZero);
        }
        if self.is_zero() {
            return Ok(Dec::zero());
        }
        let s = (p + 2 + b.coef.len()).saturating_sub(self.coef.len()) + 1;
        let (mut q, r) = divmod_mag(&shift(&self.coef, s), &b.coef);
        let mut exp = self.exp - b.exp - s as i64;
        // A remainder: a digit past the quotient's, so rounding never sees a false tie.
        if !r.is_empty() {
            q = shift(&q, 1);
            q[0] = 1;
            exp -= 1;
        }
        Ok(Dec { neg: self.neg != b.neg, coef: q, exp }.norm().round(p))
    }

    pub fn add(&self, b: &Dec) -> Result<Dec> {
        self.add_p(b, DIGITS).check()
    }

    pub fn sub(&self, b: &Dec) -> Result<Dec> {
        self.add_p(&b.neg(), DIGITS).check()
    }

    pub fn mul(&self, b: &Dec) -> Result<Dec> {
        self.mul_p(b, DIGITS).check()
    }

    pub fn div(&self, b: &Dec) -> Result<Dec> {
        self.div_p(b, DIGITS)?.check()
    }

    pub fn cmp(&self, b: &Dec) -> Ordering {
        let d = self.add_p(&b.neg(), WORK);
        if d.is_zero() {
            Ordering::Equal
        } else if d.neg {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }

    pub fn is_integer(&self) -> bool {
        self.exp >= 0
    }

    /// Its value as a whole number, if it is one that fits.
    pub fn to_i64(&self) -> Option<i64> {
        if !self.is_integer() || self.magnitude() > 17 {
            return None;
        }
        let mut n: i64 = 0;
        for d in self.coef.iter().rev() {
            n = n * 10 + *d as i64;
        }
        n = n.checked_mul(10i64.checked_pow(self.exp as u32)?)?;
        Some(if self.neg { -n } else { n })
    }

    /// The nearest whole number (half away from zero).
    fn round_whole(&self) -> Dec {
        if self.is_integer() {
            return self.clone();
        }
        let mut r = self.abs().add_p(&Dec { neg: false, coef: vec![5], exp: -1 }, WORK + 40);
        // Truncate to the whole part.
        if r.exp < 0 {
            let cut = (-r.exp) as usize;
            if cut >= r.coef.len() {
                r = Dec::zero();
            } else {
                r.coef.drain(..cut);
                r.exp = 0;
                r = r.norm();
            }
        }
        if self.neg { r.neg() } else { r }
    }

    /// An approximation, for first guesses.
    fn to_f64(&self) -> f64 {
        let s = self.to_plain(20);
        s.parse().unwrap_or(0.0)
    }

    fn from_f64(x: f64) -> Dec {
        Dec::parse(&format!("{x:e}")).unwrap_or_else(Dec::zero)
    }

    /// Written in full ("12.5", "0.0003", "-7"), at most `digits` significant.
    fn to_plain(&self, digits: usize) -> String {
        let d = self.clone().round(digits);
        if d.is_zero() {
            return "0".into();
        }
        let s: String = d.coef.iter().rev().map(|c| (b'0' + c) as char).collect();
        let mut out = String::new();
        if d.neg {
            out.push('-');
        }
        if d.exp >= 0 {
            out.push_str(&s);
            out.extend(std::iter::repeat_n('0', d.exp as usize));
        } else {
            let point = s.len() as i64 + d.exp;
            if point > 0 {
                out.push_str(&s[..point as usize]);
                out.push('.');
                out.push_str(&s[point as usize..]);
            } else {
                out.push_str("0.");
                out.extend(std::iter::repeat_n('0', (-point) as usize));
                out.push_str(&s);
            }
        }
        out
    }

    /// As the display shows it: at most `digits` significant digits, in
    /// full while that is readable, else as "1.2345e+25".
    pub fn show(&self, digits: usize) -> String {
        let d = self.clone().round(digits);
        if d.is_zero() {
            return "0".into();
        }
        let m = d.magnitude();
        if (-9..digits as i64).contains(&m) {
            return d.to_plain(digits);
        }
        let s: String = d.coef.iter().rev().map(|c| (b'0' + c) as char).collect();
        let mut out = String::new();
        if d.neg {
            out.push('-');
        }
        out.push_str(&s[..1]);
        if s.len() > 1 {
            out.push('.');
            out.push_str(&s[1..]);
        }
        out.push_str(&format!("e{}{}", if m < 0 { "-" } else { "+" }, m.abs()));
        out
    }

    /// Shown with exactly `places` digits after the point (rounded).
    pub fn show_fixed(&self, places: usize) -> String {
        if !self.is_zero() && self.magnitude() >= DIGITS as i64 {
            return self.show(DIGITS);
        }
        let digits = (self.magnitude() + 1 + places as i64).max(0) as usize;
        let d = if digits == 0 { Dec::zero() } else { self.clone().round(digits) };
        let s = d.to_plain(DIGITS + places);
        let (int, frac) = match s.split_once('.') {
            Some((i, f)) => (i.to_string(), f.to_string()),
            None => (s, String::new()),
        };
        if places == 0 {
            return int;
        }
        let mut frac = frac;
        frac.truncate(places);
        while frac.len() < places {
            frac.push('0');
        }
        format!("{int}.{frac}")
    }
}

impl PartialEq for Dec {
    fn eq(&self, other: &Dec) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

// ---- constants, to more digits than WORK ----

const PI: &str = "3.14159265358979323846264338327950288419716939937510582097494459";
const LN10: &str = "2.30258509299404568401799145468436420760110148862877297603332790";

pub fn pi() -> Dec {
    Dec::parse_work(PI)
}

pub fn e() -> Dec {
    exp_work(&Dec::from_i64(1)).round(DIGITS)
}

impl Dec {
    /// Parsed at working precision (constants).
    fn parse_work(s: &str) -> Dec {
        let (int, frac) = s.split_once('.').unwrap_or((s, ""));
        let mut digits: Vec<u8> = int.bytes().chain(frac.bytes()).map(|b| b - b'0').collect();
        digits.reverse();
        Dec { neg: false, coef: digits, exp: -(frac.len() as i64) }.norm().round(WORK)
    }
}

fn one() -> Dec {
    Dec::from_i64(1)
}

fn int(n: i64) -> Dec {
    Dec::from_i64(n)
}

fn half() -> Dec {
    Dec { neg: false, coef: vec![5], exp: -1 }
}

/// A term as small as this no longer changes a sum at working precision.
fn negligible(term: &Dec, sum: &Dec) -> bool {
    term.is_zero() || (!sum.is_zero() && term.magnitude() < sum.magnitude() - WORK as i64 - 2)
}

// ---- the functions, at working precision ----

/// e^x.
fn exp_work(x: &Dec) -> Dec {
    if x.is_zero() {
        return one();
    }
    // Halve until small, then the series, then square back.
    let mut r = x.clone();
    let mut halvings = 0;
    while r.magnitude() >= -1 {
        r = r.mul_p(&half(), WORK);
        halvings += 1;
    }
    let mut sum = one();
    let mut term = one();
    for n in 1..400 {
        term = term.mul_p(&r, WORK).div_p(&int(n), WORK).unwrap();
        if negligible(&term, &sum) {
            break;
        }
        sum = sum.add_p(&term, WORK);
    }
    for _ in 0..halvings {
        sum = sum.mul_p(&sum, WORK);
    }
    sum
}

/// ln x, x positive.
fn ln_work(x: &Dec) -> Dec {
    // x = m × 10^k with m in [1, 10): ln x = ln m + k ln 10.
    let k = x.magnitude();
    let mut m = Dec { neg: false, coef: x.coef.clone(), exp: -(x.coef.len() as i64 - 1) };
    // Square roots bring m near 1: ln m = 2^j ln(m^(1/2^j)).
    let mut j = 0;
    while m.add_p(&one().neg(), WORK).abs().cmp(&Dec { neg: false, coef: vec![1], exp: -2 }) == Ordering::Greater {
        m = sqrt_work(&m);
        j += 1;
    }
    // ln m = 2 atanh z, z = (m - 1)/(m + 1).
    let z = m.add_p(&one().neg(), WORK).div_p(&m.add_p(&one(), WORK), WORK).unwrap();
    let z2 = z.mul_p(&z, WORK);
    let mut sum = z.clone();
    let mut power = z.clone();
    for n in (3..2000).step_by(2) {
        power = power.mul_p(&z2, WORK);
        let term = power.div_p(&int(n), WORK).unwrap();
        if negligible(&term, &sum) {
            break;
        }
        sum = sum.add_p(&term, WORK);
    }
    let mut ln_m = sum.mul_p(&int(2), WORK);
    for _ in 0..j {
        ln_m = ln_m.mul_p(&int(2), WORK);
    }
    ln_m.add_p(&Dec::parse_work(LN10).mul_p(&int(k), WORK), WORK)
}

/// The square root, x not negative.
fn sqrt_work(x: &Dec) -> Dec {
    if x.is_zero() {
        return Dec::zero();
    }
    // A first guess from its digits, then Newton's steps.
    let k = x.magnitude();
    let even = if k % 2 == 0 { k } else { k - 1 };
    let m = Dec { exp: x.exp - even, ..x.clone() };
    let mut y = Dec::from_f64(m.to_f64().sqrt());
    y.exp += even / 2;
    for _ in 0..60 {
        let next = y.add_p(&x.div_p(&y, WORK).unwrap(), WORK).mul_p(&half(), WORK);
        if next.cmp(&y) == Ordering::Equal {
            break;
        }
        y = next;
    }
    y
}

/// sin of r and cos of r, |r| at most π/4 (or so).
fn sin_cos_small(r: &Dec) -> (Dec, Dec) {
    let r2 = r.mul_p(r, WORK);
    let (mut s, mut c) = (r.clone(), one());
    let (mut ts, mut tc) = (r.clone(), one());
    for n in 1..200i64 {
        ts = ts.mul_p(&r2, WORK).div_p(&int((2 * n) * (2 * n + 1)), WORK).unwrap().neg();
        tc = tc.mul_p(&r2, WORK).div_p(&int((2 * n - 1) * (2 * n)), WORK).unwrap().neg();
        let done = negligible(&ts, &s) && negligible(&tc, &c);
        s = s.add_p(&ts, WORK);
        c = c.add_p(&tc, WORK);
        if done {
            break;
        }
    }
    (s, c)
}

/// An angle as a number of right angles and what is left (in radians):
/// in degrees the split is exact, so 180° leaves exactly nothing.
fn quadrant(x: &Dec, degrees: bool) -> Result<(i64, Dec)> {
    if !x.is_zero() && x.magnitude() > 20 {
        return Err(CalcError::TooLarge);
    }
    if degrees {
        let n = x.div_p(&int(90), WORK)?.round_whole();
        let rest = x.add_p(&n.mul_p(&int(90), WORK).neg(), WORK);
        let r = rest.mul_p(&pi(), WORK).div_p(&int(180), WORK)?;
        return Ok((n.to_i64().unwrap_or(0), r));
    }
    let half_pi = pi().mul_p(&half(), WORK);
    let n = x.div_p(&half_pi, WORK)?.round_whole();
    let r = x.add_p(&n.mul_p(&half_pi, WORK).neg(), WORK);
    Ok((n.to_i64().unwrap_or(0), r))
}

/// Results smaller than this are zero (sin π in radians).
fn snap(x: Dec) -> Dec {
    if !x.is_zero() && x.magnitude() < -(DIGITS as i64) - 4 { Dec::zero() } else { x }
}

pub fn sqrt(x: &Dec) -> Result<Dec> {
    if x.is_neg() {
        return Err(CalcError::Domain);
    }
    sqrt_work(x).round(DIGITS).check()
}

pub fn exp(x: &Dec) -> Result<Dec> {
    if !x.is_zero() && x.cmp(&int(230_000)) == Ordering::Greater {
        return Err(CalcError::TooLarge);
    }
    if x.cmp(&int(-230_000)) == Ordering::Less {
        return Ok(Dec::zero());
    }
    exp_work(x).round(DIGITS).check()
}

pub fn ln(x: &Dec) -> Result<Dec> {
    if x.is_neg() || x.is_zero() {
        return Err(CalcError::Domain);
    }
    snap(ln_work(x)).round(DIGITS).check()
}

pub fn log10(x: &Dec) -> Result<Dec> {
    if x.is_neg() || x.is_zero() {
        return Err(CalcError::Domain);
    }
    // A power of ten is exact.
    if x.coef == [1] {
        return Ok(int(x.exp));
    }
    ln_work(x).div_p(&Dec::parse_work(LN10), WORK)?.round(DIGITS).check()
}

/// x to the power y.
pub fn pow(x: &Dec, y: &Dec) -> Result<Dec> {
    if y.is_zero() {
        return Ok(one());
    }
    if x.is_zero() {
        return if y.is_neg() { Err(CalcError::DivideByZero) } else { Ok(Dec::zero()) };
    }
    if let Some(n) = y.to_i64().filter(|n| n.abs() <= 1_000_000) {
        let mut base = x.clone();
        let mut acc = one();
        let mut e = n.unsigned_abs();
        while e > 0 {
            if e & 1 == 1 {
                acc = acc.mul_p(&base, WORK);
            }
            base = base.mul_p(&base, WORK);
            e >>= 1;
            if !acc.is_zero() && acc.magnitude().abs() > MAX_EXP * 2 {
                return Err(CalcError::TooLarge);
            }
        }
        let r = if n < 0 { one().div_p(&acc, WORK)? } else { acc };
        return r.round(DIGITS).check();
    }
    if x.is_neg() {
        return Err(CalcError::Domain);
    }
    let l = ln_work(x).mul_p(y, WORK);
    if l.cmp(&int(230_000)) == Ordering::Greater {
        return Err(CalcError::TooLarge);
    }
    exp_work(&l).round(DIGITS).check()
}

pub fn sin(x: &Dec, degrees: bool) -> Result<Dec> {
    let (n, r) = quadrant(x, degrees)?;
    let (s, c) = sin_cos_small(&r);
    let v = match n.rem_euclid(4) {
        0 => s,
        1 => c,
        2 => s.neg(),
        _ => c.neg(),
    };
    snap(v).round(DIGITS).check()
}

pub fn cos(x: &Dec, degrees: bool) -> Result<Dec> {
    let (n, r) = quadrant(x, degrees)?;
    let (s, c) = sin_cos_small(&r);
    let v = match n.rem_euclid(4) {
        0 => c,
        1 => s.neg(),
        2 => c.neg(),
        _ => s,
    };
    snap(v).round(DIGITS).check()
}

pub fn tan(x: &Dec, degrees: bool) -> Result<Dec> {
    let (n, r) = quadrant(x, degrees)?;
    let (s, c) = sin_cos_small(&r);
    let (num, den) = if n.rem_euclid(2) == 0 { (s, c) } else { (c, s.neg()) };
    if snap(den.clone()).is_zero() {
        return Err(CalcError::Domain);
    }
    snap(num.div_p(&den, WORK)?).round(DIGITS).check()
}

/// atan at working precision, in radians.
fn atan_work(x: &Dec) -> Dec {
    if x.is_neg() {
        return atan_work(&x.neg()).neg();
    }
    if x.cmp(&one()) == Ordering::Greater {
        // atan x = π/2 - atan(1/x).
        let inv = one().div_p(x, WORK).unwrap();
        return pi().mul_p(&half(), WORK).add_p(&atan_work(&inv).neg(), WORK);
    }
    // Halve the angle until small: atan x = 2 atan(x / (1 + √(1 + x²))).
    let mut z = x.clone();
    let mut doublings = 0;
    while z.cmp(&Dec { neg: false, coef: vec![1], exp: -1 }) == Ordering::Greater {
        let root = sqrt_work(&one().add_p(&z.mul_p(&z, WORK), WORK));
        z = z.div_p(&one().add_p(&root, WORK), WORK).unwrap();
        doublings += 1;
    }
    let z2 = z.mul_p(&z, WORK);
    let mut sum = z.clone();
    let mut power = z.clone();
    for n in (3..4000).step_by(2) {
        power = power.mul_p(&z2, WORK).neg();
        let term = power.div_p(&int(n), WORK).unwrap();
        if negligible(&term, &sum) {
            break;
        }
        sum = sum.add_p(&term, WORK);
    }
    for _ in 0..doublings {
        sum = sum.mul_p(&int(2), WORK);
    }
    sum
}

/// Radians into what is shown (degrees, if asked).
fn angle_out(r: Dec, degrees: bool) -> Result<Dec> {
    let v = if degrees { r.mul_p(&int(180), WORK).div_p(&pi(), WORK)? } else { r };
    snap(v).round(DIGITS).check()
}

pub fn atan(x: &Dec, degrees: bool) -> Result<Dec> {
    angle_out(atan_work(x), degrees)
}

pub fn asin(x: &Dec, degrees: bool) -> Result<Dec> {
    match x.abs().cmp(&one()) {
        Ordering::Greater => Err(CalcError::Domain),
        Ordering::Equal => angle_out(if x.is_neg() { pi().mul_p(&half(), WORK).neg() } else { pi().mul_p(&half(), WORK) }, degrees),
        Ordering::Less => {
            let c = sqrt_work(&one().add_p(&x.mul_p(x, WORK).neg(), WORK));
            angle_out(atan_work(&x.div_p(&c, WORK)?), degrees)
        }
    }
}

pub fn acos(x: &Dec, degrees: bool) -> Result<Dec> {
    if x.abs().cmp(&one()) == Ordering::Greater {
        return Err(CalcError::Domain);
    }
    let a = match x.abs().cmp(&one()) {
        Ordering::Equal => {
            if x.is_neg() {
                pi()
            } else {
                Dec::zero()
            }
        }
        _ => {
            let c = sqrt_work(&one().add_p(&x.mul_p(x, WORK).neg(), WORK));
            pi().mul_p(&half(), WORK).add_p(&atan_work(&x.div_p(&c, WORK)?).neg(), WORK)
        }
    };
    angle_out(a, degrees)
}

/// n!, for whole numbers from 0 to 3000.
pub fn factorial(x: &Dec) -> Result<Dec> {
    let n = x.to_i64().filter(|n| (0..=3000).contains(n)).ok_or(if x.is_integer() && !x.is_neg() { CalcError::TooLarge } else { CalcError::Domain })?;
    let mut acc = one();
    for k in 2..=n {
        acc = acc.mul_p(&int(k), WORK);
    }
    acc.round(DIGITS).check()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Dec {
        Dec::parse(s).unwrap()
    }

    fn show(x: Result<Dec>) -> String {
        x.map(|v| v.show(16)).unwrap_or_else(|e| format!("{e:?}"))
    }

    #[test]
    fn decimals_are_exact() {
        assert_eq!(show(d("0.1").add(&d("0.2"))), "0.3");
        assert_eq!(show(d("1").sub(&d("0.9"))), "0.1");
        assert_eq!(show(d("1.1").mul(&d("1.1"))), "1.21");
        assert_eq!(show(d("1").div(&d("3")).and_then(|t| t.mul(&int(3)))), "1");
        assert_eq!(show(d("10").div(&d("4"))), "2.5");
        assert_eq!(show(d("1").div(&d("0"))), "DivideByZero");
        assert_eq!(d("2.50"), d("2.5"));
        assert_eq!(show(d("-3").add(&d("3"))), "0");
    }

    #[test]
    fn rounding_is_half_to_even_at_34_digits() {
        let third = d("1").div(&d("3")).unwrap();
        assert_eq!(third.show(40), "0.3333333333333333333333333333333333");
        let two_thirds = d("2").div(&d("3")).unwrap();
        assert_eq!(two_thirds.show(40), "0.6666666666666666666666666666666667");
        assert_eq!(d("0.125").round(2).show(10), "0.12");
        assert_eq!(d("0.135").round(2).show(10), "0.14");
    }

    #[test]
    fn showing_numbers() {
        assert_eq!(d("1234567").show(16), "1234567");
        assert_eq!(d("0.000012").show(16), "0.000012");
        assert_eq!(d("1e25").show(16), "1e+25");
        assert_eq!(d("-1.5e-12").show(16), "-1.5e-12");
        assert_eq!(d("2").div(&d("3")).unwrap().show(10), "0.6666666667");
        assert_eq!(d("3.14159").show_fixed(2), "3.14");
        assert_eq!(d("2").show_fixed(3), "2.000");
        assert_eq!(d("-0.005").show_fixed(2), "0.00");
        assert_eq!(Dec::parse("1..2"), None);
        assert_eq!(Dec::parse("abc"), None);
    }

    #[test]
    fn roots_powers_and_logarithms() {
        assert_eq!(show(sqrt(&d("2"))), "1.414213562373095");
        assert_eq!(show(sqrt(&d("144"))), "12");
        assert_eq!(show(sqrt(&d("-1"))), "Domain");
        assert_eq!(show(pow(&d("2"), &d("10"))), "1024");
        assert_eq!(show(pow(&d("2"), &d("-2"))), "0.25");
        assert_eq!(show(pow(&d("2"), &d("0.5"))), "1.414213562373095");
        assert_eq!(show(pow(&d("-8"), &d("0.5"))), "Domain");
        assert_eq!(show(ln(&e())), "1");
        assert_eq!(show(ln(&d("1"))), "0");
        assert_eq!(show(log10(&d("1000"))), "3");
        assert_eq!(show(log10(&d("2"))), "0.3010299956639812");
        assert_eq!(show(exp(&d("1"))), "2.718281828459045");
        assert_eq!(show(exp(&ln(&d("7")).unwrap())), "7");
        assert_eq!(factorial(&d("20")).unwrap().show(24), "2432902008176640000");
        assert_eq!(show(factorial(&d("2.5"))), "Domain");
    }

    #[test]
    fn trigonometry_is_exact_where_it_should_be() {
        assert_eq!(show(sin(&d("30"), true)), "0.5");
        assert_eq!(show(sin(&d("180"), true)), "0");
        assert_eq!(show(cos(&d("60"), true)), "0.5");
        assert_eq!(show(cos(&d("90"), true)), "0");
        assert_eq!(show(tan(&d("45"), true)), "1");
        assert_eq!(show(tan(&d("90"), true)), "Domain");
        assert_eq!(show(sin(&pi(), false)), "0");
        assert_eq!(show(sin(&d("1"), false)), "0.8414709848078965");
        assert_eq!(show(asin(&d("0.5"), true)), "30");
        assert_eq!(show(acos(&d("0"), true)), "90");
        assert_eq!(show(atan(&d("1"), true)), "45");
        assert_eq!(show(atan(&d("1"), false)), "0.7853981633974483");
        assert_eq!(show(asin(&d("2"), true)), "Domain");
        assert_eq!(show(sin(&d("-390"), true)), "-0.5");
    }

    #[test]
    fn huge_and_tiny_are_said_not_wrapped() {
        assert_eq!(show(pow(&d("10"), &d("200000"))), "TooLarge");
        assert_eq!(show(exp(&d("-1000000"))), "0");
        assert_eq!(show(d("1e-90").mul(&d("1e-90"))), "1e-180");
        assert!(d("9.99e99999").mul(&d("100")).is_err());
    }
}
