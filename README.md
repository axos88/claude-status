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

It began life as a bash + `jq` script; at ~1 ms per invocation this Rust
rewrite is roughly 20–40× faster, and depends on no external binaries at runtime.

## Install

> Replace `<owner>/claude-status` below with the actual repository slug.

### Download a prebuilt binary (no Rust toolchain needed)

The `latest` release always carries a prebuilt binary per platform (each with a
`.sha256` checksum alongside). Copy-paste the one line for your platform — it
downloads and installs `claude-status` to `~/.local/bin` (make sure that's on
your `PATH`):

**Linux — x86_64**
```sh
mkdir -p ~/.local/bin && curl -fsSL https://github.com/<owner>/claude-status/releases/latest/download/claude-status-x86_64-unknown-linux-gnu.tar.gz | tar -xz -C ~/.local/bin claude-status
```

**Linux — ARM64**
```sh
mkdir -p ~/.local/bin && curl -fsSL https://github.com/<owner>/claude-status/releases/latest/download/claude-status-aarch64-unknown-linux-gnu.tar.gz | tar -xz -C ~/.local/bin claude-status
```

**macOS — Apple Silicon**
```sh
mkdir -p ~/.local/bin && curl -fsSL https://github.com/<owner>/claude-status/releases/latest/download/claude-status-aarch64-apple-darwin.tar.gz | tar -xz -C ~/.local/bin claude-status
```

**macOS — Intel**
```sh
mkdir -p ~/.local/bin && curl -fsSL https://github.com/<owner>/claude-status/releases/latest/download/claude-status-x86_64-apple-darwin.tar.gz | tar -xz -C ~/.local/bin claude-status
```

**Windows — x86_64** (PowerShell)
```powershell
irm https://github.com/<owner>/claude-status/releases/latest/download/claude-status-x86_64-pc-windows-msvc.zip -OutFile claude-status.zip; Expand-Archive -Force claude-status.zip .
```

On Linux, swap `-gnu` for `-musl` in the URL to get a fully static binary that
runs on any distro. Every asset has a matching `<asset>.sha256` on the release
if you want to verify the download first.

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
cargo test
```

`render()` is pure over `(JSON, now)`, so the tests in `src/lib.rs` drive it with
a fixed clock and assert exact output across every branch — the color/unit
thresholds (75, 90), the flashing zone (95, checked on both even and odd seconds
via `now`), the over-limit layout, the context collapse, missing metrics, the
cwd fallback, and malformed input degrading to defaults.

## Releases

Every push to `main` runs [`.github/workflows/release.yml`](.github/workflows/release.yml),
which rebuilds all targets and refreshes a single rolling **`latest`** GitHub
Release with the fresh binaries (and their `sha256` checksums) — so the download
commands above always fetch the newest build. No manual tagging needed.

The [release profile](Cargo.toml) is tuned for a small binary — `opt-level="z"`,
fat LTO, a single codegen unit, and `panic="abort"` — but deliberately keeps the
symbol table (`strip="debuginfo"`) so **panic backtraces still resolve to
function names** on Linux and macOS. Full DWARF is dropped, which shrinks the
Linux x86_64 binary from ~2 MB to ~410 KB; a panic still prints its exact source
location plus a symbolicated stack (set `RUST_BACKTRACE=1`). (On Windows, symbols
live in a separate `.pdb` that the release archive doesn't ship, so backtrace
frames there show addresses only — the panic location is still printed.)
