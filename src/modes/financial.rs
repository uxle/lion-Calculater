//! Time-Value-of-Money solver and basic financial utilities.
//!
//! Standard TVM equation (end-of-period payments, ordinary annuity):
//!   PV·(1+i)^N  +  PMT·[(1+i)^N − 1]/i  +  FV  =  0
//!
//! where  i = I/Y / 100   (periodic rate already expressed as a decimal)

#[derive(Debug, Clone, Default)]
pub struct TvmState {
    pub n:   Option<f64>,
    pub iy:  Option<f64>,
    pub pv:  Option<f64>,
    pub pmt: Option<f64>,
    pub fv:  Option<f64>,
}

impl TvmState {
    pub fn new() -> Self { Self::default() }
    pub fn clear(&mut self) { *self = Self::default(); }

    // Display strings ---------------------------------------------------------
    pub fn n_str(&self)   -> String { fmt(self.n)   }
    pub fn iy_str(&self)  -> String { fmt(self.iy)  }
    pub fn pv_str(&self)  -> String { fmt(self.pv)  }
    pub fn pmt_str(&self) -> String { fmt(self.pmt) }
    pub fn fv_str(&self)  -> String { fmt(self.fv)  }

    // Solver ------------------------------------------------------------------
    pub fn compute(&self, target: &str) -> Result<f64, String> {
        match target {
            "FV"  => self.solve_fv(),
            "PV"  => self.solve_pv(),
            "PMT" => self.solve_pmt(),
            "N"   => self.solve_n(),
            "I/Y" => self.solve_iy(),
            _     => Err(format!("Unknown TVM target: {target}")),
        }
    }

    // FV = -(PV·(1+i)^N + PMT·((1+i)^N-1)/i)
    fn solve_fv(&self) -> Result<f64, String> {
        let (n, i, pv, pmt) = self.need("FV", &["N","I/Y","PV","PMT"])?;
        let r = valid_rate(i)?;
        let fv = if r == 0.0 {
            -(pv + pmt * n)
        } else {
            let f = (1.0 + r).powf(n);
            -(pv * f + pmt * (f - 1.0) / r)
        };
        Ok(r12(fv))
    }

    // PV = -(FV + PMT·((1+i)^N-1)/i) / (1+i)^N
    fn solve_pv(&self) -> Result<f64, String> {
        let (n, i, pmt, fv) = self.need("PV", &["N","I/Y","PMT","FV"])?;
        let r = valid_rate(i)?;
        let pv = if r == 0.0 {
            -(fv + pmt * n)
        } else {
            let f = (1.0 + r).powf(n);
            -(fv + pmt * (f - 1.0) / r) / f
        };
        Ok(r12(pv))
    }

    // PMT = -r·(PV·(1+i)^N + FV) / ((1+i)^N - 1)
    fn solve_pmt(&self) -> Result<f64, String> {
        let (n, i, pv, fv) = self.need("PMT", &["N","I/Y","PV","FV"])?;
        let r = valid_rate(i)?;
        let pmt = if r == 0.0 {
            if n == 0.0 { return Err("N cannot be zero".into()); }
            -(pv + fv) / n
        } else {
            let f = (1.0 + r).powf(n);
            -r * (pv * f + fv) / (f - 1.0)
        };
        Ok(r12(pmt))
    }

    fn solve_n(&self) -> Result<f64, String> {
        let (i, pv, pmt, fv) = self.need("N", &["I/Y","PV","PMT","FV"])?;
        let r = valid_rate(i)?;
        let n = if r == 0.0 {
            if pmt == 0.0 { return Err("PMT cannot be zero when I/Y = 0".into()); }
            -(pv + fv) / pmt
        } else {
            // f = (1+r)^N = (PMT − FV·r) / (PMT + PV·r)
            // Only the RATIO must be positive — requiring both terms positive
            // (as before) wrongly rejected standard loan/deposit patterns.
            let a = pmt - fv * r;
            let b = pmt + pv * r;
            if a == 0.0 || b == 0.0 {
                return Err("Cannot solve N: check inputs".into());
            }
            let ratio = a / b;
            if ratio <= 0.0 {
                return Err("Cannot solve N: check signs of PV / FV / PMT".into());
            }
            ratio.ln() / (1.0 + r).ln()
        };
        if !n.is_finite() || n < 0.0 {
            return Err("No valid solution for N".into());
        }
        Ok(r12(n))
    }

    // Newton-Raphson for I/Y (transcendental)
    fn solve_iy(&self) -> Result<f64, String> {
        let (n, pv, pmt, fv) = self.need("I/Y", &["N","PV","PMT","FV"])?;
        if n <= 0.0 { return Err("N must be positive".into()); }

        // Special case: zero PMT → closed form
        if pmt.abs() < 1e-14 {
            if pv == 0.0 { return Err("PV cannot be zero".into()); }
            let ratio = -fv / pv;
            if ratio <= 0.0 { return Err("Cannot solve I/Y: negative ratio".into()); }
            let v = (ratio.powf(1.0 / n) - 1.0) * 100.0;
            if !v.is_finite() { return Err("No valid solution for I/Y".into()); }
            return Ok(r12(v));
        }

        let f  = |r: f64| -> f64 {
            let factor = (1.0 + r).powf(n);
            pv * factor + pmt * (factor - 1.0) / r + fv
        };
        let df = |r: f64| -> f64 {
            let factor = (1.0 + r).powf(n);
            n * pv * factor / (1.0 + r)
                + pmt * (n * factor / (1.0 + r) * r - (factor - 1.0)) / (r * r)
        };

        // Residual tolerance scaled to the magnitude of the cash flows, so
        // large PV/FV values do not fake convergence (or never reach it).
        let scale = (pv.abs() + fv.abs() + pmt.abs() * n).max(1.0);
        let tol   = 1e-9 * scale;

        // Multi-start Newton-Raphson: a single 0.1 seed could NaN-out or land
        // in a basin that never converges for adversarial sign patterns.
        for &guess in &[0.1_f64, 0.02, 0.005, 0.5, 2.0, -0.25, -0.75] {
            let mut r = guess;
            for _ in 0..200 {
                let fr = f(r);
                if !fr.is_finite() { break; }
                if fr.abs() <= tol && r > -1.0 {
                    return Ok(r12(r * 100.0));
                }
                let dfr = df(r);
                if !dfr.is_finite() || dfr.abs() < 1e-15 { break; }
                let r2 = (r - fr / dfr).clamp(-0.9999, 100.0);
                if !r2.is_finite() { break; }
                if (r2 - r).abs() < 1e-12 { r = r2; break; }
                r = r2;
            }
            // Accept the point if the residual is small enough.
            if r > -1.0 && f(r).abs() <= tol {
                return Ok(r12(r * 100.0));
            }
        }
        Err("I/Y solver did not converge — check inputs".into())
    }

    // Helper: extract exactly 4 named TVM values
    fn need(&self, solving: &str, names: &[&str; 4]) -> Result<(f64,f64,f64,f64), String> {
        let keys = ["N","I/Y","PV","PMT","FV"];
        let vals = [self.n, self.iy, self.pv, self.pmt, self.fv];
        let mut out = [0f64; 4];
        for (j, &name) in names.iter().enumerate() {
            let idx = keys.iter().position(|&k| k == name)
                .ok_or_else(|| format!("Unknown key '{name}'"))?;
            out[j] = vals[idx].ok_or_else(||
                format!("{name} is not set (needed to solve {solving})"))?;
        }
        Ok((out[0], out[1], out[2], out[3]))
    }
}

/// Convert I/Y percent to a decimal rate, rejecting r ≤ −1 where the
/// compounding factor (1+r)^N degenerates (0 or NaN territory).
fn valid_rate(i: f64) -> Result<f64, String> {
    let r = i / 100.0;
    if r <= -1.0 { Err("I/Y must be greater than -100%".into()) } else { Ok(r) }
}

fn fmt(v: Option<f64>) -> String {
    v.map(crate::parser::fmt_num).unwrap_or_else(|| "?".into())
}

fn r12(v: f64) -> f64 { (v * 1e10).round() / 1e10 }

// ── Business helpers ──────────────────────────────────────────────────────

/// Add tax:    total = base × (1 + rate/100)
pub fn tax_add(base: f64, rate: f64) -> f64 { r12(base * (1.0 + rate / 100.0)) }

/// Remove tax: base  = total / (1 + rate/100)
pub fn tax_remove(total: f64, rate: f64) -> f64 { r12(total / (1.0 + rate / 100.0)) }

/// Gross margin %: (price − cost) / price × 100
pub fn margin_pct(cost: f64, price: f64) -> Result<f64, String> {
    if price == 0.0 { return Err("Price cannot be zero".into()); }
    Ok(r12((price - cost) / price * 100.0))
}

// ── Tests ─────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    fn tvm(n: f64, iy: f64, pv: f64, pmt: f64) -> TvmState {
        TvmState { n: Some(n), iy: Some(iy), pv: Some(pv), pmt: Some(pmt), fv: None }
    }

    #[test]
    fn fv_compound_growth() {
        // 1000 invested at 5% for 10 periods → FV ≈ 1628.894627
        let t = tvm(10.0, 5.0, -1000.0, 0.0);
        let fv = t.compute("FV").unwrap();
        assert!((fv - 1_628.894_627).abs() < 0.001, "FV = {fv}");
    }

    #[test]
    fn fv_zero_rate() {
        let t = tvm(10.0, 0.0, -1000.0, -100.0);
        let fv = t.compute("FV").unwrap();
        assert_eq!(fv, 2000.0); // 1000 + 100×10
    }

    #[test]
    fn pmt_of_loan() {
        // 200k loan, 6%/yr … using period-consistent inputs directly
        let t = TvmState {
            n: Some(360.0), iy: Some(0.5), pv: Some(-200_000.0),
            pmt: None, fv: Some(0.0),
        };
        let pmt = t.compute("PMT").unwrap();
        assert!((pmt - 1199.10).abs() < 0.01, "PMT = {pmt}");
    }

    #[test]
    fn solve_n_loan_pattern() {
        // Receive 10_000, repay 500/period at 1% → N = ln(1.25)/ln(1.01) ≈ 22.4
        // (the old strict sign check rejected this classic pattern)
        let t = TvmState {
            n: None, iy: Some(1.0), pv: Some(10_000.0),
            pmt: Some(-500.0), fv: Some(0.0),
        };
        let n = t.compute("N").unwrap();
        assert!(n > 22.0 && n < 23.0, "N = {n}");
    }

    #[test]
    fn solve_iy_roundtrip() {
        // Grow −1000 → 1628.894627 in 10 periods → I/Y = 5%
        let t = TvmState {
            n: Some(10.0), iy: None, pv: Some(-1000.0),
            pmt: Some(0.0), fv: Some(1_628.894_626_877),
        };
        let iy = t.compute("I/Y").unwrap();
        assert!((iy - 5.0).abs() < 1e-6, "I/Y = {iy}");
    }

    #[test]
    fn solve_iy_with_payments() {
        // Loan: PV=+1000 received, PMT=−100/period, FV=0, N=10 → I/Y ≈ 0?
        // 1000 = 100·a(10, i) → a = 10 → i = 0
        let t = TvmState {
            n: Some(10.0), iy: None, pv: Some(1000.0),
            pmt: Some(-100.0), fv: Some(0.0),
        };
        let iy = t.compute("I/Y").unwrap();
        assert!(iy.abs() < 1e-6, "I/Y = {iy}");
    }

    #[test]
    fn missing_slot_errors() {
        let t = TvmState::new(); // nothing set
        assert!(t.compute("FV").is_err());
    }

    #[test]
    fn rate_below_minus_100_rejected() {
        let t = tvm(10.0, -150.0, -1000.0, 0.0);
        let err = t.compute("FV").unwrap_err();
        assert!(err.contains("I/Y"), "{err}");
    }

    #[test]
    fn tax_helpers() {
        assert_eq!(tax_add(100.0, 20.0), 120.0);
        assert_eq!(tax_remove(120.0, 20.0), 100.0);
        assert_eq!(tax_add(80.0, 12.5), 90.0);
        assert!((tax_add(99.99, 8.25) - 108.239_175).abs() < 1e-6);
    }

    #[test]
    fn margin() {
        assert_eq!(margin_pct(60.0, 100.0).unwrap(), 40.0);
        assert!(margin_pct(100.0, 0.0).is_err());
    }
}
