#![allow(dead_code)]
//! Programmer calculator engine — integer arithmetic, base conversion, bit ops.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base { Hex = 16, Dec = 10, Oct = 8, Bin = 2 }

impl Base {
    pub fn name(self) -> &'static str {
        match self { Self::Hex=>"HEX", Self::Dec=>"DEC", Self::Oct=>"OCT", Self::Bin=>"BIN" }
    }
    pub fn from_name(s: &str) -> Option<Self> {
        match s { "HEX"=>Some(Self::Hex), "DEC"=>Some(Self::Dec),
                  "OCT"=>Some(Self::Oct), "BIN"=>Some(Self::Bin), _=>None }
    }
    pub fn radix(self) -> u32 { self as u32 }
    pub fn digit_ok(self, c: char) -> bool {
        match self {
            Self::Bin => matches!(c, '0'|'1'),
            Self::Oct => matches!(c, '0'..='7'),
            Self::Dec => c.is_ascii_digit(),
            Self::Hex => c.is_ascii_hexdigit(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitOp { And, Or, Xor, Shl, Shr }

impl BitOp {
    pub fn symbol(self) -> &'static str {
        match self { Self::And=>"AND", Self::Or=>"OR", Self::Xor=>"XOR",
                     Self::Shl=>"<<",  Self::Shr=>">>" }
    }
}

#[derive(Debug)]
struct Pending { lhs: i64, op: BitOp }

/// One completed programmer-mode computation (lhs, rhs, op, result),
/// used to build a history entry.
#[derive(Debug, Clone, Copy)]
pub struct Computed {
    pub lhs:   i64,
    pub rhs:   i64,
    pub op:    BitOp,
    pub result: i64,
}

#[derive(Debug)]
pub struct ProgrammerState {
    pub base:      Base,
    pub bit_width: u8,   // 8 | 16 | 32 | 64
    input:         String,
    pending:       Option<Pending>,
}

impl ProgrammerState {
    pub fn new() -> Self {
        Self { base: Base::Dec, bit_width: 64, input: "0".into(), pending: None }
    }

    // ── Value ─────────────────────────────────────────────────────────────
    /// Parse the raw input. Unsigned-first so full-width hex/oct/bin literals
    /// such as FFFFFFFFFFFFFFFF wrap around (two's complement) instead of
    /// silently becoming 0 (the old i64-only parse failed and fell back to 0).
    fn raw_value(&self) -> i64 {
        match u64::from_str_radix(&self.input, self.base.radix()) {
            Ok(v)  => v as i64,
            // Negative decimal display (e.g. after NOT / subtraction results)
            Err(_) => i64::from_str_radix(&self.input, self.base.radix()).unwrap_or(0),
        }
    }
    fn mask(&self, v: i64) -> i64 {
        match self.bit_width { 8=>v as i8 as i64, 16=>v as i16 as i64,
                               32=>v as i32 as i64, _=>v }
    }
    /// Unsigned representation truncated to the active bit width, so HEX /
    /// OCT / BIN render e.g. −1 as `FF`, not `FFFFFFFFFFFFFFFF`, when a
    /// narrower width is selected.
    fn mask_u64(&self, v: i64) -> u64 {
        match self.bit_width { 8=>(v as u8) as u64, 16=>(v as u16) as u64,
                               32=>(v as u32) as u64, _=>v as u64 }
    }
    pub fn value(&self) -> i64 { self.mask(self.raw_value()) }

    pub fn display_in(&self, b: Base) -> String {
        let v = self.mask_u64(self.value());
        match b {
            Base::Dec => format!("{}", self.value()),
            Base::Hex => format!("{v:X}"),
            Base::Oct => format!("{v:o}"),
            Base::Bin => format!("{v:b}"),
        }
    }

    pub fn current_display(&self) -> String { self.input.clone() }

    // ── Input ─────────────────────────────────────────────────────────────
    pub fn clear(&mut self) { self.input = "0".into(); self.pending = None; }

    pub fn backspace(&mut self) {
        if self.input.len() > 1 {
            self.input.pop();
            // Never leave a bare "-" (negative decimal) on the display.
            if self.input == "-" || self.input.is_empty() {
                self.input = "0".into();
            }
        } else {
            self.input = "0".into();
        }
    }

    pub fn digit(&mut self, c: char) {
        if !self.base.digit_ok(c) { return; }
        let ch = c.to_ascii_uppercase().to_string();
        if self.input == "0" { self.input = ch; }
        else if self.input == "-0" { self.input = format!("-{ch}"); }
        else if self.input.len() < self.max_input_len() { self.input.push_str(&ch); }
    }

    /// Max representable digit count for the active base and bit width.
    /// Beyond this the literal would overflow the width — the old fixed
    /// cap of 20 chars let DEC/BIN inputs grow until `from_str_radix`
    /// failed and silently reset the value to 0.
    fn max_input_len(&self) -> usize {
        let base_max = match self.base {
            Base::Hex => self.bit_width as usize / 4,
            Base::Oct => self.bit_width as usize / 3,
            Base::Bin => self.bit_width as usize,
            Base::Dec => match self.bit_width {
                8 => 3, 16 => 5, 32 => 10, _ => 19,   // digits of 2^w − 1
            },
        };
        base_max + usize::from(self.input.starts_with('-'))
    }

    // ── Operations ────────────────────────────────────────────────────────
    /// Set a pending bit-op. Any previously pending op is evaluated first so
    /// chains like `5 AND 3 OR 8 =` work left-to-right.
    pub fn set_op(&mut self, op: BitOp) {
        self.compute(); // flush pending chain — its result becomes the LHS
        self.pending = Some(Pending { lhs: self.value(), op });
        self.input   = "0".into();
    }

    pub fn compute(&mut self) -> Option<Computed> {
        let p = self.pending.take()?;
        let rhs = self.value();
        let res = match p.op {
            BitOp::And => p.lhs & rhs,
            BitOp::Or  => p.lhs | rhs,
            BitOp::Xor => p.lhs ^ rhs,
            BitOp::Shl => p.lhs.wrapping_shl(rhs.unsigned_abs() as u32),
            BitOp::Shr => p.lhs.wrapping_shr(rhs.unsigned_abs() as u32),
        };
        let res = self.mask(res);
        self.set_value(res);
        Some(Computed { lhs: p.lhs, rhs, op: p.op, result: res })
    }

    pub fn not(&mut self) { let v = self.mask(!self.value()); self.set_value(v); }

    fn set_value(&mut self, v: i64) {
        let u = self.mask_u64(v);
        self.input = match self.base {
            Base::Dec => format!("{}", self.mask(v)),
            Base::Hex => format!("{u:X}"),
            Base::Oct => format!("{u:o}"),
            Base::Bin => format!("{u:b}"),
        };
        if self.input.is_empty() { self.input = "0".into(); }
    }

    pub fn set_base(&mut self, b: Base) {
        let v     = self.value();
        self.base = b;
        self.set_value(v);
    }

    /// Switch the working bit width (8/16/32/64). The current value is
    /// re-masked (truncated/sign-extended) to the new width.
    pub fn set_width(&mut self, w: u8) {
        if !matches!(w, 8 | 16 | 32 | 64) || w == self.bit_width { return; }
        self.bit_width = w;
        let v = self.value();      // raw input, re-masked to the new width
        self.set_value(v);
    }

    pub fn width_name(&self) -> &'static str {
        match self.bit_width { 8 => "BYTE", 16 => "WORD", 32 => "DWORD", _ => "QWORD" }
    }

    /// Feedback line for the display: `FF AND …` while a bit-op is pending.
    pub fn pending_expr(&self) -> String {
        match &self.pending {
            Some(p) => format!("{} {} …", self.fmt_base(p.lhs), p.op.symbol()),
            None    => String::new(),
        }
    }

    fn fmt_base(&self, v: i64) -> String {
        let u = self.mask_u64(v);
        match self.base {
            Base::Dec => format!("{v}"),
            Base::Hex => format!("{u:X}"),
            Base::Oct => format!("{u:o}"),
            Base::Bin => format!("{u:b}"),
        }
    }

    pub fn load_f64(&mut self, v: f64) { self.set_value(v as i64); }
}

impl Default for ProgrammerState { fn default() -> Self { Self::new() } }

// ── Tests ─────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_full_width_wraps_not_zero() {
        let mut p = ProgrammerState::new();
        p.set_base(Base::Hex);
        for _ in 0..16 { p.digit('F'); }
        assert_eq!(p.value(), -1); // 0xFFFF…FFFF = u64::MAX → −1
    }

    #[test]
    fn oct_wraps_after_21_digits_start() {
        let mut p = ProgrammerState::new();
        p.set_base(Base::Oct);
        for _ in 0..21 { p.digit('7'); }
        // 21 octal digits exceed i64 — must not silently become 0
        assert_ne!(p.value(), 0);
    }

    #[test]
    fn bitop_chain_left_to_right() {
        let mut p = ProgrammerState::new();
        p.digit('5');
        p.set_op(BitOp::And);
        p.digit('3');
        p.set_op(BitOp::Or);      // flushes 5 AND 3 = 1
        p.digit('8');
        let c = p.compute().unwrap();
        assert_eq!(c.result, 9);
        assert_eq!((c.lhs, c.rhs), (1, 8));
    }

    #[test]
    fn backspace_on_negative_decimal() {
        let mut p = ProgrammerState::new();
        p.set_value(-42);
        assert_eq!(p.current_display(), "-42");
        p.backspace();            // "-4"
        p.backspace();            // "-" → guard → "0"
        assert_eq!(p.current_display(), "0");
    }

    #[test]
    fn not_respects_mask() {
        let mut p = ProgrammerState::new();
        p.bit_width = 8;
        p.set_value(0);
        p.not();
        assert_eq!(p.value(), -1);
        assert_eq!(p.display_in(Base::Hex), "FF");
    }

    #[test]
    fn base_switch_preserves_value() {
        let mut p = ProgrammerState::new();
        p.set_value(255);
        p.set_base(Base::Hex);
        assert_eq!(p.current_display(), "FF");
        p.set_base(Base::Bin);
        assert_eq!(p.current_display(), "11111111");
        p.set_base(Base::Dec);
        assert_eq!(p.current_display(), "255");
    }

    #[test]
    fn digit_rejects_invalid_for_base() {
        let mut p = ProgrammerState::new();
        p.set_base(Base::Bin);
        p.digit('2'); // invalid in BIN
        assert_eq!(p.current_display(), "0");
        p.digit('1');
        assert_eq!(p.current_display(), "1");
    }

    #[test]
    fn digit_cap_matches_width() {
        let mut p = ProgrammerState::new(); // DEC, QWORD
        for _ in 0..25 { p.digit('9'); }
        assert_eq!(p.current_display().len(), 19); // capped, never overflows
        p.set_base(Base::Hex);
        for _ in 0..25 { p.digit('F'); }
        assert_eq!(p.current_display().len(), 16);
        p.set_width(8);
        // BYTE: re-masked value re-rendered, further input capped at 2 hex digits
        assert_eq!(p.current_display(), "FF");
        p.clear();
        for _ in 0..10 { p.digit('F'); }
        assert_eq!(p.current_display(), "FF");
    }

    #[test]
    fn set_width_re_masks_value() {
        let mut p = ProgrammerState::new();
        p.set_value(300);            // QWORD DEC
        p.set_width(8);
        assert_eq!(p.current_display(), "44"); // 300 as i8
        assert_eq!(p.display_in(Base::Hex), "2C");
        // Truncation is irreversible — 44 fits every wider width unchanged
        p.set_width(16);
        assert_eq!(p.current_display(), "44");
        // A value that fits is preserved through width switches
        p.set_value(300);
        p.set_width(16);
        assert_eq!(p.current_display(), "300");
        p.set_width(32);
        assert_eq!(p.current_display(), "300");
    }

    #[test]
    fn width_name_mapping() {
        let p = ProgrammerState::new();
        assert_eq!(p.width_name(), "QWORD");
    }

    #[test]
    fn pending_expr_shows_lhs_and_op() {
        let mut p = ProgrammerState::new();
        p.set_base(Base::Hex);
        p.digit('F');
        p.set_op(BitOp::And);
        assert_eq!(p.pending_expr(), "F AND …");
    }
}
