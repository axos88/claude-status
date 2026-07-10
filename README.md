# claude-status

A fast status-line renderer for [Claude Code](https://claude.ai/code), written in Rust.

Claude Code invokes the `statusLine.command` on every refresh, piping the
status JSON to **stdin** and using **stdout** as the status line. This binary
reads that JSON and prints a single line:

```
🤖 Opus  📁 claude-status  🧠 [██████░░░░] 63%  ⏳ [█████░░░░░] 45%  📅 [███████░░░] 72%
```

- 🤖 model family · 📁 current folder · 🧠 context window · ⏳ 5-hour limit · 📅 7-day limit
- Each usage metric is a colored progress bar (green < 60%, yellow < 85%, red ≥ 85%),
  followed by the time until that limit resets, shown as its single
  highest-magnitude unit (e.g. `4d`, `2h`, `50m`).
- When a rate limit is **near** (90–99%) its bar is replaced by a live, pulsing
  countdown to reset shown to two units (e.g. `1h 56m`); at **100%** it shows ⛔
  plus that two-unit countdown and the reset clock. When any limit is near, the
  context bar collapses to just its percentage.

It's a port of the original `statusline.sh` (kept in this repo as the
differential-test reference). At ~1 ms per invocation it's roughly 20–40× faster
than the bash + `jq` original, and depends on no external binaries at runtime.

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

The test harness feeds fixtures covering each branch (normal / near / hit /
missing metrics / cwd fallback / thresholds) through both the Rust binary and
the original shell script and diffs the output (ANSI stripped, since the
per-second blink is time-dependent). Malformed input is checked separately: the
Rust port degrades to documented defaults rather than the shell's `""`→`0`
coercion.
