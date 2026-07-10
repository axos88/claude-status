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
| ≥ 100% | 🔴 red | 2 units | ⚡ | dropped → `⛔ 100% 4d 15h` |

When any limit is red (> 90%), the 🧠 context bar collapses to just its
percentage to save room.

It's kept in lockstep with the original `statusline.sh`, which lives in this repo
as an independent differential-test reference. At ~1 ms per invocation the Rust
binary is roughly 20–40× faster than the bash + `jq` original, and depends on no
external binaries at runtime.

## Install

> Replace `<owner>/claude-status` below with the actual repository slug.

### Download a prebuilt binary (no Rust toolchain needed)

Every tagged release ships a prebuilt binary per platform on the
[Releases page](https://github.com/<owner>/claude-status/releases/latest),
each with a `.sha256` checksum. Pick the archive for your OS/CPU:

| OS | CPU | Archive |
|----|-----|---------|
| Linux | x86_64 | `claude-status-x86_64-unknown-linux-gnu.tar.gz` (or `-musl` for a fully static build) |
| Linux | ARM64 | `claude-status-aarch64-unknown-linux-gnu.tar.gz` (or `-musl`) |
| macOS | Apple Silicon | `claude-status-aarch64-apple-darwin.tar.gz` |
| macOS | Intel | `claude-status-x86_64-apple-darwin.tar.gz` |
| Windows | x86_64 | `claude-status-x86_64-pc-windows-msvc.zip` |

Linux x86_64, for example — download, verify, extract to `~/.local/bin`:

```sh
base=https://github.com/<owner>/claude-status/releases/latest/download
curl -LO $base/claude-status-x86_64-unknown-linux-gnu.tar.gz
curl -LO $base/claude-status-x86_64-unknown-linux-gnu.tar.gz.sha256
sha256sum -c claude-status-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf claude-status-x86_64-unknown-linux-gnu.tar.gz -C ~/.local/bin claude-status
```

(On macOS, `shasum -a 256 -c` instead of `sha256sum -c`. On Windows, unzip and
place `claude-status.exe` somewhere on your `PATH`.)

### Build from source

```sh
cargo install --path .        # installs to ~/.cargo/bin/claude-status
```

### Point Claude Code at it

In `~/.claude/settings.json`, set the `command` to wherever you put the binary:

```json
"statusLine": {
  "type": "command",
  "command": "/home/you/.local/bin/claude-status",
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

## Releases

Pushing a version tag builds and publishes the binaries via
[`.github/workflows/release.yml`](.github/workflows/release.yml):

```sh
git tag v0.1.0 && git push origin v0.1.0
```

The [release profile](Cargo.toml) is tuned for a small binary — `opt-level="z"`,
fat LTO, a single codegen unit, and `panic="abort"` — but deliberately keeps the
symbol table (`strip="debuginfo"`) so **panic backtraces still resolve to
function names**. Full DWARF is dropped, which shrinks the Linux x86_64 binary
from ~2 MB to ~410 KB; a panic still prints its exact source location plus a
symbolicated stack (set `RUST_BACKTRACE=1`).
