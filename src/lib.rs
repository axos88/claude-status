// Claude Code status line renderer.
//
// `render` turns the status-line JSON into a single line:
//   🤖 <family>  📁 <folder>  🧠 <ctx>  ⏳ <5h>  📅 <7d>
//
// Each usage metric renders as:  icon  reset-countdown  progress-bar  pct%,
// all sharing one threshold color (green <=75, yellow 76-90, red >90). The
// countdown shows two units in the red zone (one unit below it), pulses
// red<->dark-red at >=95%, and at >=100% the bar is dropped, leaving
// "⛔ <pct>% <countdown>". When ANY limit is red (>90), the context bar
// collapses to just its %.
//
// `render` is pure over (JSON, now) so the tests at the bottom exercise every
// branch deterministically with a fixed clock.

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
// per-second countdown pulse: red <-> dark red, when >= 95%
const DRED: &str = "\x1b[38;5;88m";

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

/// threshold color for a percentage: green ≤ 75, yellow 76–90, red > 90
fn color_for(pct: i64) -> &'static str {
    if pct > 90 {
        RED
    } else if pct > 75 {
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
    let filled = (pct * width + 50) / 100; // integer rounding
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

/// One usage metric. Color scales with `pct` (green/yellow/red); the reset
/// countdown shows two units in the red zone and one unit below it, pulses
/// red<->dark-red at >=95%, and at >=100% the bar/% give way to a ⛔.
fn metric(label: &str, pct: i64, resets_at: i64, now: i64) -> String {
    let color = color_for(pct);
    // countdown color: pulse red<->dark-red each second once near the ceiling,
    // so motion is perceived even when the displayed digits don't change
    let cd_color = if pct >= 95 {
        if now % 2 == 0 {
            RED
        } else {
            DRED
        }
    } else {
        color
    };
    // countdown text: two units in the red zone (>90), one unit below it
    let cd = if resets_at > 0 {
        let secs = resets_at - now;
        let txt = if pct > 90 {
            fmt_left(secs)
        } else {
            fmt_top(secs)
        };
        format!("{cd_color}{txt}{RESET}")
    } else {
        String::new()
    };

    if pct >= 100 {
        // over the limit: no bar — ⛔ flag, percentage, then the countdown
        let tail = if cd.is_empty() {
            String::new()
        } else {
            format!(" {cd}")
        };
        format!("{DIM}{label}{RESET} ⛔ {color}{pct}%{RESET}{tail}")
    } else {
        // countdown sits between the icon and the bar, sharing its color
        let lead = if cd.is_empty() {
            String::new()
        } else {
            format!("{cd} ")
        };
        format!("{DIM}{label}{RESET} {lead}{}", bar(pct))
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
        // first whitespace-delimited word
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

/// Render the full status line from the status JSON and the current unix time.
pub fn render(v: &Value, now: i64) -> String {
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

    // context: bar normally, but only the % if any limit is in the red zone
    if five > 90 || seven > 90 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Fixed clock (even second). FAR is far enough out that the countdown is a
    // stable "4d" (one unit) / "4d 15h" (two units) regardless of small deltas.
    const NOW: i64 = 1_000_000_000;
    const FAR: i64 = NOW + 400_000; // 4d 15h 6m 40s

    /// Remove ANSI SGR escapes so layout can be asserted without color noise.
    fn strip(s: &str) -> String {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for c2 in chars.by_ref() {
                    if c2 == 'm' {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    /// A single five-hour metric at `pct`, resetting at FAR, plus model/ctx.
    fn with_five(pct: i64, reset: i64) -> Value {
        json!({
            "model": {"display_name": "Opus 4.8"},
            "context_window": {"used_percentage": 40},
            "rate_limits": {"five_hour": {"used_percentage": pct, "resets_at": reset}}
        })
    }

    #[test]
    fn color_thresholds() {
        assert_eq!(color_for(0), GRN);
        assert_eq!(color_for(75), GRN); // 75 stays green
        assert_eq!(color_for(76), YEL); // 76 first yellow
        assert_eq!(color_for(90), YEL); // 90 stays yellow
        assert_eq!(color_for(91), RED); // 91 first red ("over 90")
        assert_eq!(color_for(200), RED);
    }

    #[test]
    fn countdown_formatting() {
        assert_eq!(fmt_top(400_000), "4d");
        assert_eq!(fmt_left(400_000), "4d 15h");
        assert_eq!(fmt_top(7200), "2h");
        assert_eq!(fmt_left(3661), "1h 1m");
        assert_eq!(fmt_top(45), "45s");
        assert_eq!(fmt_left(-5), "0s"); // clamps negatives
    }

    #[test]
    fn full_line_green_zone() {
        let s = strip(&render(&with_five(30, FAR), NOW));
        assert_eq!(
            s,
            "🤖 Opus  📁 ?  🧠 [████░░░░░░]  40%  ⏳ 4d [███░░░░░░░]  30%"
        );
    }

    #[test]
    fn green_countdown_is_one_unit_and_green() {
        let out = render(&with_five(30, FAR), NOW);
        assert!(strip(&out).contains("⏳ 4d [")); // one unit, then the bar
        assert!(out.contains(&format!("{GRN}4d{RESET}"))); // colored to match
    }

    #[test]
    fn yellow_zone_76_to_90() {
        for pct in [76, 80, 90] {
            let out = render(&with_five(pct, FAR), NOW);
            assert!(strip(&out).contains("⏳ 4d [")); // still one unit
            assert!(out.contains(&format!("{YEL}4d{RESET}")));
        }
    }

    #[test]
    fn red_zone_switches_to_two_units_and_stays_static_below_95() {
        let out = render(&with_five(92, FAR), NOW);
        assert!(strip(&out).contains("⏳ 4d 15h [")); // two units, bar present
        assert!(out.contains(&format!("{RED}4d 15h{RESET}")));
        assert!(!out.contains(DRED)); // 92 < 95 -> not flashing
    }

    #[test]
    fn flashing_at_95_pulses_on_odd_seconds() {
        // bar is still present in the 95–99 band
        assert!(strip(&render(&with_five(95, FAR), NOW)).contains("⏳ 4d 15h ["));
        // even second -> bright red, no dark-red anywhere
        assert!(!render(&with_five(95, FAR), NOW).contains(DRED));
        // odd second -> countdown pulses to dark red
        assert!(render(&with_five(95, FAR), NOW + 1).contains(DRED));
    }

    #[test]
    fn over_limit_drops_bar_shows_flag_and_percent() {
        let s = strip(&render(&with_five(100, FAR), NOW));
        assert!(s.contains("⏳ ⛔ 100% 4d 15h")); // ⛔, %, then countdown
        assert!(!s.contains('[')); // no bar anywhere (ctx also collapses)
        // and >135% keeps rendering the real percentage
        assert!(strip(&render(&with_five(135, FAR), NOW)).contains("⛔ 135%"));
    }

    #[test]
    fn context_collapses_when_a_limit_is_red() {
        let v = json!({
            "model": {"display_name": "Opus"},
            "context_window": {"used_percentage": 96},
            "rate_limits": {"five_hour": {"used_percentage": 92, "resets_at": FAR}}
        });
        let s = strip(&render(&v, NOW));
        assert!(s.contains("🧠 96%")); // raw percentage, no bar
        assert!(!s.contains("🧠 [")); // bar collapsed away
    }

    #[test]
    fn context_keeps_bar_at_exactly_90() {
        // 90 is yellow, not red -> no collapse
        let s = strip(&render(&with_five(90, FAR), NOW));
        assert!(s.contains("🧠 [████░░░░░░]  40%"));
    }

    #[test]
    fn absent_metric_is_omitted_present_one_shown() {
        // only seven_day present
        let v = json!({
            "model": {"display_name": "Sonnet 5"},
            "context_window": {"used_percentage": 33},
            "rate_limits": {"seven_day": {"used_percentage": 50, "resets_at": FAR}}
        });
        let s = strip(&render(&v, NOW));
        assert!(s.contains("📅 4d ["));
        assert!(!s.contains('⏳')); // five_hour absent -> omitted
    }

    #[test]
    fn no_reset_means_no_countdown() {
        let s = strip(&render(&with_five(20, 0), NOW));
        assert!(s.contains("⏳ [")); // bar immediately after icon, no countdown
    }

    #[test]
    fn model_family_and_cwd_fallback() {
        assert!(strip(&render(
            &json!({"model": {"display_name": "Haiku 4.5"}}),
            NOW
        ))
        .contains("🤖 Haiku"));
        // top-level cwd fallback + basename
        let s = strip(&render(&json!({"cwd": "/var/log/app"}), NOW));
        assert!(s.contains("📁 app"));
    }

    #[test]
    fn malformed_input_degrades_to_defaults() {
        // Value::Null is what main() falls back to on unparseable stdin
        let s = strip(&render(&Value::Null, NOW));
        assert_eq!(s, "🤖 ?  📁 ?  🧠 [░░░░░░░░░░]   0%");
    }
}
