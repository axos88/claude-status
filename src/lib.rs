// Claude Code status line renderer.
//
// `render` turns the status-line JSON into one line:
//   🤖 <family>  📁 <folder>  👤 <account>  🧠 <ctx>  ⏳ <5h>  📅 <7d>
// the limits only when Claude Code reports them. Where that leaves a bar under
// ONE_LINE_BAR cells, the model goes; where that doesn't help either, the
// limits move to a second line:
//   🤖 <family>  📁 <folder>  👤 <account>  🧠 <ctx>
//   ⏳ <5h>  📅 <7d>
//
// Each usage metric renders as:  icon  reset-countdown  progress-bar  pct%,
// all sharing one threshold color (green <=75, yellow 76-90, red >90). The
// countdown shows two units in the red zone (one unit below it), pulses
// red<->dark-red at >=95%, and at >=100% the bar is dropped, leaving
// "⛔ <pct>% <countdown>".
//
// The bars stretch to fill the terminal width: the bars of a line share the
// columns its text leaves free. Without a width they are 10 cells, on one line.
//
// `render` is pure over (JSON, now, account, width) so the tests at the bottom
// exercise every branch deterministically with a fixed clock.

use std::path::{Path, PathBuf};

use serde_json::Value;
use unicode_width::UnicodeWidthStr;

pub mod update;

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
// a bar's empty part
const GREY: &str = "\x1b[90m";

const SEP: &str = "  ";

/// A bar cell, filled or empty (the color tells them apart): 3/4 high, so the
/// bars of the two lines don't touch, as full blocks would.
const CELL: char = '▆';
/// Bar width when the terminal width is unknown.
const BAR_WIDTH: usize = 10;
/// Narrowest a stretched bar gets, however little room is left.
const MIN_BAR: usize = 5;
/// The status fits on one line when each of its bars gets this many cells.
const ONE_LINE_BAR: usize = 7;
/// Columns left free at the right edge: Claude Code cuts the status line short
/// of the terminal width (with a "…"); at 97 columns, 92 fit.
const MARGIN: usize = 5;

/// The file in a config dir that names its account (first line).
pub const ACCOUNT_FILE: &str = "account-name";

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

/// A piece of a line: text, or a bar whose width is decided per line by `layout`.
#[derive(Clone)]
enum Seg {
    Text(String),
    /// the percentage it is filled to
    Bar(i64),
}

/// Terminal cells taken by `s`: ANSI escapes take none, emoji two.
fn visible_width(s: &str) -> usize {
    let mut plain = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            for c2 in chars.by_ref() {
                if c2 == 'm' {
                    break;
                }
            }
        } else {
            plain.push(c);
        }
    }
    plain.width()
}

/// colored progress bar `width` cells wide: `bar(42, 10)` -> "[▆▆▆▆▆▆▆▆▆▆]",
/// the first 4 cells in the threshold color, the rest grey
fn bar(pct: i64, width: usize) -> String {
    let pct = pct.clamp(0, 100) as usize;
    let filled = (pct * width + 50) / 100; // integer rounding
    let cells = |n: usize| CELL.to_string().repeat(n);
    format!(
        "{RESET}[{}{}{RESET}{GREY}{}{RESET}]",
        color_for(pct as i64),
        cells(filled),
        cells(width - filled)
    )
}

/// the percentage after a bar: " 42%", in the threshold color
fn pct_label(pct: i64) -> String {
    let pct = pct.clamp(0, 100);
    format!(" {}{pct:3}%{RESET}", color_for(pct))
}

/// The columns a line's bars share: those its text (and the bars' brackets)
/// leave free of `columns`, keeping MARGIN; and the number of bars.
fn bar_space(line: &[Seg], columns: usize) -> (usize, usize) {
    let bars = line.iter().filter(|s| matches!(s, Seg::Bar(_))).count();
    let fixed: usize = line
        .iter()
        .map(|s| match s {
            Seg::Text(t) => visible_width(t),
            Seg::Bar(_) => 2,
        })
        .sum();
    (columns.saturating_sub(fixed + MARGIN), bars)
}

/// Whether every bar of `line` gets ONE_LINE_BAR cells in `columns`.
fn roomy(line: &[Seg], columns: usize) -> bool {
    let (free, bars) = bar_space(line, columns);
    free / bars.max(1) >= ONE_LINE_BAR
}

/// Join a line's segments, sizing its bars to share the columns the text leaves
/// free (keeping MARGIN) — or BAR_WIDTH each when `columns` is unknown.
fn layout(line: &[Seg], columns: Option<usize>) -> String {
    let space = columns.map(|c| bar_space(line, c));

    let mut out = String::new();
    let mut i = 0;
    for seg in line {
        match seg {
            Seg::Text(t) => out.push_str(t),
            Seg::Bar(pct) => {
                let width = match space {
                    // spread evenly, the first bars taking the remainder
                    Some((free, bars)) => (free / bars + usize::from(i < free % bars)).max(MIN_BAR),
                    None => BAR_WIDTH,
                };
                out.push_str(&bar(*pct, width));
                i += 1;
            }
        }
    }
    out
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
fn metric(label: &str, pct: i64, resets_at: i64, now: i64) -> Vec<Seg> {
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
        vec![Seg::Text(format!(
            "{DIM}{label}{RESET} ⛔ {color}{pct}%{RESET}{tail}"
        ))]
    } else {
        // countdown sits between the icon and the bar, sharing its color
        let lead = if cd.is_empty() {
            String::new()
        } else {
            format!("{cd} ")
        };
        vec![
            Seg::Text(format!("{DIM}{label}{RESET} {lead}")),
            Seg::Bar(pct),
            Seg::Text(pct_label(pct)),
        ]
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

/// The config directory Claude Code runs from: CLAUDE_CONFIG_DIR (`env`) if set;
/// else the one the transcript sits in (<config dir>/projects/<project>/<id>.jsonl),
/// as Claude Code leaves the variable unset for the default; else ~/.claude.
pub fn config_dir(env: Option<&str>, v: &Value, home: Option<&str>) -> Option<PathBuf> {
    if let Some(dir) = env.filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let transcript = text(v, &["transcript_path"], "");
    let projects = Path::new(&transcript).parent().and_then(Path::parent);
    if let Some(projects) = projects.filter(|p| p.file_name().is_some_and(|n| n == "projects")) {
        return projects.parent().map(Path::to_path_buf);
    }
    home.map(|h| Path::new(h).join(".claude"))
}

/// The account a config dir belongs to: the first line of its ACCOUNT_FILE
/// (`file`, its contents), so every machine can name its own dirs; without one,
/// the dir's name: `.claude-herta` -> "herta", `.claude` -> "default".
pub fn account_name(dir: Option<&Path>, file: Option<&str>) -> String {
    if let Some(name) = file.and_then(|f| f.lines().next()).map(str::trim) {
        if !name.is_empty() {
            return name.to_owned();
        }
    }
    let base = dir
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .unwrap_or("?");
    match base.strip_prefix(".claude") {
        Some("") => "default".to_owned(),
        Some(rest) if rest.starts_with('-') && rest.len() > 1 => rest[1..].to_owned(),
        _ => base.to_owned(),
    }
}

fn folder_of(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| cwd.to_owned())
}

/// Render the status line from the status JSON, the current unix time, the
/// account name and the terminal width (`None` when unknown).
pub fn render(v: &Value, now: i64, account: &str, columns: Option<usize>) -> String {
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

    let model = Seg::Text(format!("{BOLD}{MAG}🤖 {model}{RESET}{SEP}"));
    let context = [
        Seg::Text(format!(
            "{CYAN}📁 {folder}{RESET}{SEP}{YEL}👤 {account}{RESET}{SEP}🧠 "
        )),
        Seg::Bar(ctx),
        Seg::Text(pct_label(ctx)),
    ];

    let mut limits = Vec::new();
    if five >= 0 {
        limits.extend(metric("⏳", five, five_reset, now));
    }
    if seven >= 0 {
        if !limits.is_empty() {
            limits.push(Seg::Text(SEP.to_owned()));
        }
        limits.extend(metric("📅", seven, seven_reset, now));
    }

    // the context, after the model if `with_model`, before the limits if `with_limits`
    let line = |with_model: bool, with_limits: bool| {
        let mut line = Vec::new();
        if with_model {
            line.push(model.clone());
        }
        line.extend(context.iter().cloned());
        if with_limits && !limits.is_empty() {
            line.push(Seg::Text(SEP.to_owned()));
            line.extend(limits.iter().cloned());
        }
        line
    };

    let Some(columns) = columns else {
        return layout(&line(true, true), None);
    };
    // one line if its bars get room enough — with the model, else without it
    for with_model in [true, false] {
        let one = line(with_model, true);
        if roomy(&one, columns) {
            return layout(&one, Some(columns));
        }
    }
    let mut out = layout(&line(true, false), Some(columns));
    if !limits.is_empty() {
        out.push('\n');
        out.push_str(&layout(&limits, Some(columns)));
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

    /// `render` at no known width, as plain text.
    fn plain(v: &Value, now: i64) -> String {
        strip(&render(v, now, "akos", None))
    }

    /// A single five-hour metric at `pct`, resetting at FAR, plus model/ctx.
    fn with_five(pct: i64, reset: i64) -> Value {
        json!({
            "model": {"display_name": "Opus 4.8"},
            "context_window": {"used_percentage": 40},
            "rate_limits": {"five_hour": {"used_percentage": pct, "resets_at": reset}}
        })
    }

    /// Both limits, for the two-line layout.
    fn with_both(ctx: i64, five: i64, seven: i64) -> Value {
        json!({
            "model": {"display_name": "Opus 4.8"},
            "workspace": {"current_dir": "/home/me/claude-status"},
            "context_window": {"used_percentage": ctx},
            "rate_limits": {
                "five_hour": {"used_percentage": five, "resets_at": FAR},
                "seven_day": {"used_percentage": seven, "resets_at": FAR}
            }
        })
    }

    /// The bars' widths in a rendered line, in cells.
    fn bar_widths(line: &str) -> Vec<usize> {
        line.split('[')
            .skip(1)
            .map(|b| b.chars().take_while(|&c| c == CELL).count())
            .collect()
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
        assert_eq!(
            plain(&with_five(30, FAR), NOW),
            "🤖 Opus  📁 ?  👤 akos  🧠 [▆▆▆▆▆▆▆▆▆▆]  40%  ⏳ 4d [▆▆▆▆▆▆▆▆▆▆]  30%"
        );
    }

    #[test]
    fn wraps_only_when_needed() {
        // with_both's text takes 76 cells on one line, 9 of them the model's
        let at = |columns| strip(&render(&with_both(30, 55, 20), NOW, "akos", Some(columns)));

        // 102 - 76 - MARGIN = 21: three bars of 7, model and all
        let s = at(102);
        assert_eq!(s.lines().count(), 1);
        assert!(s.starts_with("🤖 Opus  📁 claude-status"));
        assert_eq!(bar_widths(&s), [7, 7, 7]);

        // one cell short: the model goes, the bars get its room
        let s = at(101);
        assert_eq!(s.lines().count(), 1);
        assert!(s.starts_with("📁 claude-status"));
        assert_eq!(bar_widths(&s), [10, 10, 9]);

        // 93 - 67 - MARGIN = 21: still one line, without the model
        let s = at(93);
        assert_eq!(s.lines().count(), 1);
        assert_eq!(bar_widths(&s), [7, 7, 7]);

        // one cell short again: two lines, the model back
        let s = at(92);
        assert_eq!(s.lines().count(), 2);
        assert!(s.starts_with("🤖 Opus  📁 claude-status"));
    }

    #[test]
    fn bar_fills_in_the_threshold_color_and_empties_in_grey() {
        assert_eq!(
            bar(42, 10),
            format!("{RESET}[{GRN}▆▆▆▆{RESET}{GREY}▆▆▆▆▆▆{RESET}]")
        );
        assert_eq!(
            bar(80, 5),
            format!("{RESET}[{YEL}▆▆▆▆{RESET}{GREY}▆{RESET}]")
        );
        assert_eq!(
            bar(150, 4),
            format!("{RESET}[{RED}▆▆▆▆{RESET}{GREY}{RESET}]")
        ); // clamped
    }

    #[test]
    fn bars_fill_the_width_less_the_margin() {
        for columns in [60, 97, 140] {
            let out = render(&with_both(30, 55, 20), NOW, "akos", Some(columns));
            for line in out.lines() {
                assert_eq!(
                    visible_width(line),
                    columns - MARGIN,
                    "{columns}: {}",
                    strip(line)
                );
            }
        }
    }

    #[test]
    fn the_limits_share_their_line_evenly() {
        let out = strip(&render(&with_both(30, 55, 20), NOW, "akos", Some(92)));
        let lines: Vec<&str> = out.lines().collect();
        // 92 - MARGIN - 46 cells of text
        assert_eq!(bar_widths(lines[0]), [41]);
        // 59 free cells, split between the two bars, the first taking the odd one
        assert_eq!(bar_widths(lines[1]), [30, 29]);
    }

    #[test]
    fn narrow_terminal_keeps_min_bar() {
        let out = strip(&render(&with_both(30, 55, 20), NOW, "akos", Some(20)));
        for line in out.lines() {
            assert!(bar_widths(line).iter().all(|&w| w == MIN_BAR), "{line}");
        }
    }

    #[test]
    fn green_countdown_is_one_unit_and_green() {
        let out = render(&with_five(30, FAR), NOW, "akos", None);
        assert!(strip(&out).contains("⏳ 4d [")); // one unit, then the bar
        assert!(out.contains(&format!("{GRN}4d{RESET}"))); // colored to match
    }

    #[test]
    fn yellow_zone_76_to_90() {
        for pct in [76, 80, 90] {
            let out = render(&with_five(pct, FAR), NOW, "akos", None);
            assert!(strip(&out).contains("⏳ 4d [")); // still one unit
            assert!(out.contains(&format!("{YEL}4d{RESET}")));
        }
    }

    #[test]
    fn red_zone_switches_to_two_units_and_stays_static_below_95() {
        let out = render(&with_five(92, FAR), NOW, "akos", None);
        assert!(strip(&out).contains("⏳ 4d 15h [")); // two units, bar present
        assert!(out.contains(&format!("{RED}4d 15h{RESET}")));
        assert!(!out.contains(DRED)); // 92 < 95 -> not flashing
    }

    #[test]
    fn flashing_at_95_pulses_on_odd_seconds() {
        // bar is still present in the 95–99 band
        assert!(plain(&with_five(95, FAR), NOW).contains("⏳ 4d 15h ["));
        // even second -> bright red, no dark-red anywhere
        assert!(!render(&with_five(95, FAR), NOW, "akos", None).contains(DRED));
        // odd second -> countdown pulses to dark red
        assert!(render(&with_five(95, FAR), NOW + 1, "akos", None).contains(DRED));
    }

    #[test]
    fn over_limit_drops_bar_shows_flag_and_percent() {
        let s = plain(&with_five(100, FAR), NOW);
        assert!(s.ends_with("  ⏳ ⛔ 100% 4d 15h")); // ⛔, %, then countdown, no bar
                                                     // and >135% keeps rendering the real percentage
        assert!(plain(&with_five(135, FAR), NOW).contains("⛔ 135%"));
    }

    #[test]
    fn context_keeps_its_bar_when_a_limit_is_red() {
        let v = json!({
            "model": {"display_name": "Opus"},
            "context_window": {"used_percentage": 96},
            "rate_limits": {"five_hour": {"used_percentage": 100, "resets_at": FAR}}
        });
        let out = strip(&render(&v, NOW, "akos", Some(60)));
        assert!(out.contains("🧠 [▆")); // bar stays, there is room for it
        assert!(out.contains("]  96%"));
        assert_eq!(visible_width(&out), 60 - MARGIN); // and still stretches
    }

    #[test]
    fn absent_metric_is_omitted_present_one_shown() {
        // only seven_day present
        let v = json!({
            "model": {"display_name": "Sonnet 5"},
            "context_window": {"used_percentage": 33},
            "rate_limits": {"seven_day": {"used_percentage": 50, "resets_at": FAR}}
        });
        let s = plain(&v, NOW);
        assert!(s.contains("  📅 4d ["));
        assert!(!s.contains('⏳')); // five_hour absent -> omitted
    }

    #[test]
    fn no_limits_means_one_line() {
        let v =
            json!({"model": {"display_name": "Opus"}, "context_window": {"used_percentage": 10}});
        assert_eq!(render(&v, NOW, "akos", Some(80)).lines().count(), 1);
    }

    #[test]
    fn no_reset_means_no_countdown() {
        let s = plain(&with_five(20, 0), NOW);
        assert!(s.contains("⏳ [")); // bar immediately after icon, no countdown
    }

    #[test]
    fn model_family_and_cwd_fallback() {
        assert!(plain(&json!({"model": {"display_name": "Haiku 4.5"}}), NOW).contains("🤖 Haiku"));
        // top-level cwd fallback + basename
        assert!(plain(&json!({"cwd": "/var/log/app"}), NOW).contains("📁 app"));
    }

    #[test]
    fn config_dir_resolution() {
        let home = Some("/home/me");
        let v = json!({"transcript_path": "/home/me/.claude-herta/projects/-x/s.jsonl"});
        // the env var wins
        assert_eq!(
            config_dir(Some("/home/me/.claude-lvgl"), &v, home),
            Some(PathBuf::from("/home/me/.claude-lvgl"))
        );
        // then the transcript's config dir (an empty env var counts as unset)
        assert_eq!(
            config_dir(Some(""), &v, home),
            Some(PathBuf::from("/home/me/.claude-herta"))
        );
        // then ~/.claude
        let other = json!({"transcript_path": "/elsewhere/s.jsonl"});
        assert_eq!(
            config_dir(None, &other, home),
            Some(PathBuf::from("/home/me/.claude"))
        );
        assert_eq!(config_dir(None, &Value::Null, None), None);
    }

    #[test]
    fn account_name_from_file_or_dir() {
        let dir = |d| Some(Path::new(d));
        // the file decides, whatever the dir is called
        assert_eq!(
            account_name(dir("/home/me/.claude"), Some("akos\n")),
            "akos"
        );
        assert_eq!(
            account_name(dir("/home/me/.claude"), Some("  lvgl  \nrest")),
            "lvgl"
        );
        // no file, or a blank one: the dir's name
        assert_eq!(account_name(dir("/home/me/.claude"), None), "default");
        assert_eq!(
            account_name(dir("/home/me/.claude-herta/"), Some("\n")),
            "herta"
        );
        assert_eq!(account_name(dir("/opt/cfg"), None), "cfg");
        assert_eq!(account_name(dir("/home/me/.claude-"), None), ".claude-");
        assert_eq!(account_name(None, None), "?");
        assert!(plain(&Value::Null, NOW).contains("👤 akos"));
    }

    #[test]
    fn malformed_input_degrades_to_defaults() {
        // Value::Null is what main() falls back to on unparseable stdin
        assert_eq!(
            plain(&Value::Null, NOW),
            "🤖 ?  📁 ?  👤 akos  🧠 [▆▆▆▆▆▆▆▆▆▆]   0%"
        );
    }
}
