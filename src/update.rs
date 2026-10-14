// Periodic self-update from the rolling `latest` GitHub release.
//
// Only release builds update themselves: the release workflow compiles them with
// CLAUDE_STATUS_RELEASE set. A local `cargo build` / `cargo install` doesn't, so
// a binary built from source is never overwritten by the (older) release. How
// often (by default every 2 hours) is CLAUDE_STATUS_AUTOUPDATE_INTERVAL's, at
// run time; 0 turns it off.
//
// `spawn_if_due` runs after the status line is printed: when an update is due it
// starts
// `claude-status --update` detached in the background, so the status line never
// waits on the network. That process (`run`) fetches the asset's .sha256 — a few
// bytes — and only when it differs from the one it installed last downloads the
// archive, checks it, unpacks it and renames the new binary over the old one
// (sessions still running the old one keep their copy).
//
// Downloading and unpacking shell out to curl and tar (both ship with Linux,
// macOS and Windows 10+, whose tar also reads .zip), which keeps an HTTP/TLS
// stack out of a binary that runs every second.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

/// The argument that makes the binary run the update instead of rendering.
pub const FLAG: &str = "--update";

const RELEASE_BUILD: bool = option_env!("CLAUDE_STATUS_RELEASE").is_some();
/// Seconds between updates; 0 turns them off (`--update` still works).
const INTERVAL_VAR: &str = "CLAUDE_STATUS_AUTOUPDATE_INTERVAL";
/// The interval when INTERVAL_VAR is unset, or not a number.
const DEFAULT_INTERVAL: Duration = Duration::from_secs(2 * 60 * 60);
const BASE_URL: &str = "https://github.com/axos88/claude-status/releases/latest/download";

/// Touched when an update is started; its age says when the next one is due.
const STAMP: &str = "last-check";
/// The checksum of the archive installed last.
const INSTALLED: &str = "installed.sha256";
/// The last update's outcome.
const LOG: &str = "update.log";

/// The release asset built for a platform, as (name without extension, archive
/// extension): ("claude-status-x86_64-unknown-linux-gnu", "tar.gz"). Its
/// checksum is "<name>.sha256".
fn asset(arch: &str, os: &str, musl: bool) -> Option<(String, &'static str)> {
    let (platform, ext) = match os {
        "linux" if musl => ("unknown-linux-musl", "tar.gz"),
        "linux" => ("unknown-linux-gnu", "tar.gz"),
        "macos" => ("apple-darwin", "tar.gz"),
        "windows" => ("pc-windows-msvc", "zip"),
        _ => return None,
    };
    Some((format!("claude-status-{arch}-{platform}"), ext))
}

/// Where the update keeps its files: $XDG_STATE_HOME (or ~/.local/state), or
/// %LOCALAPPDATA% on Windows, + /claude-status.
fn state_dir() -> Option<PathBuf> {
    let var = |name| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        var("LOCALAPPDATA")?
    } else {
        var("XDG_STATE_HOME").or_else(|| var("HOME").map(|h| h.join(".local/state")))?
    };
    Some(base.join("claude-status"))
}

/// The interval between updates INTERVAL_VAR (holding `value`) asks for; `None`:
/// updates are off.
fn interval(value: Option<&str>) -> Option<Duration> {
    match value.and_then(|v| v.trim().parse::<u64>().ok()) {
        Some(0) => None,
        Some(secs) => Some(Duration::from_secs(secs)),
        None => Some(DEFAULT_INTERVAL),
    }
}

/// Whether an update is due, `interval` after the last one, which was started
/// at `last` (`None`: never). A `last` in the future (the clock went back)
/// counts as due too.
fn is_due(last: Option<SystemTime>, now: SystemTime, interval: Duration) -> bool {
    match last.map(|last| now.duration_since(last)) {
        Some(Ok(age)) => age >= interval,
        _ => true,
    }
}

/// The checksum in a .sha256 file ("<hex>  <file name>"), lowercased.
fn parse_checksum(file: &str) -> Option<String> {
    let hex = file.split_whitespace().next()?;
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| hex.to_ascii_lowercase())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Start `<this binary> --update` in the background if a release build's update
/// is due (and not turned off). Never fails: the status line must not suffer for
/// it.
pub fn spawn_if_due() {
    if !RELEASE_BUILD {
        return;
    }
    let Some(interval) = interval(std::env::var(INTERVAL_VAR).ok().as_deref()) else {
        return;
    };
    let Some(dir) = state_dir() else { return };
    let stamp = dir.join(STAMP);
    let last = fs::metadata(&stamp).and_then(|m| m.modified()).ok();
    if !is_due(last, SystemTime::now(), interval) {
        return;
    }
    // stamp first, so the sessions refreshing every second don't all start one
    if fs::create_dir_all(&dir)
        .and_then(|()| fs::write(&stamp, ""))
        .is_err()
    {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };

    let mut cmd = Command::new(exe);
    cmd.arg(FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // its own process group, so it outlives the status line command
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    // CREATE_NO_WINDOW: no console flashing up
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000);
    let _ = cmd.spawn();
}

/// `run`, with its outcome written to the update log.
pub fn run_and_log() {
    let outcome = run().unwrap_or_else(|e| format!("failed: {e}"));
    if let Some(dir) = state_dir() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let _ = fs::write(dir.join(LOG), format!("{now} {outcome}\n"));
    }
}

/// Install the latest release over this binary, unless it is the one installed last.
fn run() -> Result<String, String> {
    let (name, ext) = asset(
        std::env::consts::ARCH,
        std::env::consts::OS,
        cfg!(target_env = "musl"),
    )
    .ok_or("no release is built for this platform")?;
    let dir = state_dir().ok_or("no state directory")?;
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    let checksum = curl(&format!("{BASE_URL}/{name}.sha256"), None)?;
    let latest = parse_checksum(&String::from_utf8_lossy(&checksum))
        .ok_or("the release's .sha256 holds no checksum")?;
    let installed = dir.join(INSTALLED);
    if fs::read_to_string(&installed).is_ok_and(|s| s.trim() == latest) {
        return Ok(format!("up to date ({latest})"));
    }

    // per process, in case two ever run at once
    let work = dir.join(format!("download-{}", std::process::id()));
    let result = download_and_install(&name, ext, &latest, &work);
    let _ = fs::remove_dir_all(&work);
    result?;

    fs::write(&installed, &latest).map_err(|e| format!("{}: {e}", installed.display()))?;
    Ok(format!("updated to {latest}"))
}

fn download_and_install(name: &str, ext: &str, checksum: &str, work: &Path) -> Result<(), String> {
    fs::create_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))?;
    let archive = work.join(format!("{name}.{ext}"));
    curl(&format!("{BASE_URL}/{name}.{ext}"), Some(&archive))?;

    let bytes = fs::read(&archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let got = sha256_hex(&bytes);
    if got != checksum {
        return Err(format!("checksum mismatch: expected {checksum}, got {got}"));
    }

    let status = Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(work)
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("tar: {e}"))?;
    if !status.success() {
        return Err(format!("tar: {status}"));
    }

    let new = work.join(format!("claude-status{}", std::env::consts::EXE_SUFFIX));
    let exe = std::env::current_exe().map_err(|e| format!("this binary's path: {e}"))?;
    replace(&new, &exe).map_err(|e| format!("installing to {}: {e}", exe.display()))
}

/// Download `url` into `to`, or return its body when `to` is `None`.
fn curl(url: &str, to: Option<&Path>) -> Result<Vec<u8>, String> {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--max-time", "120", url])
        .stdin(Stdio::null());
    if let Some(to) = to {
        cmd.arg("-o").arg(to);
    }
    let out = cmd.output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("curl {url}: {}", err.trim()));
    }
    Ok(out.stdout)
}

/// Put `new` in place of `exe`: copied beside it first, so the final rename
/// stays on one filesystem and is atomic.
fn replace(new: &Path, exe: &Path) -> std::io::Result<()> {
    let staged = exe.with_extension("new");
    fs::copy(new, &staged)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&staged, fs::Permissions::from_mode(0o755))?;
    }
    // a running .exe can't be replaced, but it can be renamed out of the way
    #[cfg(windows)]
    {
        let old = exe.with_extension("old");
        let _ = fs::remove_file(&old);
        fs::rename(exe, &old)?;
    }
    fs::rename(&staged, exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_per_platform() {
        let name = |arch, os, musl| asset(arch, os, musl).map(|(n, e)| format!("{n}.{e}"));
        assert_eq!(
            name("x86_64", "linux", false).as_deref(),
            Some("claude-status-x86_64-unknown-linux-gnu.tar.gz")
        );
        assert_eq!(
            name("aarch64", "linux", true).as_deref(),
            Some("claude-status-aarch64-unknown-linux-musl.tar.gz")
        );
        assert_eq!(
            name("aarch64", "macos", false).as_deref(),
            Some("claude-status-aarch64-apple-darwin.tar.gz")
        );
        assert_eq!(
            name("x86_64", "windows", false).as_deref(),
            Some("claude-status-x86_64-pc-windows-msvc.zip")
        );
        assert_eq!(name("x86_64", "freebsd", false), None);
    }

    #[test]
    fn due_after_the_interval() {
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let hour = Duration::from_secs(3600);
        assert!(is_due(None, now, hour)); // never checked
        assert!(!is_due(
            Some(now - hour + Duration::from_secs(1)),
            now,
            hour
        ));
        assert!(is_due(Some(now - hour), now, hour));
        assert!(is_due(Some(now + Duration::from_secs(60)), now, hour)); // clock went back
    }

    #[test]
    fn interval_from_the_env_var() {
        assert_eq!(interval(None), Some(DEFAULT_INTERVAL));
        assert_eq!(interval(Some("0")), None); // off
        assert_eq!(interval(Some(" 0 ")), None);
        assert_eq!(interval(Some("600")), Some(Duration::from_secs(600)));
        assert_eq!(DEFAULT_INTERVAL, Duration::from_secs(7200));
        // not a number: the default
        assert_eq!(interval(Some("")), Some(DEFAULT_INTERVAL));
        assert_eq!(interval(Some("2h")), Some(DEFAULT_INTERVAL));
        assert_eq!(interval(Some("-1")), Some(DEFAULT_INTERVAL));
    }

    #[test]
    fn checksum_file_parsing() {
        let hex = "00bf6db5244859ead7df91b224e46d078bd00b0acd6d9c0008f430ca0163fb25";
        assert_eq!(
            parse_checksum(&format!(
                "{hex}  claude-status-x86_64-unknown-linux-gnu.tar.gz\n"
            ))
            .as_deref(),
            Some(hex)
        );
        assert_eq!(parse_checksum(&hex.to_uppercase()).as_deref(), Some(hex));
        assert_eq!(parse_checksum("Not Found"), None);
        assert_eq!(parse_checksum(""), None);
        assert_eq!(parse_checksum(&hex[1..]), None);
    }

    #[test]
    fn sha256_of_bytes() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
