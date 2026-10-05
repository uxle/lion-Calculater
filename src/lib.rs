// Lion Calculator — library root.
// Shared by the native binary (src/main.rs) and the WebAssembly build
// (web/ target: `cargo build --target wasm32-unknown-unknown --profile wasm-release`).
//
// The crate is exposed as both an rlib (for the native binary and tests)
// and a cdylib (for wasm-bindgen to generate JS glue).

pub mod app;
pub mod calculator;
pub mod history;
pub mod memory;
pub mod modes;
pub mod parser;

slint::include_modules!();

/// WebAssembly entry point.
///
/// Called explicitly from the generated JS glue (`run_app()` after `init()`)
/// so the startup order is deterministic across wasm-bindgen versions.
#[cfg(target_arch = "wasm32")]
pub mod wasm {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn run_app() {
        crate::app::run().unwrap();
    }
}
