//! Core calculator state machine.
//! Handles all 4 modes. All side-effects go through `handle_input`.

use crate::{
    history::History,
    memory::Memory,
    modes::{
        financial::{TvmState, margin_pct, tax_add, tax_remove},
        programmer::{Base, BitOp, ProgrammerState},
    },
    parser::fmt_num,
};

// ── Mode ──────────────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode { Basic=0, Scientific=1, Financial=2, Programmer=3 }

impl Mode {
    pub fn from_int(i: i32) -> Self {
        match i { 1=>Self::Scientific, 2=>Self::Financial, 3=>Self::Programmer, _=>Self::Basic }
    }
}

// ── Arithmetic op ─────────────────────────────────────────────────────────
#[derive(Debug, Clone, Copy, PartialEq)]
enum Op { Add, Sub, Mul, Div, Pow }

impl Op {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "+"    => Some(Self::Add), "−"|"-"  => Some(Self::Sub),
            "×"|"*"=> Some(Self::Mul), "÷"|"/"  => Some(Self::Div),
            "xʸ"|"^"=> Some(Self::Pow),
            _      => None,
        }
    }
    fn apply(self, a: f64, b: f64) -> Result<f64, String> {
        match self {
            Self::Add => Ok(a + b),
            Self::Sub => Ok(a - b),
            Self::Mul => Ok(a * b),
            Self::Div => { if b == 0.0 { Err("÷0".into()) } else { Ok(a / b) } }
            Self::Pow => Ok(a.powf(b)),
        }
    }
    fn symbol(self) -> &'static str {
        match self { Self::Add=>"+", Self::Sub=>"−", Self::Mul=>"×",
                     Self::Div=>"÷", Self::Pow=>"^" }
    }
}

// ── Financial CPT state ───────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq)]
enum FinMode { Normal, Cpt }

// ═══════════════════════════════════════════════════════════════════════════
pub struct Calculator {
    pub mode:       Mode,
    pub display:    String,
    pub expression: String,
    pub memory:     Memory,
    pub history:    History,

    // Shared numeric state
    input:          String,
    pending_op:     Option<Op>,
    pending_val:    f64,
    should_reset:   bool,
    error:          bool,

    // Repeat-last-operation ("=" pressed again after a result)
    last_op:        Option<Op>,
    last_rhs:       f64,

    // Scientific
    pub degree_mode: bool,
    /// Scientific mode is expression-based: `expression` holds the live
    /// formula, `sci_evaluated` marks the just-showed-result state, and
    /// `preview` tells the UI to render the display dimmed (live preview).
    sci_evaluated:   bool,
    preview:         bool,

    // Financial
    pub tvm:      TvmState,
    pub tvm_sel:  String,
    fin_mode:     FinMode,
    pub tax_rate: f64,

    // Programmer
    pub prog: ProgrammerState,
}

impl Calculator {
    pub fn new() -> Self {
        Self {
            mode:         Mode::Basic,
            display:      "0".into(),
            expression:   String::new(),
            memory:       Memory::new(),
            history:      History::new(),
            input:        "0".into(),
            pending_op:   None,
            pending_val:  0.0,
            should_reset: false,
            error:        false,
            last_op:      None,
            last_rhs:     0.0,
            degree_mode:  true,
            sci_evaluated: false,
            preview:      false,
            tvm:          TvmState::new(),
            tvm_sel:      String::new(),
            fin_mode:     FinMode::Normal,
            tax_rate:     20.0,
            prog:         ProgrammerState::new(),
        }
    }

    pub fn set_mode(&mut self, m: i32) {
        let new_mode = Mode::from_int(m);
        // Re-clicking the already-active tab must not wipe the calculation.
        if new_mode == self.mode { return; }
        self.mode = new_mode;
        self.reset_arith();
        self.fin_mode = FinMode::Normal;
        self.sync_display();
    }

    // ── Main dispatch ─────────────────────────────────────────────────────
    pub fn handle_input(&mut self, btn: &str) {
        match self.mode {
            Mode::Basic      => self.basic(btn),
            Mode::Scientific => self.scientific(btn),
            Mode::Financial  => self.financial(btn),
            Mode::Programmer => self.programmer(btn),
        }
    }

    /// Recall a history entry without replaying the result string through
    /// the input pipeline (which would break on '-', 'e' notation, …).
    pub fn recall_from_history(&mut self, expr: &str, result: &str) {
        self.reset_arith();
        if self.mode == Mode::Scientific {
            // Scientific recalls the entry as an editable expression again.
            self.expression = expr.trim_end_matches(|c: char| c == '=' || c == ' ').to_string();
            if let Ok(v) = result.parse::<f64>() { self.input = fmt_num(v); }
            self.sci_evaluated = false;
            self.sci_sync();
            return;
        }
        self.expression = expr.to_string();
        match result.parse::<f64>() {
            Ok(v) if v.is_finite() => {
                self.input        = fmt_num(v);
                self.display      = self.input.clone();
                self.should_reset = true;
            }
            _ => {
                // Legacy / non-numeric entry ("∞", "Error"): show it, restart on input.
                self.display      = result.to_string();
                self.input        = "0".into();
                self.should_reset = true;
            }
        }
    }

    /// True when the big display currently shows a live preview of a partial
    /// scientific expression (rendered dimmed by the UI).
    pub fn preview_active(&self) -> bool { self.preview }

    // ═══════════════════════════════════════════════════════════════════════
    // BASIC
    // ═══════════════════════════════════════════════════════════════════════
    fn basic(&mut self, btn: &str) {
        match btn {
            "AC"  => self.clear_all(),
            "⌫"  => self.backspace(),
            "="   => self.evaluate_pending(),
            "%"   => self.percent_op(),
            "±"   => self.toggle_sign(),
            "MC"  => { self.memory.clear(); }
            "MR"  => { let v = self.memory.recall(); self.set_num(v); }
            "M+"  => { self.memory.add(self.cur_f64()); }
            "M-" | "M−" => { self.memory.sub(self.cur_f64()); }
            "MS"  => { self.memory.store(self.cur_f64()); }
            d if is_digit_btn(d) => self.append_digit(d),
            op    if Op::from_str(op).is_some() => self.choose_op(op),
            _ => {}
        }
        self.sync_display();
    }

    // ═══════════════════════════════════════════════════════════════════════
    // SCIENTIFIC — full expression entry. The typed formula lives in
    // `expression` and is evaluated by the recursive-descent parser on "=".
    // The big display shows a live preview while the expression is still
    // incomplete-but-valid, or the operand being typed otherwise.
    // ═══════════════════════════════════════════════════════════════════════
    fn scientific(&mut self, btn: &str) {
        // Any key after an error restarts the expression.
        if self.error && btn != "AC" { self.sci_clear(); }

        match btn {
            "AC"       => { self.sci_clear(); return; }
            "⌫"        => { self.sci_backspace(); self.sci_sync(); return; }
            "="        => { self.sci_evaluate(); return; }
            "DEG/RAD"  => { self.degree_mode = !self.degree_mode; self.sci_sync(); return; }
            "±"        => { self.sci_toggle_sign(); self.sci_sync(); return; }
            "sin" | "cos" | "tan" | "asin" | "acos" | "atan" | "log" | "ln"
                       => { self.sci_insert_fn(btn); self.sci_sync(); return; }
            "√x"       => { self.sci_insert_fn("√"); self.sci_sync(); return; }
            "10ˣ"      => { self.sci_insert_text("10^("); self.sci_sync(); return; }
            "eˣ"       => { self.sci_insert_text("e^(");  self.sci_sync(); return; }
            "x²"       => { self.sci_square();   self.sci_sync(); return; }
            "x!"       => { self.sci_bang();     self.sci_sync(); return; }
            "1/x"      => { self.sci_reciprocal(); self.sci_sync(); return; }
            "RND"      => { let r = fmt_num(pseudo_rand());
                            self.sci_insert_text(&r); self.sci_sync(); return; }
            "π" | "e"  => { self.sci_insert_text(btn); self.sci_sync(); return; }
            "("        => { self.sci_open_paren();  self.sci_sync(); return; }
            ")"        => { self.sci_close_paren(); self.sci_sync(); return; }
            _ => {}
        }

        // Digits
        if matches!(btn, "0"|"1"|"2"|"3"|"4"|"5"|"6"|"7"|"8"|"9"|".") {
            self.sci_digit(btn);
            self.sci_sync();
            return;
        }
        // Binary operators — "%" is modulo in this mode.
        if let Some(sym) = sci_op_symbol(btn) {
            self.sci_op(sym);
            self.sci_sync();
        }
    }

    // ── Scientific expression helpers ─────────────────────────────────────
    fn sci_clear(&mut self) {
        self.expression.clear();
        self.display       = "0".into();
        self.input         = "0".into();
        self.error         = false;
        self.sci_evaluated = false;
        self.preview       = false;
    }

    /// True when the expression's last char ends a value, so a following
    /// value / `(` / function needs an explicit `×` inserted.
    fn sci_ends_as_value(&self) -> bool {
        self.expression.ends_with(|c: char|
            c.is_ascii_digit() || c == '.' || c == ')' || c == '!' || c == 'π' || c == 'e')
    }

    fn sci_ends_as_operator(&self) -> bool {
        self.expression.ends_with(|c: char| matches!(c, '+' | '−' | '×' | '÷' | '^' | '%'))
    }

    fn sci_open_parens(&self) -> usize {
        self.expression.chars().filter(|&c| c == '(').count()
            - self.expression.chars().filter(|&c| c == ')').count()
    }

    /// Insert `×` when a value directly follows another value (2π, 5sin(…)).
    fn sci_value_comes(&mut self) {
        if self.sci_ends_as_value() { self.expression.push('×'); }
    }

    /// After "=", a digit starts a fresh expression; an operator continues
    /// from the result.
    fn sci_reset_evaluated(&mut self) {
        if self.sci_evaluated {
            self.expression.clear();
            self.sci_evaluated = false;
        }
    }

    fn sci_digit(&mut self, d: &str) {
        self.sci_reset_evaluated();
        // Continue a trailing operand (leading-zero / double-dot guards).
        if let Some((start, len)) = trailing_number(&self.expression) {
            if start + len == self.expression.len() {
                let num = &self.expression[start..];
                if d == "." {
                    if !num.contains('.') { self.expression.push('.'); }
                    return;
                }
                if num == "0" {
                    self.expression.truncate(start);
                    self.expression.push_str(d);
                    return;
                }
                if len < 16 { self.expression.push_str(d); }
                return;
            }
        }
        // Brand-new operand — implicit × after `)` `!` `π` `e`.
        self.sci_value_comes();
        if d == "." { self.expression.push_str("0."); }
        else        { self.expression.push_str(d); }
    }

    fn sci_op(&mut self, sym: &str) {
        // Continue a finished expression from its result.
        if self.sci_evaluated {
            self.expression   = format!("{}{sym}", self.input);
            self.sci_evaluated = false;
            return;
        }
        if self.expression.is_empty() {
            self.expression = format!("0{sym}");
            return;
        }
        // Replace a trailing operator (5 + × → 5 ×).
        if self.sci_ends_as_operator() {
            self.expression.pop();
            if self.expression.is_empty() { self.expression = format!("0{sym}"); return; }
            self.expression.push_str(sym);
            return;
        }
        // Only a unary minus may directly follow "(" → (−5
        if self.expression.ends_with('(') && sym != "−" { return; }
        self.expression.push_str(sym);
    }

    fn sci_insert_fn(&mut self, name: &str) {
        self.sci_reset_evaluated();
        self.sci_value_comes();          // "2 sin(" → "2×sin("
        self.expression.push_str(name);
        self.expression.push('(');
    }

    fn sci_insert_text(&mut self, text: &str) {
        self.sci_reset_evaluated();
        self.sci_value_comes();
        self.expression.push_str(text);
    }

    fn sci_open_paren(&mut self) {
        self.sci_reset_evaluated();
        self.sci_value_comes();          // "2(" → "2×("
        self.expression.push('(');
    }

    fn sci_close_paren(&mut self) {
        self.sci_reset_evaluated();
        if self.sci_open_parens() > 0 && self.sci_ends_as_value() {
            self.expression.push(')');
        }
    }

    fn sci_bang(&mut self) {
        self.sci_reset_evaluated();
        if self.sci_ends_as_value() { self.expression.push('!'); }
    }

    fn sci_square(&mut self) {
        self.sci_reset_evaluated();
        if self.sci_ends_as_value() { self.expression.push_str("^2"); }
    }

    /// 1/x on the trailing operand (3+5 → 3+1/(5)), or wrapping the whole
    /// expression when it already ends with a closed group.
    fn sci_reciprocal(&mut self) {
        if self.sci_evaluated {
            let v = self.cur_f64();
            self.expression    = format!("1/({})", fmt_num(v));
            self.sci_evaluated = false;
            return;
        }
        if self.expression.ends_with(')') {
            self.expression = format!("1/({})", self.expression);
            return;
        }
        if let Some((start, len)) = trailing_number(&self.expression) {
            if start + len == self.expression.len() {
                let num = self.expression[start..].to_string();
                self.expression.truncate(start);
                self.expression.push_str(&format!("1/({num})"));
                return;
            }
        }
        let v = self.cur_f64();
        self.expression = format!("1/({})", fmt_num(v));
    }

    /// Toggle the sign of the trailing operand (unary minus handling).
    fn sci_toggle_sign(&mut self) {
        if self.sci_evaluated {
            let v = -self.cur_f64();
            self.expression    = fmt_num(v);
            self.sci_evaluated = false;
            return;
        }
        let Some((start, len)) = trailing_number(&self.expression) else { return };
        if start + len != self.expression.len() { return; }
        let prefix = &self.expression[..start];
        if let Some(stripped) = prefix.strip_suffix('−') {
            let is_unary = stripped.is_empty()
                || stripped.ends_with(|c: char|
                    matches!(c, '+' | '−' | '×' | '÷' | '^' | '%' | '('));
            if is_unary {
                // remove the unary minus
                self.expression = format!("{stripped}{}", &self.expression[start..]);
            } else {
                // binary minus before the operand → negate it: 5−3 → 5−−3
                self.expression.insert(start, '−');
            }
        } else {
            self.expression.insert(start, '−');
        }
    }

    fn sci_backspace(&mut self) {
        if self.error { self.sci_clear(); return; }
        if self.sci_evaluated {
            // "5 + 3 =" → back to editable "5 + 3"
            let len = self.expression.trim_end_matches(|c: char| c == '=' || c == ' ').len();
            self.expression.truncate(len);
            self.sci_evaluated = false;
            self.sci_sync();
            return;
        }
        if self.expression.is_empty() { return; }
        // Remove a whole "fn(" in one stroke (longest names first).
        for unit in ["asin(", "acos(", "atan(", "sin(", "cos(", "tan(",
                     "log(", "ln(", "√(", "10^(", "e^("] {
            if let Some(head) = self.expression.strip_suffix(unit) {
                self.expression = head.to_string();
                return;
            }
        }
        self.expression.pop();
    }

    fn sci_evaluate(&mut self) {
        if self.expression.is_empty() { return; }
        // Re-pressing "=" re-evaluates the same expression.
        let expr = self.expression.trim_end_matches(|c: char| c == '=' || c == ' ').to_string();
        if expr.is_empty() { return; }
        match crate::parser::evaluate(&expr, self.degree_mode) {
            Ok(v) if v.is_finite() => {
                let s = fmt_num(v);
                self.history.push(&expr, &s);
                self.expression    = format!("{expr} =");
                self.display       = s.clone();
                self.input         = s;
                self.sci_evaluated = true;
                self.preview       = false;
                self.pending_op    = None;
                self.pending_val   = 0.0;
                self.should_reset  = true;
            }
            Ok(_)  => self.set_error("Overflow"),
            Err(_) => {
                // Keep the typed expression; only flag the display.
                self.display    = "Error".into();
                self.expression = expr;
                self.error      = true;
                self.input      = "0".into();
            }
        }
    }

    /// Refresh the display: live preview when the partial expression already
    /// evaluates, otherwise the operand being typed.
    fn sci_sync(&mut self) {
        if self.error || self.sci_evaluated { return; }
        if self.expression.is_empty() {
            self.display = "0".into();
            self.preview = false;
            return;
        }
        match crate::parser::evaluate(&self.expression, self.degree_mode) {
            Ok(v) if v.is_finite() => {
                self.display = fmt_num(v);
                self.preview = true;
            }
            _ => {
                // Show the operand being typed (a trailing "!" may follow it).
                let trimmed = self.expression.trim_end_matches('!');
                self.display = trailing_number(trimmed)
                    .map(|(s, l)| trimmed[s..s + l].to_string())
                    .unwrap_or_else(|| "0".into());
                self.preview = false;
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // FINANCIAL
    // ═══════════════════════════════════════════════════════════════════════
    fn financial(&mut self, btn: &str) {
        match btn {
            "AC" => {
                self.clear_all();
                self.fin_mode = FinMode::Normal;
                self.tvm_sel  = String::new();
                self.sync_display();
                return;
            }
            "⌫" => { self.backspace(); self.sync_display(); return; }
            "±" => { self.toggle_sign(); self.sync_display(); return; }
            "CLR TVM" => {
                self.tvm.clear();
                self.fin_mode = FinMode::Normal;
                self.tvm_sel  = String::new();
                self.clear_all();
                return;
            }
            "CPT" => {
                self.fin_mode = FinMode::Cpt;
                self.expression = "CPT →".into();
                self.sync_display();
                return;
            }
            _ => {}
        }

        // TVM slot buttons
        if let Some(slot) = btn.strip_prefix("TVM:") {
            let slot = slot.to_string();
            if self.fin_mode == FinMode::Cpt {
                // Compute the missing variable
                match self.tvm.compute(&slot) {
                    Ok(v) if v.is_finite() => {
                        self.store_tvm(&slot, v);
                        let expr = format!("{slot} = {}", fmt_num(v));
                        self.history.push(&expr, fmt_num(v));
                        self.expression = expr;
                        self.set_num(v);
                    }
                    Ok(_)  => self.set_error("Overflow"),
                    Err(e) => self.set_error(&e),
                }
                self.fin_mode = FinMode::Normal;
                self.tvm_sel  = String::new();
            } else {
                // Store current display into TVM slot
                let v = self.cur_f64();
                self.store_tvm(&slot, v);
                self.expression = format!("{slot} = {}", fmt_num(v));
                self.tvm_sel    = slot;
                self.should_reset = true;
            }
            self.sync_display();
            return;
        }

        // Tax / Margin helpers
        match btn {
            "Tax+" => {
                let base  = self.cur_f64();
                let total = tax_add(base, self.tax_rate);
                self.expression = format!("{} + {}% = {}", fmt_num(base), self.tax_rate, fmt_num(total));
                self.set_num(total);
                self.sync_display();
                return;
            }
            "Tax−" => {
                let total = self.cur_f64();
                let base  = tax_remove(total, self.tax_rate);
                self.expression = format!("{} (incl {}% tax, net = {})", fmt_num(total), self.tax_rate, fmt_num(base));
                self.set_num(base);
                self.sync_display();
                return;
            }
            "Mar" => {
                let sell = self.cur_f64();
                let cost = self.tvm.pv.unwrap_or(self.pending_val);
                match margin_pct(cost, sell) {
                    Ok(pct) => {
                        self.expression = format!("Margin {}/{} = {:.2}%", fmt_num(cost), fmt_num(sell), pct);
                        self.set_num(pct);
                    }
                    Err(e) => self.set_error(&e),
                }
                self.sync_display();
                return;
            }
            _ => {}
        }

        // Standard arithmetic
        match btn {
            "=" => self.evaluate_pending(),
            "%" => self.percent_op(),
            d if is_digit_btn(d)           => self.append_digit(d),
            op if Op::from_str(op).is_some()=> self.choose_op(op),
            _ => {}
        }
        self.sync_display();
    }

    fn store_tvm(&mut self, slot: &str, v: f64) {
        match slot {
            "N"   => self.tvm.n   = Some(v),
            "I/Y" => self.tvm.iy  = Some(v),
            "PV"  => self.tvm.pv  = Some(v),
            "PMT" => self.tvm.pmt = Some(v),
            "FV"  => self.tvm.fv  = Some(v),
            _ => {}
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // PROGRAMMER
    // ═══════════════════════════════════════════════════════════════════════
    fn programmer(&mut self, btn: &str) {
        let mut expr_out: Option<String> = None;
        match btn {
            "AC" => { self.prog.clear(); }
            "⌫"  => { self.prog.backspace(); }
            "="   => {
                if let Some(com) = self.prog.compute() {
                    let expr = format!("{} {} {} =", com.lhs, com.op.symbol(), com.rhs);
                    let res  = com.result.to_string();
                    self.history.push(&expr, &res);
                    expr_out = Some(expr);
                }
            }
            "NOT" => { self.prog.not(); }
            "AND" => { self.prog.set_op(BitOp::And); }
            "OR"  => { self.prog.set_op(BitOp::Or);  }
            "XOR" => { self.prog.set_op(BitOp::Xor); }
            "<<"  => { self.prog.set_op(BitOp::Shl); }
            ">>"  => { self.prog.set_op(BitOp::Shr); }
            s if s.starts_with("BASE:") => {
                if let Some(b) = Base::from_name(&s[5..]) { self.prog.set_base(b); }
            }
            s if s.starts_with("WIDTH:") => {
                // Bit-width selector: QWORD / DWORD / WORD / BYTE
                if let Ok(w) = s[6..].parse::<u8>() { self.prog.set_width(w); }
            }
            ch if ch.len() == 1 => {
                if let Some(c) = ch.chars().next() { self.prog.digit(c); }
            }
            _ => {}
        }
        self.display    = self.prog.current_display();
        // `=` keeps its full "lhs op rhs =" line; a pending bit-op otherwise
        // shows "FF AND …" so the display area is no longer blank mid-input.
        self.expression = expr_out.unwrap_or_else(|| self.prog.pending_expr());
    }

    // ── Programmer display helpers (called from app.rs) ───────────────────
    pub fn hex_text(&self) -> String { self.prog.display_in(Base::Hex) }
    pub fn dec_text(&self) -> String { self.prog.display_in(Base::Dec) }
    pub fn oct_text(&self) -> String { self.prog.display_in(Base::Oct) }
    pub fn bin_text(&self) -> String { self.prog.display_in(Base::Bin) }

    // ═══════════════════════════════════════════════════════════════════════
    // Shared arithmetic helpers
    // ═══════════════════════════════════════════════════════════════════════

    fn append_digit(&mut self, d: &str) {
        if self.error { self.clear_all(); }
        if self.should_reset {
            self.input = String::new();
            self.should_reset = false;
            self.last_op = None; // fresh number breaks "="-repeat chain
        }
        match d {
            "." => {
                if !self.input.contains('.') {
                    if self.input.is_empty() { self.input.push('0'); }
                    self.input.push('.');
                }
            }
            _ => {
                if self.input == "0"       { self.input  = d.into(); }
                else if self.input.len()<16 { self.input.push_str(d); }
            }
        }
    }

    fn choose_op(&mut self, op: &str) {
        if self.error { return; }
        let Some(new_op) = Op::from_str(op) else { return; };

        // Chain: if a pending op exists, evaluate it first — errors must
        // surface instead of being silently dropped.
        if self.pending_op.is_some() && !self.should_reset {
            if let Some(o) = self.pending_op {
                match o.apply(self.pending_val, self.cur_f64()) {
                    Ok(r) => { self.pending_val = r; self.input = fmt_num(r); }
                    Err(e) => { self.set_error(&e); return; }
                }
            }
        }
        self.pending_val  = self.cur_f64();
        self.pending_op   = Some(new_op);
        self.expression   = format!("{} {}", fmt_num(self.pending_val), new_op.symbol());
        self.should_reset = true;
    }

    fn evaluate_pending(&mut self) {
        if self.error  { self.clear_all(); return; }

        // A second "=" repeats the last operation on the current value
        // (standard calculator behaviour).
        let (lhs, op, rhs) = match self.pending_op {
            Some(op) => (self.pending_val, op, self.cur_f64()),
            None => match self.last_op {
                Some(op) => (self.cur_f64(), op, self.last_rhs),
                None => return,
            },
        };

        let sym  = op.symbol();
        let expr = format!("{} {sym} {} =", fmt_num(lhs), fmt_num(rhs));

        match op.apply(lhs, rhs) {
            Ok(r) if r.is_finite() => {
                let s = fmt_num(r);
                self.history.push(&expr, &s);
                self.expression   = expr;
                self.input        = s.clone();
                self.display      = s;
                self.pending_op   = None;
                self.pending_val  = 0.0;
                self.last_op      = Some(op);
                self.last_rhs     = rhs;
                self.should_reset = true;
            }
            // Non-finite results (NaN / ±∞) must not pollute history.
            Ok(r)  => self.set_error(if r.is_nan() { "Undefined" } else { "Overflow" }),
            Err(e) => self.set_error(&e),
        }
    }

    fn percent_op(&mut self) {
        let v = self.cur_f64();
        let result = match self.pending_op {
            Some(Op::Add) | Some(Op::Sub) => self.pending_val * v / 100.0,
            _ => v / 100.0,
        };
        self.input        = fmt_num(result);
        self.should_reset = false;
    }

    fn toggle_sign(&mut self) {
        if self.error      { self.clear_all(); return; }
        if self.input.is_empty() || self.input == "0" { return; }
        if let Some(rest) = self.input.strip_prefix('-') {
            self.input = rest.into();
        } else {
            self.input.insert(0, '-');
        }
    }

    fn cur_f64(&self) -> f64 { self.input.parse().unwrap_or(0.0) }

    fn set_num(&mut self, v: f64) {
        self.input        = fmt_num(v);
        self.display      = self.input.clone();
        self.should_reset = true;
    }

    fn set_error(&mut self, msg: &str) {
        self.display      = "Error".into();
        self.expression   = msg.into();
        self.error        = true;
        self.input        = "0".into();
        self.should_reset = true;
    }

    fn clear_all(&mut self) {
        self.input        = "0".into();
        self.pending_op   = None;
        self.pending_val  = 0.0;
        self.should_reset = false;
        self.error        = false;
        self.last_op      = None;
        self.last_rhs     = 0.0;
        self.display      = "0".into();
        self.expression   = String::new();
        self.sci_evaluated = false;
        self.preview       = false;
    }

    fn backspace(&mut self) {
        if self.error        { self.clear_all(); return; }
        if self.should_reset { return; }
        if self.input.len() > 1 {
            self.input.pop();
            // Never leave a bare "-" (or empty input) on the display.
            if self.input.is_empty() || self.input == "-" {
                self.input = "0".into();
            }
        } else {
            self.input = "0".into();
        }
    }

    fn reset_arith(&mut self) {
        self.input        = "0".into();
        self.pending_op   = None;
        self.pending_val  = 0.0;
        self.should_reset = false;
        self.error        = false;
        self.last_op      = None;
        self.last_rhs     = 0.0;
        self.display      = "0".into();
        self.expression   = String::new();
        self.sci_evaluated = false;
        self.preview       = false;
    }

    fn sync_display(&mut self) {
        if !self.error { self.display = self.input.clone(); }
    }
}

impl Default for Calculator { fn default() -> Self { Self::new() } }

// ── Utilities ─────────────────────────────────────────────────────────────
// Digits and '.' only. Hex letters are programmer-mode-only (they used to
// leak into Basic/Scientific/Financial input and produce unparseable text
// like "A" that silently evaluated to 0).
fn is_digit_btn(s: &str) -> bool {
    matches!(s, "0"|"1"|"2"|"3"|"4"|"5"|"6"|"7"|"8"|"9"|".")
}

/// Byte range (start, len) of the LAST run of digits/dots in `s`.
/// None when the string contains no digits at all.
fn trailing_number(s: &str) -> Option<(usize, usize)> {
    let b     = s.as_bytes();
    let end   = b.len();
    let mut start = end;
    while start > 0 && (b[start - 1].is_ascii_digit() || b[start - 1] == b'.') {
        start -= 1;
    }
    if start == end { None } else { Some((start, end - start)) }
}

/// Map an incoming operator label to the pretty single-char symbol used
/// inside scientific expressions ("xʸ"/"^" → "^", ASCII '-' → "−", …).
fn sci_op_symbol(btn: &str) -> Option<&'static str> {
    Some(match btn {
        "+"          => "+",
        "-" | "−"    => "−",
        "*" | "×"    => "×",
        "/" | "÷"    => "÷",
        "xʸ" | "^"   => "^",
        "%"          => "%",
        _ => return None,
    })
}

/// Thread-local splitmix64 PRNG — a real sequence instead of a raw
/// nanosecond-timestamp hash, so rapid consecutive presses differ.
fn pseudo_rand() -> f64 {
    use std::cell::Cell;
    use std::time::{SystemTime, UNIX_EPOCH};

    thread_local! {
        static RNG: Cell<u64> = const { Cell::new(0) };
    }

    RNG.with(|cell| {
        let mut state = cell.get();
        if state == 0 {
            // Seed once from the wall clock (odd so it can never stick at 0).
            state = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x853C_49E6_748F_EA9B)
                | 1;
        }
        // splitmix64 step
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        cell.set(state);

        // 53 uniform bits → double in [0, 1)
        (z >> 11) as f64 / (1u64 << 53) as f64
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    fn press(c: &mut Calculator, btns: &[&str]) {
        for b in btns { c.handle_input(b); }
    }

    #[test]
    fn basic_arithmetic() {
        let mut c = Calculator::new();
        press(&mut c, &["5", "+", "3", "="]);
        assert_eq!(c.display, "8");
    }

    #[test]
    fn chained_operations() {
        let mut c = Calculator::new();
        press(&mut c, &["5", "+", "3", "×", "2", "="]);
        assert_eq!(c.display, "16");
    }

    #[test]
    fn division_by_zero_shows_error() {
        let mut c = Calculator::new();
        press(&mut c, &["5", "÷", "0", "="]);
        assert_eq!(c.display, "Error");
        assert_eq!(c.expression, "÷0");
        // Recovery: next digit starts fresh
        press(&mut c, &["7"]);
        assert_eq!(c.display, "7");
    }

    #[test]
    fn chained_division_by_zero_errors() {
        // The chain "5 ÷ 0 +" must surface ÷0 immediately — it used to be
        // silently swallowed, after which "3 =" produced the bogus "3".
        let mut c = Calculator::new();
        press(&mut c, &["5", "÷", "0", "+"]);
        assert_eq!(c.display, "Error");
        assert_eq!(c.expression, "÷0");
        assert_eq!(c.history.len(), 0);
    }

    #[test]
    fn repeat_equals_repeats_last_op() {
        let mut c = Calculator::new();
        press(&mut c, &["5", "+", "3", "=", "="]);
        assert_eq!(c.display, "11");
    }

    #[test]
    fn typing_after_equals_breaks_repeat() {
        let mut c = Calculator::new();
        press(&mut c, &["5", "+", "3", "=", "2", "0", "="]);
        assert_eq!(c.display, "20");
    }

    #[test]
    fn percent_additive() {
        let mut c = Calculator::new();
        press(&mut c, &["2", "0", "0", "+", "1", "0", "%", "="]);
        assert_eq!(c.display, "220");
    }

    #[test]
    fn memory_roundtrip() {
        let mut c = Calculator::new();
        press(&mut c, &["4", "2", "MS", "AC", "MR"]);
        assert_eq!(c.display, "42");
        press(&mut c, &["M+"]);                    // 42 + 42 = 84
        press(&mut c, &["AC", "MR"]);
        assert_eq!(c.display, "84");
        press(&mut c, &["M−", "MC", "AC", "MR"]);  // cleared
        assert_eq!(c.display, "0");
    }

    #[test]
    fn factorial_not_saturated() {
        let mut c = Calculator::new();
        c.set_mode(1);
        press(&mut c, &["2", "5", "x!"]);
        // 25! = 1.5511210043330986e25 (u64 version clamped this to 1.8e19)
        assert_eq!(c.display, "1.551121e25");
    }

    #[test]
    fn factorial_domain_error() {
        let mut c = Calculator::new();
        c.set_mode(1);
        // Live preview shows the operand; the error surfaces on "=".
        press(&mut c, &["5", ".", "5", "x!"]);
        assert_eq!(c.display, "5.5");
        press(&mut c, &["="]);
        assert_eq!(c.display, "Error");
        assert_eq!(c.history.len(), 0);
    }

    #[test]
    fn hex_letters_rejected_in_basic() {
        let mut c = Calculator::new();
        press(&mut c, &["A", "F"]);
        assert_eq!(c.display, "0");
    }

    #[test]
    fn non_finite_result_not_in_history() {
        let mut c = Calculator::new();
        c.set_mode(1);
        press(&mut c, &["1", "0", "xʸ", "4", "0", "0", "0", "="]);
        assert_eq!(c.display, "Error");
        assert_eq!(c.history.len(), 0);
    }

    #[test]
    fn same_mode_reclick_preserves_state() {
        let mut c = Calculator::new();
        press(&mut c, &["9"]);
        c.set_mode(0); // clicking the already-active tab
        assert_eq!(c.display, "9");
    }

    #[test]
    fn recall_negative_result_from_history() {
        let mut c = Calculator::new();
        press(&mut c, &["3", "−", "8", "="]);
        assert_eq!(c.display, "-5");
        let (expr, result) = {
            let e = c.history.get(0).unwrap();
            (e.expr.clone(), e.result.clone())
        };
        c.recall_from_history(&expr, &result);
        assert_eq!(c.display, "-5");
        // Typing after a recall starts a fresh number
        press(&mut c, &["7"]);
        assert_eq!(c.display, "7");
    }

    #[test]
    fn recall_scientific_notation_from_history() {
        let mut c = Calculator::new();
        c.recall_from_history("1 ÷ 8 =", "0.125");
        assert_eq!(c.display, "0.125");
        c.recall_from_history("tiny =", "1e-10");
        assert_eq!(c.display, "1.000000e-10");
    }

    #[test]
    fn backspace_never_leaves_bare_minus() {
        let mut c = Calculator::new();
        press(&mut c, &["3", "−", "8", "="]);   // -5, reset guard on
        press(&mut c, &["%"]);                  // -0.05, typing continues
        press(&mut c, &["⌫", "⌫", "⌫", "⌫"]);   // "-0.0" → "-0." → "-0" → "0"
        assert_eq!(c.display, "0");
    }

    #[test]
    fn sign_toggle() {
        let mut c = Calculator::new();
        c.set_mode(2);
        press(&mut c, &["1", "0", "0", "±"]);
        assert_eq!(c.display, "-100");
        press(&mut c, &["5"]);                   // continue typing
        assert_eq!(c.display, "-1005");
        press(&mut c, &["±"]);
        assert_eq!(c.display, "1005");
    }

    #[test]
    fn tvm_compound_growth() {
        let mut c = Calculator::new();
        c.set_mode(2);
        press(&mut c, &["1", "0", "TVM:N"]);
        press(&mut c, &["5", "TVM:I/Y"]);
        press(&mut c, &["1", "0", "0", "0", "±", "TVM:PV"]); // PV = -1000
        press(&mut c, &["0", "TVM:PMT"]);
        press(&mut c, &["CPT", "TVM:FV"]);
        // FV = 1000 × 1.05^10 = 1628.894627…
        let v: f64 = c.display.parse().unwrap();
        assert!((v - 1_628.894_627).abs() < 0.001, "FV was {v}");
    }

    #[test]
    fn tvm_recorded_in_history() {
        let mut c = Calculator::new();
        c.set_mode(2);
        press(&mut c, &["1", "0", "TVM:N"]);
        press(&mut c, &["5", "TVM:I/Y"]);
        press(&mut c, &["1", "0", "0", "0", "±", "TVM:PV"]);
        press(&mut c, &["0", "TVM:PMT"]);
        press(&mut c, &["CPT", "TVM:FV"]);
        assert_eq!(c.history.len(), 1);
    }

    #[test]
    fn programmer_hex_wraparound() {
        let mut c = Calculator::new();
        c.set_mode(3);
        press(&mut c, &["BASE:HEX"]);
        for _ in 0..16 { press(&mut c, &["F"]); }
        // 0xFFFFFFFFFFFFFFFF is u64::MAX → two's complement −1, not 0
        assert_eq!(c.dec_text(), "-1");
    }

    #[test]
    fn programmer_bitop_chain() {
        let mut c = Calculator::new();
        c.set_mode(3);
        press(&mut c, &["5", "AND", "3", "OR", "8", "="]);
        // ((5 AND 3) OR 8) = 9 — chaining used to drop the AND
        assert_eq!(c.dec_text(), "9");
    }

    #[test]
    fn programmer_shift_chain() {
        let mut c = Calculator::new();
        c.set_mode(3);
        press(&mut c, &["1", "<<", "4", "<<", "2", "="]);
        assert_eq!(c.dec_text(), "64"); // (1<<4)<<2
    }

    #[test]
    fn programmer_compute_recorded_in_history() {
        let mut c = Calculator::new();
        c.set_mode(3);
        press(&mut c, &["1", "2", "AND", "1", "0", "="]);
        assert_eq!(c.history.len(), 1);
        assert_eq!(c.history.get(0).unwrap().result, "8");
    }

    #[test]
    fn programmer_base_switch_roundtrip() {
        let mut c = Calculator::new();
        c.set_mode(3);
        press(&mut c, &["2", "5", "5"]);
        press(&mut c, &["BASE:HEX"]);
        assert_eq!(c.display, "FF");
        press(&mut c, &["BASE:BIN"]);
        assert_eq!(c.display, "11111111");
        press(&mut c, &["BASE:DEC"]);
        assert_eq!(c.display, "255");
    }

    #[test]
    fn rnd_stays_in_unit_range() {
        let mut c = Calculator::new();
        c.set_mode(1);
        for _ in 0..64 {
            press(&mut c, &["RND"]);
            let v: f64 = c.display.parse().unwrap();
            assert!((0.0..1.0).contains(&v), "RND out of range: {v}");
        }
    }

    // ── Scientific expression entry ──────────────────────────────────────
    #[test]
    fn sci_expression_builds_and_evaluates() {
        let mut c = Calculator::new();
        c.set_mode(1);
        press(&mut c, &["2", "+", "3", "×", "4"]);     // live preview: 14
        assert_eq!(c.expression, "2+3×4");
        assert_eq!(c.display, "14");
        assert!(c.preview_active());
        press(&mut c, &["="]);
        assert_eq!(c.display, "14");
        assert_eq!(c.expression, "2+3×4 =");
        assert_eq!(c.history.len(), 1);
    }

    #[test]
    fn sci_parens_and_functions() {
        let mut c = Calculator::new();
        c.set_mode(1);
        // (2+3)×4 = 20 — note "sin" already opens its own paren
        press(&mut c, &["(", "2", "+", "3", ")", "×", "4", "="]);
        assert_eq!(c.display, "20");
        // 2·sin(30°) = 1
        let mut c = Calculator::new();
        c.set_mode(1);
        press(&mut c, &["2", "sin", "3", "0", ")", "="]);
        assert_eq!(c.display, "1");
    }

    #[test]
    fn sci_degree_mode_changes_trig_result() {
        let mut c = Calculator::new();
        c.set_mode(1);
        press(&mut c, &["sin", "9", "0", ")", "="]);
        assert_eq!(c.display, "1");            // DEG
        press(&mut c, &["DEG/RAD", "="]);      // re-evaluate in RAD
        let v: f64 = c.display.parse().unwrap();
        assert!((v - 0.894072).abs() < 0.001); // sin(90 rad)
    }

    #[test]
    fn sci_operator_replace_and_unary_minus() {
        let mut c = Calculator::new();
        c.set_mode(1);
        press(&mut c, &["5", "+", "×"]);      // 5 + × → 5 ×
        assert_eq!(c.expression, "5×");
        press(&mut c, &["2", "±"]);           // negate trailing operand
        assert_eq!(c.expression, "5×−2");
        press(&mut c, &["="]);
        assert_eq!(c.display, "-10");
    }

    #[test]
    fn sci_constants_and_factorial() {
        let mut c = Calculator::new();
        c.set_mode(1);
        press(&mut c, &["2", "π"]);           // implicit ×
        assert_eq!(c.expression, "2×π");
        press(&mut c, &["="]);
        let v: f64 = c.display.parse().unwrap();
        assert!((v - 6.28318530718).abs() < 1e-9);
        let mut c = Calculator::new();
        c.set_mode(1);
        press(&mut c, &["5", "x!", "="]);
        assert_eq!(c.display, "120");
    }

    #[test]
    fn sci_second_level_functions() {
        let mut c = Calculator::new();
        c.set_mode(1);
        // asin(1) = 90°
        press(&mut c, &["asin", "1", ")", "="]);
        assert_eq!(c.display, "90");
        let mut c = Calculator::new();
        c.set_mode(1);
        // 10ˣ: 10^(3) = 1000
        press(&mut c, &["10ˣ", "3", ")", "="]);
        assert_eq!(c.display, "1000");
        let mut c = Calculator::new();
        c.set_mode(1);
        // x²: 7² = 49
        press(&mut c, &["7", "x²", "="]);
        assert_eq!(c.display, "49");
    }

    #[test]
    fn sci_reciprocal_on_trailing_operand() {
        let mut c = Calculator::new();
        c.set_mode(1);
        press(&mut c, &["3", "+", "5"]);
        press(&mut c, &["1/x"]);              // 3+1/(5)
        assert_eq!(c.expression, "3+1/(5)");
        press(&mut c, &["="]);
        let v: f64 = c.display.parse().unwrap();
        assert!((v - 3.2).abs() < 1e-12);
    }

    #[test]
    fn sci_backspace_removes_function_and_edits_after_equals() {
        let mut c = Calculator::new();
        c.set_mode(1);
        press(&mut c, &["2", "+", "sin"]);
        assert_eq!(c.expression, "2+sin(");
        press(&mut c, &["⌫"]);                // whole "sin(" goes
        assert_eq!(c.expression, "2+");
        press(&mut c, &["3", "="]);
        assert_eq!(c.display, "5");
        press(&mut c, &["⌫"]);                // "2+3 =" → editable "2+3"
        assert_eq!(c.expression, "2+3");
    }

    #[test]
    fn sci_error_keeps_expression_for_retry() {
        let mut c = Calculator::new();
        c.set_mode(1);
        press(&mut c, &["5", "÷", "0", "="]);
        assert_eq!(c.display, "Error");
        // any key restarts
        press(&mut c, &["7"]);
        assert_eq!(c.expression, "7");
        assert_eq!(c.display, "7");
    }

    #[test]
    fn programmer_width_selector() {
        let mut c = Calculator::new();
        c.set_mode(3);
        press(&mut c, &["3", "0", "0"]);      // DEC QWORD: 300
        press(&mut c, &["WIDTH:8"]);
        assert_eq!(c.dec_text(), "44");       // 300 truncated to a byte
        assert_eq!(c.hex_text(), "2C");
        // Truncation is irreversible (standard behaviour): re-enter 300
        press(&mut c, &["AC", "3", "0", "0"]);
        press(&mut c, &["WIDTH:16"]);
        assert_eq!(c.dec_text(), "300");      // fits a word unchanged
    }

    #[test]
    fn programmer_pending_expr_shown() {
        let mut c = Calculator::new();
        c.set_mode(3);
        press(&mut c, &["BASE:HEX", "F", "AND"]);
        assert_eq!(c.expression, "F AND …");
    }
}
