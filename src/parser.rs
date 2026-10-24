#![allow(dead_code)]
//! Recursive-descent expression parser / evaluator.
//!
//! Grammar
//! -------
//!   expr    → term   ( ('+' | '-')  term   )*
//!   term    → power  ( ('*' | '/' | '%') power
//!                    | power )*        ← implicit multiplication (2π, 3(4+1))
//!   power   → unary  ( '^' power )?          right-associative
//!   unary   → '-'? postfix                     (-2^2 = -4, -3! = -6)
//!   postfix → primary ( '!' )*                 factorial binds tightest
//!   primary → NUMBER | '(' expr ')' | IDENT | IDENT '(' expr ')'
//!
//! Pretty symbols accepted: × ÷ − (U+2212) √ π

use std::f64::consts::{E, PI};

// ── Tokens ────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Plus, Minus, Star, Slash, Percent, Caret, Bang,
    LParen, RParen,
    Ident(String),
    End,
}

// ── Tokeniser ─────────────────────────────────────────────────────────────
fn tokenise(input: &str) -> Result<Vec<Tok>, String> {
    let mut toks  = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&ch) = chars.peek() {
        match ch {
            ' ' | '\t' => { chars.next(); }

            '0'..='9' | '.' => {
                let mut s = String::new();
                let mut last_was_e = false;
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() || c == '.' {
                        s.push(c); chars.next(); last_was_e = false;
                    } else if (c == 'e' || c == 'E') && exp_continues(&chars) {
                        // 'e' starts an exponent ONLY when digits actually
                        // follow — otherwise it is Euler's constant ("2e").
                        s.push(c); chars.next(); last_was_e = true;
                    } else if last_was_e && (c == '+' || c == '-') {
                        s.push(c); chars.next(); last_was_e = false;
                    } else { break; }
                }
                toks.push(Tok::Num(s.parse().map_err(|_| format!("Bad number: {s}"))?));
            }

            '√' => { toks.push(Tok::Ident("sqrt".into())); chars.next(); }

            'a'..='z' | 'A'..='Z' | '_' | 'π' => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_alphanumeric() || c == '_' || c == 'π' {
                        s.push(c); chars.next();
                    } else { break; }
                }
                toks.push(Tok::Ident(s));
            }

            '+'             => { toks.push(Tok::Plus);    chars.next(); }
            '-' | '\u{2212}' => { toks.push(Tok::Minus);   chars.next(); } // ASCII + U+2212
            '*' | '×'       => { toks.push(Tok::Star);    chars.next(); }
            '/' | '÷'       => { toks.push(Tok::Slash);   chars.next(); }
            '%'             => { toks.push(Tok::Percent); chars.next(); }
            '^'             => { toks.push(Tok::Caret);   chars.next(); }
            '!'             => { toks.push(Tok::Bang);    chars.next(); }
            '('             => { toks.push(Tok::LParen);  chars.next(); }
            ')'             => { toks.push(Tok::RParen);  chars.next(); }
            c   => return Err(format!("Unknown character '{c}'")),
        }
    }
    toks.push(Tok::End);
    Ok(toks)
}

/// True when the peekable (positioned ON an 'e'/'E') is followed by an
/// exponent continuation: a digit, or a sign followed by a digit.
fn exp_continues<I: Iterator<Item = char> + Clone>(it: &std::iter::Peekable<I>) -> bool {
    let mut rest = it.clone();
    rest.next();                            // consume the 'e'/'E' itself
    match rest.next() {
        Some(c) if c.is_ascii_digit() => true,
        Some('+' | '-') => matches!(rest.next(), Some(d) if d.is_ascii_digit()),
        _ => false,
    }
}

// ── Parser ────────────────────────────────────────────────────────────────
struct Parser<'a> { toks: &'a [Tok], pos: usize, deg: bool }

impl<'a> Parser<'a> {
    fn peek(&self) -> &Tok { &self.toks[self.pos] }
    fn bump(&mut self) -> &Tok {
        let t = &self.toks[self.pos];
        if self.pos + 1 < self.toks.len() { self.pos += 1; }
        t
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t { self.bump(); true } else { false }
    }
}

fn parse_expr(p: &mut Parser<'_>) -> Result<f64, String> {
    let mut v = parse_term(p)?;
    loop {
        match p.peek() {
            Tok::Plus  => { p.bump(); v += parse_term(p)?; }
            Tok::Minus => { p.bump(); v -= parse_term(p)?; }
            _          => break,
        }
    }
    Ok(v)
}

fn parse_term(p: &mut Parser<'_>) -> Result<f64, String> {
    let mut v = parse_unary(p)?;
    loop {
        match p.peek() {
            Tok::Star    => { p.bump(); v *= parse_unary(p)?; }
            Tok::Slash   => {
                p.bump();
                let d = parse_unary(p)?;
                if d == 0.0 { return Err("Division by zero".into()); }
                v /= d;
            }
            Tok::Percent => { p.bump(); v %= parse_unary(p)?; }
            // Implicit multiplication: 2π, 3(4+1), 2sin(30), (2)(3)
            Tok::LParen | Tok::Ident(_) => { v *= parse_unary(p)?; }
            _            => break,
        }
    }
    Ok(v)
}

fn parse_power(p: &mut Parser<'_>) -> Result<f64, String> {
    let base = parse_postfix(p)?;
    if let Tok::Caret = p.peek() {
        p.bump();
        Ok(base.powf(parse_unary(p)?))  // right-assoc; RHS may be signed (2^-3)
    } else { Ok(base) }
}

fn parse_unary(p: &mut Parser<'_>) -> Result<f64, String> {
    if matches!(p.peek(), Tok::Minus) {
        p.bump();
        // Unary minus binds looser than '^': -2^2 evaluates to -(2^2) = -4.
        Ok(-parse_unary(p)?)
    } else {
        parse_power(p)
    }
}

/// Postfix factorial — binds tighter than '^' (2^3! = 2^6) and looser
/// than primary. Chainable: 5!! = (5!)! (domain-checked, so 120! errors).
fn parse_postfix(p: &mut Parser<'_>) -> Result<f64, String> {
    let mut v = parse_primary(p)?;
    while matches!(p.peek(), Tok::Bang) {
        p.bump();
        v = factorial_f64(v)?;
    }
    Ok(v)
}

fn parse_primary(p: &mut Parser<'_>) -> Result<f64, String> {
    match p.peek().clone() {
        Tok::Num(n) => { p.bump(); Ok(n) }

        Tok::LParen => {
            p.bump();
            let v = parse_expr(p)?;
            if !p.eat(&Tok::RParen) { return Err("Expected ')'".into()); }
            Ok(v)
        }

        Tok::Ident(name) => {
            p.bump();
            // Constants
            match name.as_str() {
                "pi" | "π" => return Ok(PI),
                "e"        => return Ok(E),
                _          => {}
            }
            // Functions
            if p.eat(&Tok::LParen) {
                let arg = parse_expr(p)?;
                if !p.eat(&Tok::RParen) {
                    return Err(format!("Expected ')' after {name}(...)"));
                }
                apply_fn(&name, arg, p.deg)
            } else {
                Err(format!("Unknown identifier '{name}'"))
            }
        }

        other => Err(format!("Unexpected token: {other:?}")),
    }
}

fn apply_fn(name: &str, x: f64, deg: bool) -> Result<f64, String> {
    let to_rad   = if deg { PI / 180.0 } else { 1.0 };
    let from_rad = if deg { 180.0 / PI } else { 1.0 };
    match name {
        "sin"   => Ok((x * to_rad).sin()),
        "cos"   => Ok((x * to_rad).cos()),
        "tan"   => Ok((x * to_rad).tan()),
        "asin"  => { if x.abs() > 1.0 { Err("asin: domain error".into()) }
                     else { Ok(x.asin() * from_rad) } }
        "acos"  => { if x.abs() > 1.0 { Err("acos: domain error".into()) }
                     else { Ok(x.acos() * from_rad) } }
        "atan"  => Ok(x.atan() * from_rad),
        "log"   => { if x <= 0.0 { Err("log: domain error".into()) }
                     else { Ok(x.log10()) } }
        "ln"    => { if x <= 0.0 { Err("ln: domain error".into()) }
                     else { Ok(x.ln()) } }
        "exp"   => Ok(x.exp()),
        "sqrt"  => { if x < 0.0 { Err("sqrt: negative".into()) }
                     else { Ok(x.sqrt()) } }
        "cbrt"  => Ok(x.cbrt()),
        "abs"   => Ok(x.abs()),
        "floor" => Ok(x.floor()),
        "ceil"  => Ok(x.ceil()),
        "round" => Ok(x.round()),
        "fact"  => factorial_f64(x),
        _ => Err(format!("Unknown function '{name}'")),
    }
}

/// f64 factorial with domain check — accepts integers 0..=170.
/// u64/saturating arithmetic silently clamped every n ≥ 21 to u64::MAX
/// while still advertising n ≤ 170, so this runs fully in f64.
fn factorial_f64(v: f64) -> Result<f64, String> {
    if v < 0.0 || (v - v.floor()).abs() > 1e-9 || v > 170.0 {
        return Err("!: domain error (0..=170)".into());
    }
    Ok(factorial(v as u64))
}

/// f64 factorial — u64/saturating arithmetic silently clamped every n ≥ 21
/// to u64::MAX while still advertising n ≤ 170.
fn factorial(n: u64) -> f64 {
    let mut r = 1.0;
    for i in 2..=n { r *= i as f64; }
    r
}

// ── Public API ────────────────────────────────────────────────────────────

/// Evaluate a mathematical expression. `degrees` controls trig units.
pub fn evaluate(expr: &str, degrees: bool) -> Result<f64, String> {
    let toks = tokenise(expr)?;
    let mut p = Parser { toks: &toks, pos: 0, deg: degrees };
    let val = parse_expr(&mut p)?;
    if *p.peek() != Tok::End {
        return Err(format!("Unexpected input at position {}", p.pos));
    }
    Ok(val)
}

/// Format an f64 nicely for display (strips trailing zeros, uses sci notation
/// for very large or very small values).
pub fn fmt_num(n: f64) -> String {
    if n.is_nan()      { return "Error".into(); }
    if n.is_infinite() { return if n > 0.0 { "∞".into() } else { "-∞".into() }; }
    if n == 0.0        { return "0".into(); }

    let abs = n.abs();
    if abs >= 1e15 || (abs < 1e-9 && abs > 0.0) {
        return format!("{:.6e}", n);
    }
    if n.fract().abs() < 1e-10 && abs < 1e15 {
        return format!("{:.0}", n);
    }
    let s = format!("{:.10}", n);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        assert_eq!(evaluate("2+3",    false).unwrap(), 5.0);
        assert_eq!(evaluate("10-4",   false).unwrap(), 6.0);
        assert_eq!(evaluate("3*4",    false).unwrap(), 12.0);
        assert_eq!(evaluate("8/2",    false).unwrap(), 4.0);
        assert_eq!(evaluate("10%3",   false).unwrap(), 1.0);
    }

    #[test]
    fn precedence_and_parens() {
        let v = evaluate("2+3*4", false).unwrap();
        assert!((v - 14.0).abs() < 1e-10, "Expected 14, got {v}");
        let v = evaluate("(2+3)*4", false).unwrap();
        assert!((v - 20.0).abs() < 1e-10);
    }

    #[test]
    fn power_right_assoc() {
        let v = evaluate("2^3^2", false).unwrap(); // 2^(3^2) = 2^9 = 512
        assert!((v - 512.0).abs() < 1e-10, "Expected 512, got {v}");
    }

    #[test]
    fn unary_minus_binds_looser_than_power() {
        // Was parsed as (-2)^2 = 4; standard convention is -(2^2) = -4.
        let v = evaluate("-2^2", false).unwrap();
        assert!((v + 4.0).abs() < 1e-10, "Expected -4, got {v}");
    }

    #[test]
    fn negative_exponent() {
        let v = evaluate("2^-3", false).unwrap();
        assert!((v - 0.125).abs() < 1e-12, "Expected 0.125, got {v}");
    }

    #[test]
    fn division_by_zero_is_error() {
        assert!(evaluate("1/0", false).is_err());
        assert!(evaluate("4/0/2", false).is_err());
    }

    #[test]
    fn large_factorial_not_saturated() {
        // 25! ≈ 1.5511210043330986e25 — the u64 version clamped this.
        let v = evaluate("fact(25)", false).unwrap();
        let expect = 1.551_121_004_333_098_6e25;
        assert!((v - expect).abs() / expect < 1e-12, "fact(25) = {v}");
    }

    #[test]
    fn fact_at_170_is_finite() {
        let v = evaluate("fact(170)", false).unwrap();
        assert!(v.is_finite() && v > 7e306, "fact(170) = {v}");
    }

    #[test]
    fn trig_degrees() {
        let s = evaluate("sin(90)", true).unwrap();
        assert!((s - 1.0).abs() < 1e-10, "sin(90°) = {s}");
        let c = evaluate("cos(0)", true).unwrap();
        assert!((c - 1.0).abs() < 1e-10, "cos(0°) = {c}");
    }

    #[test]
    fn constants() {
        let p = evaluate("pi", false).unwrap();
        assert!((p - PI).abs() < 1e-10);
        let e = evaluate("e", false).unwrap();
        assert!((e - E).abs() < 1e-10);
    }

    #[test]
    fn factorial_val() {
        let f = evaluate("fact(5)", false).unwrap();
        assert!((f - 120.0).abs() < 1e-10);
    }

    // ── Postfix factorial ────────────────────────────────────────────────
    #[test]
    fn postfix_factorial() {
        assert!((evaluate("5!",  false).unwrap() - 120.0).abs() < 1e-10);
        assert!((evaluate("0!",  false).unwrap() - 1.0).abs()   < 1e-10);
        // Tighter than '^' on both sides:
        assert!((evaluate("2^3!", false).unwrap() - 64.0).abs() < 1e-10); // 2^(3!)
        assert!((evaluate("3!^2", false).unwrap() - 36.0).abs() < 1e-10); // (3!)^2
        // Looser than unary minus: -3! = -(3!) = -6
        assert!((evaluate("-3!", false).unwrap() + 6.0).abs() < 1e-10);
        // Domain errors
        assert!(evaluate("171!", false).is_err());
        assert!(evaluate("2.5!", false).is_err());
        assert!(evaluate("(-1)!", false).is_err());
    }

    // ── Implicit multiplication ──────────────────────────────────────────
    #[test]
    fn implicit_multiplication() {
        let v = evaluate("2π", false).unwrap();
        assert!((v - 2.0 * PI).abs() < 1e-12, "2π = {v}");
        assert!((evaluate("3(4+1)", false).unwrap() - 15.0).abs() < 1e-12);
        assert!((evaluate("(2)(3)", false).unwrap() - 6.0).abs() < 1e-12);
        // 2·sin(30°) = 1
        let s = evaluate("2sin(30)", true).unwrap();
        assert!((s - 1.0).abs() < 1e-12, "2sin(30) = {s}");
        // Must NOT swallow subtraction: 2-3 = -1, not 2·(-3)
        assert!((evaluate("2-3", false).unwrap() + 1.0).abs() < 1e-12);
    }

    // ── Pretty symbols + e-notation guard ────────────────────────────────
    #[test]
    fn pretty_symbols() {
        assert!((evaluate("5 − 3", false).unwrap() - 2.0).abs() < 1e-12); // U+2212
        assert!((evaluate("6 ÷ 2", false).unwrap() - 3.0).abs() < 1e-12);
        assert!((evaluate("2 × 3", false).unwrap() - 6.0).abs() < 1e-12);
        assert!((evaluate("√(9)",  false).unwrap() - 3.0).abs() < 1e-12);
    }

    #[test]
    fn e_after_digit_is_euler_not_exponent() {
        // "2e" used to tokenise as a malformed number; now it is 2·e.
        let v = evaluate("2e", false).unwrap();
        assert!((v - 2.0 * E).abs() < 1e-12, "2e = {v}");
        // Real exponents still parse:
        assert!((evaluate("1.5e3",  false).unwrap() - 1500.0).abs() < 1e-9);
        assert!((evaluate("2e-2",   false).unwrap() - 0.02).abs()  < 1e-12);
        assert!((evaluate("1e+2",   false).unwrap() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn unclosed_paren_is_error() {
        assert!(evaluate("sin(30", false).is_err());
        assert!(evaluate("(2+3",   false).is_err());
    }

    #[test]
    fn fmt_num_cases() {
        assert_eq!(fmt_num(0.0),     "0");
        assert_eq!(fmt_num(1.0),     "1");
        assert_eq!(fmt_num(2.784),   "2.784");
        assert_eq!(fmt_num(1.5),     "1.5");
        assert_eq!(fmt_num(-0.125),  "-0.125");
    }
}
