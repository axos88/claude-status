# claude-status

A fast status-line renderer for [Claude Code](https://claude.ai/code), written in Rust.

Claude Code invokes the `statusLine.command` on every refresh, piping the
status JSON to **stdin** and using **stdout** as the status line. This binary
reads that JSON and prints a single line:

```
🤖 Opus  📁 claude-status  🧠 [██████░░░░] 63%  ⏳ 4d [█████░░░░░] 45%  📅 2h [███████░░░] 72%
```

- 🤖 model family · 📁 current folder · 🧠 context window · ⏳ 5-hour limit · 📅 7-day limit
- Each usage metric renders as **icon · reset-countdown · progress-bar · %**, and
  everything in it shares one threshold color driven by the percentage.

The reset countdown and its styling escalate with usage, so a glance tells you
both how full a limit is and how long until it clears:

| Usage | Color | Countdown | Flashing | Bar |
|------:|-------|-----------|:--------:|-----|
| ≤ 75% | 🟢 green | 1 unit (`4d`) | – | ✓ |
| 76–90% | 🟡 yellow | 1 unit (`4d`) | – | ✓ |
| 91–94% | 🔴 red | 2 units (`4d 15h`) | – | ✓ |
| 95–99% | 🔴 red | 2 units | ⚡ pulses red↔dark-red | ✓ |
| ≥ 100% | 🔴 red | 2 units | ⚡ | — replaced by ⛔ |

When any limit is red (> 90%), the 🧠 context bar collapses to just its
percentage to save room.

It's kept in lockstep with the original `statusline.sh`, which lives in this repo
as an independent differential-test reference. At ~1 ms per invocation the Rust
binary is roughly 20–40× faster than the bash + `jq` original, and depends on no
external binaries at runtime.

## Build & install

```sh
cargo install --path .        # installs to ~/.cargo/bin/claude-status
```

Then point Claude Code at it in `~/.claude/settings.json`:

```json
"statusLine": {
  "type": "command",
  "command": "/home/you/.cargo/bin/claude-status",
  "refreshInterval": 1
}
```

## Testing

```sh
cargo build --release
bash tests/diff_against_bash.sh   # renders must match statusline.sh on every branch
```

The harness feeds fixtures covering every branch — each color/unit threshold,
the flashing and over-limit zones, missing metrics, the cwd fallback, and the
context collapse — through both the Rust binary and the shell reference and
diffs their output. Static fixtures (< 95%) are diffed **with ANSI intact** so
color is actually verified; only the flashing fixtures (≥ 95%) strip ANSI, since
their per-second pulse is time-dependent. Malformed input is checked separately:
the Rust port degrades to documented defaults rather than the shell's `""`→`0`
coercion.
