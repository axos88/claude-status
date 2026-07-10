// Claude Code status line renderer.
//
// Reads the status-line JSON on stdin and prints a single line:
//   🤖 <family>  │  📁 <folder>  │  🧠 <ctx>  │  ⏳ <5h>  │  📅 <7d>
//
// Normal:  each usage metric shows a colored progress bar + %.
// Near/hit (>=90%): that metric's bar is replaced by a live countdown until its
//                   next reset (yellow approaching, red once hit). When ANY limit
//                   is near, the context bar is dropped too — only its % remains.
//
// This is a port of the original statusline.sh; behavior is intended to match it
// cell-for-cell (see tests/ for the differential fixtures).

use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{Local, TimeZone};
use serde_json::Value;

// --- ANSI palette ------------------------------------------------------------
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const CYAN: &str = "\x1b[36m";
const MAG: &str = "\x1b[35m";
const RED: &str = "\x1b[31m";
const YEL: &str = "\x1b[33m";
const GRN: &str = "\x1b[32m";
// per-second countdown pulse palettes
const BYEL: &str = "\x1b[93m"; // near mode: bright yellow <-> brown
const BROWN: &str = "\x1b[38;5;130m";
const DRED: &str = "\x1b[38;5;88m"; // hit mode: red <-> dark red

const SEP: &str = "  ";

/// Walk a dotted path of object keys, returning the leaf value if present.
fn dig<'a>(v: &'a Value, path: &[&str]) -> &'a Value {
    let mut cur = v;
    for k in path {
        cur = &cur[k];
    }
    cur
}

/// jq `path // default` for numbers: null/missing/non-numeric -> default.
fn num(v: &Value, path: &[&str], default: f64) -> f64 {
    dig(v, path).as_f64().unwrap_or(default)
}

/// jq `path // default` for strings.
fn text(v: &Value, path: &[&str], default: &str) -> String {
    dig(v, path)
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| default.to_owned())
}

/// threshold color for a percentage: green < 60, yellow < 85, red otherwise
fn color_for(pct: i64) -> &'static str {
    if pct >= 85 {
        RED
    } else if pct >= 60 {
        YEL
    } else {
        GRN
    }
}

/// colored progress bar: `bar(42)` -> "[████░░░░░░]  42%"
fn bar(pct: i64) -> String {
    let width: i64 = 10;
    let pct = pct.clamp(0, 100);
    let color = color_for(pct);
    let filled = (pct * width + 50) / 100; // integer rounding, matches bash
    let mut cells = String::from(color);
    for _ in 0..filled {
        cells.push('█');
    }
    cells.push_str(DIM);
    for _ in filled..width {
        cells.push('░');
    }
    format!("{RESET}[{cells}{RESET}] {color}{pct:3}%{RESET}")
}

/// time left, single most-significant unit: "5d" / "2h" / "50m" / "9s"
fn fmt_top(mut s: i64) -> String {
    if s < 0 {
        s = 0;
    }
    let d = s / 86400;
    let h = (s % 86400) / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if d > 0 {
        format!("{d}d")
    } else if h > 0 {
        format!("{h}h")
    } else if m > 0 {
        format!("{m}m")
    } else {
        format!("{sec}s")
    }
}

/// time left, two most-significant units: "5d 2h" / "2h 1m" / "50m 13s" / "9s"
fn fmt_left(mut s: i64) -> String {
    if s < 0 {
        s = 0;
    }
    let d = s / 86400;
    let h = (s % 86400) / 3600;
    let m = (s % 3600) / 60;
    let sec = s % 60;
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {sec}s")
    } else {
        format!("{sec}s")
    }
}

/// usage metric:
///   normal (<90%) -> colored progress bar
///   near (90-99%) -> percentage + live countdown until reset
///   hit  (100%)   -> red error emoji + countdown (no percentage)
fn metric(label: &str, pct: i64, resets_at: i64, now: i64) -> String {
    let mut cd = String::new();
    let mut clock = String::new();
    if resets_at > 0 {
        cd = fmt_left(resets_at - now);
        clock = Local
            .timestamp_opt(resets_at, 0)
            .single()
            .map(|dt| dt.format("%H:%M").to_string())
            .unwrap_or_default();
    }

    // alternate colors each second so feedback is perceived even when the
    // displayed digits don't change (e.g. in the "1h 23m" range)
    if pct >= 100 {
        let blink = if now % 2 == 0 { RED } else { DRED };
        let mut tail = String::new();
        if !cd.is_empty() {
            tail = format!(" {blink}{cd}{RESET}");
        }
        if !clock.is_empty() {
            tail.push_str(&format!(" {DIM}@ {clock}{RESET}"));
        }
        format!("{DIM}{label}{RESET} ⛔{tail}")
    } else if pct >= 90 {
        let blink = if now % 2 == 0 { BYEL } else { BROWN };
        let mut tail = String::new();
        if !cd.is_empty() {
            tail = format!(" {blink}{cd}{RESET}");
        }
        format!("{DIM}{label}{RESET} {YEL}{pct}%{RESET}{tail}")
    } else {
        // normal mode: reset countdown (single highest-magnitude unit) sits
        // between the icon and the bar, colored to match the bar/percentage
        let cd = if resets_at > 0 {
            format!("{}{}{RESET} ", color_for(pct), fmt_top(resets_at - now))
        } else {
            String::new()
        };
        format!("{DIM}{label}{RESET} {cd}{}", bar(pct))
    }
}

/// model family only — drop version/context suffix (e.g. "Opus 4.8" -> "Opus")
fn model_family(name: &str) -> String {
    if name.contains("Opus") {
        "Opus".into()
    } else if name.contains("Sonnet") {
        "Sonnet".into()
    } else if name.contains("Haiku") {
        "Haiku".into()
    } else if name.contains("Fable") {
        "Fable".into()
    } else {
        // ${MODEL%% *} — first whitespace-delimited word
        name.split(' ').next().unwrap_or(name).to_owned()
    }
}

fn folder_of(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| cwd.to_owned())
}

fn render(v: &Value, now: i64) -> String {
    let model = model_family(&text(v, &["model", "display_name"], "?"));

    // .workspace.current_dir // .cwd // "?"
    let cwd = {
        let ws = text(v, &["workspace", "current_dir"], "");
        if !ws.is_empty() {
            ws
        } else {
            let top = text(v, &["cwd"], "");
            if top.is_empty() {
                "?".to_owned()
            } else {
                top
            }
        }
    };
    let folder = folder_of(&cwd);

    let ctx = num(v, &["context_window", "used_percentage"], 0.0) as i64;
    let five = num(v, &["rate_limits", "five_hour", "used_percentage"], -1.0) as i64;
    let five_reset = num(v, &["rate_limits", "five_hour", "resets_at"], 0.0) as i64;
    let seven = num(v, &["rate_limits", "seven_day", "used_percentage"], -1.0) as i64;
    let seven_reset = num(v, &["rate_limits", "seven_day", "resets_at"], 0.0) as i64;

    let mut out = format!("{BOLD}{MAG}🤖 {model}{RESET}{SEP}{CYAN}📁 {folder}{RESET}{SEP}");

    // context: bar normally, but only the % if any limit is near
    if five >= 90 || seven >= 90 {
        // raw ctx, unclamped — matches the bash near-context branch
        out.push_str(&format!("🧠 {}{ctx}%{RESET}", color_for(ctx)));
    } else {
        out.push_str(&format!("🧠 {}", bar(ctx)));
    }

    if five >= 0 {
        out.push_str(&format!("{SEP}{}", metric("⏳", five, five_reset, now)));
    }
    if seven >= 0 {
        out.push_str(&format!("{SEP}{}", metric("📅", seven, seven_reset, now)));
    }

    out
}

fn main() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // A malformed payload shouldn't blank the status line — fall back to `{}`
    // so every field takes its default.
    let v: Value = serde_json::from_str(&input).unwrap_or(Value::Null);

    print!("{}", render(&v, now));
}
