// Thin entry point: read the status JSON on stdin, render it, print it.
// All logic (and its tests) lives in the library crate — see src/lib.rs.

use std::io::{Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use claude_status::update;
use serde_json::Value;

fn main() {
    // the background update, started by `update::spawn_if_due` below
    if std::env::args().nth(1).as_deref() == Some(update::FLAG) {
        update::run_and_log();
        return;
    }

    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // A malformed payload shouldn't blank the status line — fall back to Null
    // so every field takes its default.
    let v: Value = serde_json::from_str(&input).unwrap_or(Value::Null);

    // which config dir this session runs from — Claude Code sets it only for a
    // non-default one, so "unset" is a valid answer, not an error
    let env_dir = std::env::var("CLAUDE_CONFIG_DIR").ok();
    let home = std::env::var("HOME").ok();
    let dir = claude_status::config_dir(env_dir.as_deref(), &v, home.as_deref());
    let file = dir
        .as_ref()
        .and_then(|d| std::fs::read_to_string(d.join(claude_status::ACCOUNT_FILE)).ok());
    let account = claude_status::account_name(dir.as_deref(), file.as_deref());

    // Claude Code passes the terminal width in COLUMNS
    let columns = std::env::var("COLUMNS").ok().and_then(|c| c.parse().ok());

    print!("{}", claude_status::render(&v, now, &account, columns));
    let _ = std::io::stdout().flush();

    update::spawn_if_due();
}
