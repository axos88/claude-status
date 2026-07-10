// Thin entry point: read the status JSON on stdin, render it, print it.
// All logic (and its tests) lives in the library crate — see src/lib.rs.

use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn main() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // A malformed payload shouldn't blank the status line — fall back to Null
    // so every field takes its default.
    let v: Value = serde_json::from_str(&input).unwrap_or(Value::Null);

    print!("{}", claude_status::render(&v, now));
}
