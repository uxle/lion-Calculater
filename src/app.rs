//! Slint ↔ Calculator bridge.
//! Owns the UI handle, wires all callbacks, and pushes state on every change.

use std::{cell::{Cell, RefCell}, rc::Rc};
use slint::{ComponentHandle, SharedString, VecModel};

use crate::{calculator::Calculator, MainWindow, HistoryEntry};

/// Launch with the default (Basic) mode.
pub fn run() -> Result<(), slint::PlatformError> {
    run_with_mode(0)
}

/// Launch directly into a given mode (`lion-calculator scientific`).
pub fn run_with_mode(mode: i32) -> Result<(), slint::PlatformError> {
    let ui   = MainWindow::new()?;
    let calc = Rc::new(RefCell::new(Calculator::new()));

    // History model rebuild bookkeeping (u64::MAX forces the first build).
    let hist_ver = Rc::new(Cell::new(u64::MAX));

    // Apply the requested start-up mode before the first paint.
    if mode != 0 {
        calc.borrow_mut().set_mode(mode);
        ui.set_current_mode(mode);
    }

    // Push initial state
    push_display(&ui, &calc.borrow(), &hist_ver);

    // button-pressed ----------------------------------------------------------
    {
        let ui_w = ui.as_weak();
        let c2   = calc.clone();
        let hv   = hist_ver.clone();
        ui.on_button_pressed(move |label| {
            c2.borrow_mut().handle_input(label.as_str());
            let Some(ui) = ui_w.upgrade() else { return };
            push_display(&ui, &c2.borrow(), &hv);
        });
    }

    // mode-changed ------------------------------------------------------------
    {
        let ui_w = ui.as_weak();
        let c2   = calc.clone();
        let hv   = hist_ver.clone();
        ui.on_mode_changed(move |mode| {
            c2.borrow_mut().set_mode(mode);
            let Some(ui) = ui_w.upgrade() else { return };
            ui.set_current_mode(mode);
            push_display(&ui, &c2.borrow(), &hv);
        });
    }

    // history-clicked ---------------------------------------------------------
    {
        let ui_w = ui.as_weak();
        let c2   = calc.clone();
        let hv   = hist_ver.clone();
        ui.on_history_clicked(move |idx| {
            let Some(ui) = ui_w.upgrade() else { return };
            let mut c = c2.borrow_mut();

            // Copy out first so the history borrow ends before mutating.
            let Some(entry) = c.history.get(idx as usize) else { return };
            let (expr, result) = (entry.expr.clone(), entry.result.clone());

            // Recall as a value — replaying the string char-by-char through
            // handle_input corrupted negative and scientific-notation results.
            c.recall_from_history(&expr, &result);

            push_display(&ui, &c, &hv);
        });
    }

    // history-cleared ---------------------------------------------------------
    {
        let ui_w = ui.as_weak();
        let c2   = calc.clone();
        let hv   = hist_ver.clone();
        ui.on_history_cleared(move || {
            c2.borrow_mut().history.clear();
            let Some(ui) = ui_w.upgrade() else { return };
            push_display(&ui, &c2.borrow(), &hv);
        });
    }

    ui.run()
}

fn push_display(ui: &MainWindow, c: &Calculator, hv: &Cell<u64>) {
    ui.set_display_text(ss(&c.display));
    ui.set_expr_text(ss(&c.expression));
    ui.set_deg_mode(c.degree_mode);
    ui.set_display_preview(c.preview_active());
    ui.set_mem_indicator(if c.memory.has_value() { ss("M") } else { ss("") });

    // Programmer displays
    ui.set_active_base(ss(c.prog.base.name()));
    ui.set_active_width(ss(c.prog.width_name()));
    ui.set_hex_text(ss(&c.hex_text()));
    ui.set_dec_text(ss(&c.dec_text()));
    ui.set_oct_text(ss(&c.oct_text()));
    ui.set_bin_text(ss(&c.bin_text()));

    // Financial TVM
    ui.set_tvm_n(ss(&c.tvm.n_str()));
    ui.set_tvm_iy(ss(&c.tvm.iy_str()));
    ui.set_tvm_pv(ss(&c.tvm.pv_str()));
    ui.set_tvm_pmt(ss(&c.tvm.pmt_str()));
    ui.set_tvm_fv(ss(&c.tvm.fv_str()));
    ui.set_tvm_sel(ss(&c.tvm_sel));

    // History — rebuild the model only when the store actually changed.
    // This used to rebuild up to 100 entries on every single keypress.
    if hv.get() != c.history.version() {
        hv.set(c.history.version());
        let model = Rc::new(VecModel::<HistoryEntry>::default());
        for rec in c.history.all() {
            model.push(HistoryEntry {
                expr:   ss(&rec.expr),
                result: ss(&rec.result),
                ts:     ss(&rec.ts),
            });
        }
        ui.set_history(model.into());
    }
}

#[inline]
fn ss(s: &str) -> SharedString { SharedString::from(s) }
