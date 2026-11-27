//! Output helpers that keep `--json` stdout clean.

use comfy_table::presets::UTF8_FULL_CONDENSED;
use comfy_table::{ContentArrangement, Table};
use overtone_core::CoreError;
use serde::Serialize;

pub struct Output {
    json: bool,
}

impl Output {
    pub fn new(json: bool) -> Self {
        Self { json }
    }

    pub fn is_json(&self) -> bool {
        self.json
    }

    /// Prints `value` as JSON in JSON mode, otherwise runs `human`.
    pub fn emit<T: Serialize>(&self, value: &T, human: impl FnOnce()) {
        if self.json {
            println!(
                "{}",
                serde_json::to_string_pretty(value).expect("output types serialize")
            );
        } else {
            human();
        }
    }

    pub fn error(&self, err: &CoreError) {
        if self.json {
            let payload = serde_json::json!({
                "error": { "code": err.code(), "message": err.to_string() }
            });
            println!("{payload}");
        } else {
            eprintln!("error: {err}");
        }
    }
}

/// A table sized to the terminal, with the house style.
pub fn table(header: &[&str]) -> Table {
    let mut table = Table::new();
    table
        .load_style(UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(header.iter().copied());
    table
}

/// `value` or an empty cell.
pub fn or_blank<T: ToString>(value: Option<T>) -> String {
    value.map_or_else(String::new, |v| v.to_string())
}
