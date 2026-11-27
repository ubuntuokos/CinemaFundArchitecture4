//! Print the engine's command registry as JSON: one object per command with its `id`, `label`,
//! menu path (`menu`, empty when the command is in no menu) and `journal` (false for pure
//! queries). `cargo xtask parity --area commands` reads this to measure command coverage against a
//! reference list (docs/gaps.md G7).
//!
//! ```sh
//! cargo run -p cfa4-engine --example command_list
//! ```

use std::process::ExitCode;

use serde_json::json;

fn main() -> ExitCode {
    let items: Vec<serde_json::Value> =
        cfa4_engine::command_specs().iter().map(|c| json!({"id": c.id, "label": c.label, "menu": c.menu, "journal": c.journal})).collect();
    match serde_json::to_string_pretty(&items) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("command_list: {e}");
            ExitCode::FAILURE
        }
    }
}
