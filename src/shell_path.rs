//! Finds the captain's command-line tools when the app starts outside a terminal. macOS starts an
//! app opened from Finder, the Dock, Spotlight or `open` with the bare PATH
//! `/usr/bin:/bin:/usr/sbin:/sbin`, which holds neither `claude` nor the `gh`, `jq`, `treehouse`
//! and `no-mistakes` that firstmate's scripts run. A terminal's PATH comes from the login shell's
//! startup files, so the app asks the captain's login shell for its PATH, the way a terminal
//! window would get it, and adds the folders Claude Code's installers use. A run from a terminal
//! that already finds `claude` keeps its PATH as it is.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The longest the login shell may take to print its PATH. Startup files that wait for input
/// or the network are cut off here, and the app goes on with what it has.
const SHELL_TIMEOUT: Duration = Duration::from_secs(5);

/// Marks the PATH in the shell's output, so a greeting its startup files print is not read as
/// part of it.
const MARK: &str = "__VELVET_HAMMER_PATH__";

/// Where Claude Code's installers put `claude`, under the captain's home or system-wide, in the
/// order they are tried after the login shell's PATH.
const KNOWN: [&str; 4] = [
    "~/.local/bin",
    "~/.claude/local",
    "/opt/homebrew/bin",
    "/usr/local/bin",
];

/// Completes this process's PATH when the app was opened as a bundle or `claude` is not on it,
/// so every command the app and firstmate run finds the captain's tools. Call it first thing in
/// `main`, before any thread starts, since it changes the process's environment.
pub fn complete() {
    let current = std::env::var_os("PATH").unwrap_or_default();
    let bundled = std::env::current_exe()
        .is_ok_and(|exe| exe.to_string_lossy().contains(".app/Contents/MacOS/"));
    if !bundled && finds(&current, "claude") {
        return;
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return;
    };
    let login = std::env::var_os("SHELL").and_then(|shell| login_path(Path::new(&shell)));
    let path = merged(login.as_deref(), &current, &home);
    // SAFETY: `main` calls this before it starts any thread, so nothing reads the environment
    // while it changes.
    unsafe { std::env::set_var("PATH", path) };
}

/// The login shell's PATH, or None when the shell fails, prints no PATH or takes too long.
fn login_path(shell: &Path) -> Option<OsString> {
    // Interactive as well as login, as a terminal window's shell is, so a PATH set in
    // `.zshrc` or `.bashrc` counts too.
    let mut child = Command::new(shell)
        .args([
            "-l",
            "-i",
            "-c",
            &format!("printf '\\n{MARK}%s\\n' \"$PATH\""),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + SHELL_TIMEOUT;
    while child.try_wait().ok()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?;
    parse(&out)
}

/// The PATH from the shell's output: the line after the mark.
fn parse(out: &str) -> Option<OsString> {
    let path = out.lines().find_map(|line| line.strip_prefix(MARK))?.trim();
    (!path.is_empty()).then(|| path.into())
}

/// The login shell's folders first, as a terminal has them, then the ones this process was
/// started with, then each known install folder that exists; each folder once.
fn merged(login: Option<&OsStr>, current: &OsStr, home: &Path) -> OsString {
    let known = KNOWN.iter().map(|dir| match dir.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(dir),
    });
    let mut dirs: Vec<PathBuf> = Vec::new();
    let all = login
        .into_iter()
        .flat_map(std::env::split_paths)
        .chain(std::env::split_paths(current))
        .chain(known.filter(|dir| dir.is_dir()));
    for dir in all {
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).unwrap_or_else(|_| current.to_os_string())
}

/// Whether `program` is an executable file in one of `path`'s folders.
fn finds(path: &OsStr, program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::split_paths(path).any(|dir| {
        std::fs::metadata(dir.join(program))
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/shell-path")
            .join(format!("{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_path_is_read_after_the_mark_whatever_the_startup_files_print() {
        let out = format!("Welcome back!\n\n{MARK}/opt/homebrew/bin:/usr/bin\n");
        assert_eq!(parse(&out), Some("/opt/homebrew/bin:/usr/bin".into()));
        assert_eq!(parse("no mark here\n"), None);
        assert_eq!(parse(&format!("{MARK}\n")), None);
    }

    #[test]
    fn the_login_path_comes_first_then_the_started_path_then_known_folders_that_exist() {
        let home = scratch("home");
        std::fs::create_dir_all(home.join(".local/bin")).unwrap();
        let path = merged(
            Some(OsStr::new("/opt/homebrew/bin:/usr/bin:/bin")),
            OsStr::new("/usr/bin:/bin:/usr/sbin:/sbin"),
            &home,
        );
        let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        let local = home.join(".local/bin");
        assert_eq!(
            &dirs[..5],
            [
                Path::new("/opt/homebrew/bin"),
                Path::new("/usr/bin"),
                Path::new("/bin"),
                Path::new("/usr/sbin"),
                Path::new("/sbin"),
            ]
        );
        assert!(dirs.contains(&local), "{dirs:?}");
        assert!(
            !dirs.contains(&home.join(".claude/local")),
            "a missing folder is left out"
        );
        assert_eq!(
            dirs.iter().filter(|d| **d == Path::new("/usr/bin")).count(),
            1
        );
    }

    #[test]
    fn without_a_login_shell_the_known_folders_still_find_claude() {
        let home = scratch("no-shell");
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let claude = bin.join("claude");
        std::fs::write(&claude, "#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
        let finder = OsStr::new("/usr/bin:/bin:/usr/sbin:/sbin");
        assert!(!finds(finder, "claude"));
        assert!(finds(&merged(None, finder, &home), "claude"));
    }

    #[test]
    fn a_login_shell_that_answers_gives_its_path() {
        let dir = scratch("shell");
        let shell = dir.join("fake-shell");
        std::fs::write(
            &shell,
            "#!/bin/sh\necho 'startup noise'\nPATH=/from/login:/usr/bin\neval \"$4\"\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(login_path(&shell), Some("/from/login:/usr/bin".into()));
    }
}
