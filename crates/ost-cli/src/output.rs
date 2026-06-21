//! Output rendering helpers.
//!
//! Every command can render for a human terminal or as JSON (§13.2, §18.3).
//! Keeping this in one place ensures consistent error shapes and exit behavior.

/// Selected output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Human,
    Json,
}

impl Format {
    pub fn from_flag(json: bool) -> Format {
        if json {
            Format::Json
        } else {
            Format::Human
        }
    }

    pub fn is_json(self) -> bool {
        matches!(self, Format::Json)
    }
}

/// Render an error, matching the active format.
pub fn error(err: &ost_core::Error, fmt: Format) {
    match fmt {
        Format::Json => {
            let body = serde_json::json!({ "error": err.to_string() });
            eprintln!(
                "{}",
                serde_json::to_string_pretty(&body).unwrap_or_else(|_| err.to_string())
            );
        }
        Format::Human => {
            eprintln!("error: {err}");
        }
    }
}

/// Print a value as pretty JSON to stdout.
pub fn json(value: &serde_json::Value) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("error: failed to serialize JSON: {e}"),
    }
}
