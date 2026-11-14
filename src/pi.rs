//! The pi runtime Velvet Hammer installs, so firstmate can send crewmates on models other than
//! Claude without the captain installing any agent command-line tool. Firstmate already has a
//! verified `pi` harness; the app only has to put a `pi` on the chief of staff's `PATH`, where
//! `bin/fm-spawn.sh` finds it and writes its absolute path into each worker's launch.
//!
//! - **Install.** [`Runtime::install`] downloads pi's official release for this machine from
//!   GitHub with `curl`, checks it against the SHA-256 built into the app ([`Release`]), and
//!   unpacks it with `tar` beside its final place, renaming it in when it is complete.
//! - **Key.** The provider key stays in the macOS keychain. The app never reads it: pi's own
//!   `auth.json` in the app's pi directory names the `security` command that prints it, and
//!   pi runs that command when it needs the key. [`key_state`] only asks whether the entry
//!   exists.
//! - **Account.** [`connect`], run only when the captain presses its button on the Workers
//!   page, writes that `auth.json` and firstmate's pi account setting `config/pi-account` in
//!   the app's own firstmate home, so every pi worker uses the app's pi directory and may spend
//!   only the providers it names.
//! - **Models.** [`Runtime::list_models`] runs `pi --list-models` there, for the chief of staff's
//!   request resolution ([`crate::crew`]).
//!
//! Claude never runs through pi: Claude workers stay on Claude Code and the captain's
//! subscription, and nothing here touches Claude's sign-in.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, OnceLock, RwLock};
use std::thread;

use rune::Waker;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::store::Shared;

/// The pi release the app installs.
pub const VERSION: &str = "1.5.2";

/// The provider the app sets pi workers up for, as pi names it.
pub const PROVIDER: &str = "opencode-go";

/// The keychain entry (`security -s`) that holds the opencode Go key.
pub const KEYCHAIN_SERVICE: &str = "velvet-hammer-opencode-go";

/// The command that adds the key to the keychain. `-w` with no value makes `security` prompt
/// for it, so the key never appears in a command line or a file.
pub const ADD_KEY: &str =
    "security add-generic-password -U -a \"$USER\" -s velvet-hammer-opencode-go -w";

/// One downloadable build of pi.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// The directory URL the asset is downloaded from.
    pub base: String,
    pub asset: String,
    /// The asset's SHA-256, in hex, from the release's `SHA256SUMS`.
    pub sha256: String,
}

impl Release {
    /// The official release for this machine, or None on a platform pi does not build for.
    pub fn official() -> Option<Release> {
        let (asset, sha256) = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => (
                "pi-darwin-arm64.tar.gz",
                "56b5e194d485ced555667b8ba3276157a8272e9ecde9e001e428af80985bdd9b",
            ),
            ("macos", "x86_64") => (
                "pi-darwin-x64.tar.gz",
                "f997a088613514ac97d913aeea7642094d84fe6d46854ff7e64e2c12e536dc50",
            ),
            ("linux", "x86_64") => (
                "pi-linux-x64.tar.gz",
                "811c6f2c9e7aaee252b5404252e556fb713aa209c1f7f924226aa77d4ef70cb5",
            ),
            ("linux", "aarch64") => (
                "pi-linux-arm64.tar.gz",
                "42805cd5615952fcbbfc9c56b04f8feb84395134952e169a6cdc8d7676975666",
            ),
            _ => return None,
        };
        Some(Release {
            base: format!("https://github.com/earendil-works/pi/releases/download/v{VERSION}"),
            asset: asset.into(),
            sha256: sha256.into(),
        })
    }
}

/// Whether the opencode Go key is in the keychain. The secret itself is never read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Present,
    Missing,
    /// This system has no `security` command (not macOS).
    Unsupported,
}

/// Asks the keychain whether the entry exists, without `-w`, so nothing secret is printed.
pub fn key_state() -> Key {
    if !cfg!(target_os = "macos") {
        return Key::Unsupported;
    }
    let found = Command::new("security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match found {
        Ok(status) if status.success() => Key::Present,
        Ok(_) => Key::Missing,
        Err(_) => Key::Unsupported,
    }
}

/// The runtime in the app's data directory.
pub struct Runtime {
    data: PathBuf,
    release: Option<Release>,
    installing: Option<Receiver<Result<(), String>>>,
    /// Why the last install failed, in words for the Workers page.
    pub problem: Option<String>,
    models: Shared<Vec<String>>,
    listing: Option<Receiver<Result<Vec<String>, String>>>,
    /// Why the last model listing failed.
    pub models_problem: Option<String>,
    wake: Arc<OnceLock<Waker>>,
}

impl Runtime {
    /// The runtime under `data`, installed from `release` when the captain asks.
    pub fn open(data: &Path, release: Option<Release>) -> Runtime {
        Runtime {
            data: data.to_path_buf(),
            release,
            installing: None,
            problem: None,
            models: Arc::new(RwLock::new(Vec::new())),
            listing: None,
            models_problem: None,
            wake: Arc::new(OnceLock::new()),
        }
    }

    /// Installs from `release` from now on, for tests and headless scenes.
    pub fn set_release(&mut self, release: Release) {
        self.release = Some(release);
    }

    pub fn connect(&mut self, wake: Waker) {
        let _ = self.wake.set(wake);
    }

    /// Where this version is unpacked: the release's `pi/` directory sits inside it.
    fn dir(&self) -> PathBuf {
        self.data.join("runtime").join(format!("pi-{VERSION}"))
    }

    /// The directory that holds the `pi` executable, which goes first on the chief of staff's
    /// `PATH`.
    pub fn bin_dir(&self) -> PathBuf {
        self.dir().join("pi")
    }

    pub fn program(&self) -> PathBuf {
        self.bin_dir().join("pi")
    }

    /// The app's own pi directory (`PI_CODING_AGENT_DIR`), apart from any `~/.pi` of the
    /// captain's.
    pub fn agent_dir(&self) -> PathBuf {
        self.data.join("pi-agent")
    }

    pub fn installed(&self) -> bool {
        self.program().is_file()
    }

    pub fn installing(&self) -> bool {
        self.installing.is_some()
    }

    pub fn release(&self) -> Option<&Release> {
        self.release.as_ref()
    }

    /// `PATH` for the chief of staff: the runtime's directory, then `path`.
    pub fn path_env(&self, path: Option<OsString>) -> OsString {
        let mut dirs = vec![self.bin_dir()];
        dirs.extend(std::env::split_paths(&path.unwrap_or_default()));
        std::env::join_paths(dirs).unwrap_or_else(|_| self.bin_dir().into_os_string())
    }

    /// What pi lists, shared with the hook that resolves the captain's requests.
    pub fn models(&self) -> Shared<Vec<String>> {
        self.models.clone()
    }

    /// Downloads, checks and unpacks the release on a background thread.
    pub fn install(&mut self) {
        let Some(release) = self.release.clone() else {
            self.problem = Some(format!(
                "pi publishes no build for this computer ({} on {}).",
                std::env::consts::OS,
                std::env::consts::ARCH
            ));
            return;
        };
        if self.installing.is_some() {
            return;
        }
        self.problem = None;
        let (tx, rx) = channel();
        let (dir, wake) = (self.dir(), self.wake.clone());
        thread::spawn(move || {
            let _ = tx.send(install(&release, &dir));
            if let Some(wake) = wake.get() {
                wake();
            }
        });
        self.installing = Some(rx);
    }

    /// Lists the models pi can use with the keys it has, on a background thread.
    pub fn list_models(&mut self) {
        if !self.installed() || self.listing.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let (program, agent, wake) = (self.program(), self.agent_dir(), self.wake.clone());
        thread::spawn(move || {
            let _ = tx.send(list_models(&program, &agent));
            if let Some(wake) = wake.get() {
                wake();
            }
        });
        self.listing = Some(rx);
    }

    /// Takes in a finished install or listing; true when anything changed. A finished install
    /// lists the models at once.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(done) = self.installing.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.installing = None;
            self.problem = done.err();
            self.list_models();
            changed = true;
        }
        if let Some(listed) = self.listing.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.listing = None;
            let (models, problem) = match listed {
                Ok(models) => (models, None),
                Err(problem) => (Vec::new(), Some(problem)),
            };
            *self.models.write().unwrap_or_else(|e| e.into_inner()) = models;
            self.models_problem = problem;
            changed = true;
        }
        changed
    }

    /// Waits for the work in flight, for tests and headless scenes.
    pub fn settle(&mut self) {
        while self.installing.is_some() || self.listing.is_some() {
            if !self.poll() {
                thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
}

fn install(release: &Release, dir: &Path) -> Result<(), String> {
    let parent = dir.parent().expect("the runtime directory has a parent");
    fs::create_dir_all(parent)
        .map_err(|e| format!("Could not create {}: {e}", parent.display()))?;
    let download = dir.with_extension("download");
    let partial = dir.with_extension("partial");
    let _ = fs::remove_file(&download);
    let _ = fs::remove_dir_all(&partial);
    let result = (|| {
        let url = format!("{}/{}", release.base, release.asset);
        run(Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--output",
            ])
            .arg(&download)
            .arg(&url))
        .map_err(|e| format!("Could not download {url}: {e}"))?;
        let bytes = fs::read(&download).map_err(|e| format!("Could not read the download: {e}"))?;
        let sum: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if sum != release.sha256 {
            return Err(format!(
                "The download of {} does not match the checksum built into Velvet Hammer, so it \
                 was not installed (expected {}, got {sum}).",
                release.asset, release.sha256
            ));
        }
        fs::create_dir_all(&partial).map_err(|e| e.to_string())?;
        run(Command::new("tar")
            .arg("-xzf")
            .arg(&download)
            .arg("-C")
            .arg(&partial))
        .map_err(|e| format!("Could not unpack {}: {e}", release.asset))?;
        if !partial.join("pi/pi").is_file() {
            return Err(format!("{} has no pi/pi executable.", release.asset));
        }
        let _ = fs::remove_dir_all(dir);
        fs::rename(&partial, dir).map_err(|e| format!("Could not move pi into place: {e}"))
    })();
    let _ = fs::remove_file(&download);
    if result.is_err() {
        let _ = fs::remove_dir_all(&partial);
    }
    result
}

/// Runs a command, with its error output as the error.
fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn list_models(program: &Path, agent: &Path) -> Result<Vec<String>, String> {
    let out = Command::new(program)
        .arg("--list-models")
        .env("PI_CODING_AGENT_DIR", agent)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not run pi: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(parse_models(&String::from_utf8_lossy(&out.stdout)))
}

/// Reads `pi --list-models`: a header row starting `provider  model`, then one row per model.
/// Anything else, such as "No models available", gives no models.
pub fn parse_models(listing: &str) -> Vec<String> {
    let mut lines = listing.lines();
    let header = lines.next().unwrap_or_default();
    let mut columns = header.split_whitespace();
    if (columns.next(), columns.next()) != (Some("provider"), Some("model")) {
        return Vec::new();
    }
    lines
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            Some(format!("{}/{}", words.next()?, words.next()?))
        })
        .collect()
}

/// Firstmate's pi account setting in `home`.
pub fn pin_path(home: &Path) -> PathBuf {
    home.join("config/pi-account")
}

/// What [`connect`] writes into firstmate's pi account setting: the app's pi directory, then the
/// one provider pi workers may spend.
pub fn pin_text(runtime: &Runtime) -> String {
    format!("{}\n{PROVIDER}\n", runtime.agent_dir().display())
}

/// What firstmate's pi account setting in `home` says now, if it exists.
pub fn current_pin(home: &Path) -> Option<String> {
    fs::read_to_string(pin_path(home)).ok()
}

/// The `auth.json` entry that has pi read the key from the keychain when it needs it.
fn key_entry() -> Value {
    json!({
        "type": "api_key",
        "key": format!("!security find-generic-password -s {KEYCHAIN_SERVICE} -w"),
    })
}

/// Points firstmate's pi workers at the app's pi directory: writes the keychain entry into that
/// directory's `auth.json`, keeping any other provider in it, and writes `config/pi-account` in
/// `home`. Run only from the captain's button on the Workers page.
pub fn connect(runtime: &Runtime, home: &Path) -> Result<(), String> {
    let agent = runtime.agent_dir();
    fs::create_dir_all(&agent).map_err(|e| format!("Could not create {}: {e}", agent.display()))?;
    let auth_path = agent.join("auth.json");
    let mut auth = match fs::read(&auth_path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| format!("{} is not a JSON object.", auth_path.display()))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(format!("Could not read {}: {e}", auth_path.display())),
    };
    auth[PROVIDER] = key_entry();
    let auth_text = serde_json::to_vec_pretty(&auth).map_err(|e| e.to_string())?;
    write_whole(&auth_path, &auth_text, 0o600)?;
    write_whole(&pin_path(home), pin_text(runtime).as_bytes(), 0o644)
}

/// Writes a file whole or not at all, with `mode`.
fn write_whole(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let dir = path.parent().expect("a file in a directory");
    fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let temporary = crate::store::suffixed(path, &format!(".tmp-{}", std::process::id()));
    let written = (|| {
        let mut file = fs::File::create(&temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(mode))?;
        }
        let _ = mode;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written.map_err(|e| format!("Could not write {}: {e}", path.display()))
}

/// A stand-in pi release for tests and headless scenes: a `pi` script that answers `--version`
/// and `--list-models` like pi 1.5.2 does with an opencode Go key, packed the way the official
/// archive is, served from a `file://` URL.
pub mod lab {
    use super::*;

    /// What the stand-in lists: part of what pi 1.5.2 lists for an opencode Go key.
    pub const LISTING: &str = "\
provider     model                       context  max-out  thinking  images
opencode-go  claude-haiku-5-5            1M       128K     yes       yes
opencode-go  deepseek-v4-pro             1M       384K     yes       no
opencode-go  deepseek-v4.1-flash         1M       384K     yes       yes
opencode-go  glm-5.3                     1M       131.1K   yes       no
opencode-go  kimi-k3                     1.0M     131.1K   yes       yes
opencode-go  qwen3.8-max                 1M       131.1K   yes       yes
";

    /// Packs the stand-in under `root` and returns its release. `tamper` changes the archive
    /// after its checksum is taken.
    pub fn release(root: &Path, tamper: bool) -> Release {
        let _ = fs::remove_dir_all(root);
        let staging = root.join("staging/pi");
        fs::create_dir_all(&staging).expect("staging directory");
        let script = format!(
            "#!/bin/sh\n\
             case \"$1\" in\n\
             --version) echo {VERSION} ;;\n\
             --list-models) cat <<'EOF'\n{LISTING}EOF\n ;;\n\
             *) echo \"stand-in pi: $*\" ;;\n\
             esac\n"
        );
        let program = staging.join("pi");
        fs::write(&program, script).expect("write the stand-in pi");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let served = root.join("served");
        fs::create_dir_all(&served).expect("served directory");
        // Named like the official asset for this machine, so the Workers page reads as it will.
        let asset = Release::official().map_or("pi-lab.tar.gz".to_string(), |r| r.asset);
        let asset = asset.as_str();
        run(Command::new("tar")
            .arg("-czf")
            .arg(served.join(asset))
            .arg("-C")
            .arg(root.join("staging"))
            .arg("pi"))
        .expect("pack the stand-in");
        let bytes = fs::read(served.join(asset)).expect("read the archive");
        let sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if tamper {
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(served.join(asset))
                .expect("open the archive");
            file.write_all(b"tampered").expect("tamper");
        }
        Release {
            base: format!("file://{}", served.display()),
            asset: asset.into(),
            sha256,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/pi-lab")
            .join(format!("{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch");
        dir
    }

    #[test]
    fn a_release_is_installed_only_when_its_checksum_matches() {
        let root = scratch("install");
        let mut runtime = Runtime::open(
            &root.join("data"),
            Some(lab::release(&root.join("lab"), false)),
        );
        assert!(!runtime.installed());
        runtime.install();
        runtime.settle();
        assert_eq!(runtime.problem, None);
        assert!(runtime.installed());
        assert_eq!(
            runtime.models().read().unwrap().clone(),
            parse_models(lab::LISTING)
        );
        assert!(
            !root
                .join("data/runtime")
                .join(format!("pi-{VERSION}.partial"))
                .exists()
        );

        let root = scratch("tampered");
        let mut runtime = Runtime::open(
            &root.join("data"),
            Some(lab::release(&root.join("lab"), true)),
        );
        runtime.install();
        runtime.settle();
        let problem = runtime.problem.clone().expect("a refusal");
        assert!(problem.contains("does not match the checksum"), "{problem}");
        assert!(!runtime.installed());
    }

    #[test]
    fn a_failed_download_says_why() {
        let root = scratch("missing");
        let release = Release {
            base: format!("file://{}", root.join("nowhere").display()),
            asset: "pi.tar.gz".into(),
            sha256: "0".repeat(64),
        };
        let mut runtime = Runtime::open(&root, Some(release));
        runtime.install();
        runtime.settle();
        assert!(runtime.problem.unwrap().starts_with("Could not download"));
    }

    #[test]
    fn the_runtime_goes_first_on_the_path() {
        let runtime = Runtime::open(Path::new("/data"), None);
        let path = runtime.path_env(Some("/usr/bin:/bin".into()));
        assert_eq!(
            path,
            OsString::from(format!("/data/runtime/pi-{VERSION}/pi:/usr/bin:/bin"))
        );
    }

    #[test]
    fn the_model_listing_is_read_from_pis_table() {
        let models = parse_models(lab::LISTING);
        assert_eq!(models[0], "opencode-go/claude-haiku-5-5");
        assert!(models.contains(&"opencode-go/deepseek-v4.1-flash".to_string()));
        assert_eq!(models.len(), 6);
        assert!(parse_models("No models available. Use /login\n").is_empty());
    }

    #[test]
    fn connecting_writes_the_keychain_command_and_the_account_setting_only() {
        let root = scratch("connect");
        let (data, home) = (root.join("data"), root.join("home"));
        let runtime = Runtime::open(&data, None);
        fs::create_dir_all(runtime.agent_dir()).unwrap();
        fs::write(
            runtime.agent_dir().join("auth.json"),
            r#"{"openrouter": {"type": "api_key", "key": "$OPENROUTER_API_KEY"}}"#,
        )
        .unwrap();
        connect(&runtime, &home).unwrap();
        let auth: Value =
            serde_json::from_slice(&fs::read(runtime.agent_dir().join("auth.json")).unwrap())
                .unwrap();
        assert_eq!(
            auth[PROVIDER]["key"],
            "!security find-generic-password -s velvet-hammer-opencode-go -w"
        );
        assert_eq!(auth["openrouter"]["key"], "$OPENROUTER_API_KEY");
        assert_eq!(
            current_pin(&home).unwrap(),
            format!("{}\nopencode-go\n", data.join("pi-agent").display())
        );
        let config: Vec<_> = fs::read_dir(home.join("config"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(config, ["pi-account"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(runtime.agent_dir().join("auth.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
