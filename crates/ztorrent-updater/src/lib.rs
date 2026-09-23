//! Checks GitHub for a newer release, downloads the build for this platform, and
//! swaps it in on the next launch -- the port of electron/updater.js.
//!
//! These builds are unsigned, so the platform update services are out: both
//! want a signature before they replace an application. What is left is what
//! the curl installer does by hand: fetch the same artifact from the same release
//! and put it where the running copy lives. The swap cannot happen while the app
//! holds its own files open, so a small detached script waits for this process
//! to exit, moves the old copy aside, puts the new one in place and launches it.
//! Moving rather than deleting means a failed copy is rolled back: the worst case
//! is the old version starting again, never no version at all.
//!
//! Nothing is downloaded until a check finds a genuinely newer version, and
//! nothing is swapped until the user asks. Nothing is staged unless it matches
//! the SHA-256 digest GitHub publishes for the asset.
//!
//! Every request goes through the client the app hands over, which follows the
//! user's proxy and interface binding: an update check must not be the one
//! thing that leaves by another route.
//!
//! The artifact naming here must agree with scripts/install.sh.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use sha2::{Digest, Sha256};
use std::time::{Duration, Instant};
use ztorrent_core::update::*;
use ztorrent_core::version::compare_versions;

const REPO: &str = "odexion/ztorrent";
const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(20);
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);

/// The artifact this platform can install, the way the release names it. Each
/// packager spells the architecture in its own convention -- x64 in the dmg,
/// x86_64 in the AppImage, amd64 in the deb -- so every spelling is tried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WantedAsset {
    pub os: &'static str,
    pub ext: &'static str,
    pub arches: &'static [&'static str],
}

pub fn wanted_asset(os: &str, arch: &str, appimage: bool) -> Option<WantedAsset> {
    let arches: &'static [&'static str] = if arch == "aarch64" || arch == "arm64" { &["arm64", "aarch64"] } else { &["x64", "x86_64", "amd64"] };
    match os {
        "macos" => Some(WantedAsset { os: "mac", ext: "dmg", arches }),
        "windows" => Some(WantedAsset { os: "win", ext: "exe", arches }),
        // Only the AppImage is a single file that can be dropped in place; a .deb
        // belongs to the package manager.
        "linux" if appimage => Some(WantedAsset { os: "linux", ext: "AppImage", arches }),
        _ => None,
    }
}

/// Picks an asset anchored at the end of its name, so `-mac-x64.dmg` never
/// matches the `.dmg.blockmap` published beside it.
pub fn pick_asset<'a>(want: &WantedAsset, names: &'a [(String, String, u64)]) -> Option<&'a (String, String, u64)> {
    want.arches
        .iter()
        .find_map(|arch| names.iter().find(|(name, _, _)| name.ends_with(&format!("-{}-{arch}.{}", want.os, want.ext))))
}

/// Builds the HTTP client for one request. Built per request, as the engine's
/// own is, so a bound interface's address is read when it is used.
pub type ClientFactory = Box<dyn Fn() -> Result<reqwest::blocking::Client, String> + Send>;

/// A client that goes straight out, ignoring any proxy in the environment. For
/// tests; the app hands over one that follows its egress policy.
pub fn direct_client(version: &str) -> ClientFactory {
    let agent = format!("ztorrent/{version}");
    Box::new(move || reqwest::blocking::Client::builder().no_proxy().user_agent(agent.clone()).build().map_err(|e| e.to_string()))
}

/// A test override from the environment. Only a debug build or one built with
/// the `update-testing` feature reads these: in a shipped build they would let
/// whoever sets the environment choose what gets installed.
fn test_override(name: &str) -> Option<String> {
    if cfg!(any(debug_assertions, feature = "update-testing")) { std::env::var(name).ok() } else { None }
}

/// The hex SHA-256 GitHub publishes for an asset, as "sha256:<hex>".
pub fn asset_digest(release: &serde_json::Value, name: &str) -> Option<String> {
    release["assets"]
        .as_array()?
        .iter()
        .find(|a| a["name"].as_str() == Some(name))?["digest"]
        .as_str()?
        .strip_prefix("sha256:")
        .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|h| h.to_ascii_lowercase())
}

pub struct Options {
    pub dir: PathBuf,
    pub current: String,
    pub events: flume::Sender<UpdateStatus>,
    /// Refuses to apply anything unless this is an installed build.
    pub packaged: bool,
    pub client: ClientFactory,
}

pub struct Updater {
    repo: String,
    dir: PathBuf,
    staged_dir: PathBuf,
    status: UpdateStatus,
    events: flume::Sender<UpdateStatus>,
    packaged: bool,
    automatic: bool,
    next_check: Option<Instant>,
    client: ClientFactory,
    /// The published digest of the asset on offer.
    digest: Option<String>,
}

/// Starts the updater thread and returns its command channel.
pub fn spawn(options: Options) -> flume::Sender<UpdateCommand> {
    let (tx, rx) = flume::unbounded();
    std::thread::Builder::new()
        .name("ztorrent-updater".into())
        .spawn(move || {
            let mut updater = Updater::new(options);
            updater.resume();
            updater.run(rx);
        })
        .expect("updater thread");
    tx
}

impl Updater {
    pub fn new(options: Options) -> Updater {
        let current = std::env::var("ZTORRENT_UPDATE_PRETEND_VERSION").unwrap_or(options.current);
        let want = wanted_asset(std::env::consts::OS, std::env::consts::ARCH, std::env::var_os("APPIMAGE").is_some());
        Updater {
            repo: test_override("ZTORRENT_UPDATE_REPO").unwrap_or_else(|| REPO.into()),
            staged_dir: options.dir.join("staged"),
            dir: options.dir,
            status: UpdateStatus { installable: want.is_some(), current, ..Default::default() },
            events: options.events,
            packaged: options.packaged,
            automatic: false,
            next_check: None,
            client: options.client,
            digest: None,
        }
    }

    fn set(&mut self, f: impl FnOnce(&mut UpdateStatus)) {
        f(&mut self.status);
        let _ = self.events.send(self.status.clone());
    }

    pub fn status(&self) -> UpdateStatus {
        self.status.clone()
    }

    fn run(&mut self, rx: flume::Receiver<UpdateCommand>) {
        loop {
            let timeout = self.next_check.map(|t| t.saturating_duration_since(Instant::now())).unwrap_or(Duration::from_secs(3600));
            match rx.recv_timeout(timeout) {
                Ok(UpdateCommand::Check { manual, reply }) => {
                    let status = self.check(manual);
                    if let Some(r) = reply {
                        let _ = r.send(status);
                    }
                }
                Ok(UpdateCommand::Download) => {
                    self.download();
                }
                Ok(UpdateCommand::Apply(reply)) => {
                    let _ = reply.send(self.apply_and_restart());
                }
                Ok(UpdateCommand::SetAutomatic(on)) => {
                    self.automatic = on && self.packaged;
                    self.next_check = self.automatic.then(|| Instant::now() + FIRST_CHECK_AFTER);
                }
                Ok(UpdateCommand::Status(reply)) => {
                    let _ = reply.send(self.status());
                }
                Err(flume::RecvTimeoutError::Timeout) => {
                    if self.automatic && self.next_check.is_some_and(|t| Instant::now() >= t) {
                        self.next_check = Some(Instant::now() + CHECK_EVERY);
                        self.check(false);
                    }
                }
                Err(flume::RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    /// Leftovers from an update that was downloaded but never applied.
    pub fn reset(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    /// Picks up a build staged by an earlier run, so quitting before the restart
    /// does not cost a second 100MB download. A staging directory for a version
    /// already running is the leftover of an update that went through.
    pub fn resume(&mut self) {
        let Ok(text) = std::fs::read_to_string(self.dir.join("staged.json")) else { return };
        let staged: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let version = staged["version"].as_str().unwrap_or_default().to_string();
        let payload = staged["payload"].as_str().map(PathBuf::from);
        let newer = !version.is_empty() && compare_versions(&version, &self.status.current).is_gt();
        match payload {
            Some(p) if newer && p.exists() => self.set(|s| {
                s.state = UpdateState::Ready;
                s.version = Some(version);
                s.file = Some(p);
            }),
            _ => self.reset(),
        }
    }

    fn api(&self, url: &str) -> Result<serde_json::Value, String> {
        // A connection that stalls rather than refuses -- a captive portal, a
        // route that went away -- would otherwise leave the check pending forever.
        let client = (self.client)()?;
        let res = client.get(url).timeout(CHECK_TIMEOUT).header("Accept", "application/vnd.github+json").send().map_err(|e| {
            if e.is_timeout() { "GitHub did not answer".to_string() } else { e.to_string() }
        })?;
        if !res.status().is_success() {
            return Err(format!("GitHub returned {}", res.status().as_u16()));
        }
        let text = res.text().map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    pub fn check(&mut self, manual: bool) -> UpdateStatus {
        let state = self.status.state;
        if matches!(state, UpdateState::Downloading | UpdateState::Staging) {
            return self.status();
        }
        // A check already running, or a build ready to go: only someone asking starts another.
        if matches!(state, UpdateState::Checking | UpdateState::Ready) && !manual {
            return self.status();
        }
        let staged = (state == UpdateState::Ready).then(|| (self.status.version.clone(), self.status.file.clone()));

        self.set(|s| {
            s.state = UpdateState::Checking;
            s.error = None;
            s.error_from = None;
            s.manual = manual;
        });
        let feed = test_override("ZTORRENT_UPDATE_FEED").unwrap_or_else(|| format!("https://api.github.com/repos/{}/releases/latest", self.repo));
        let result = self.api(&feed).and_then(|release| {
            let version = release["tag_name"].as_str().unwrap_or_default().trim_start_matches('v').to_string();
            if version.is_empty() { Err("the release has no tag".to_string()) } else { Ok((version, release)) }
        });

        match result {
            Ok((version, _)) if !compare_versions(&version, &self.status.current).is_gt() => self.set(|s| {
                s.state = UpdateState::Idle;
                s.version = None;
                s.url = None;
                s.size = 0;
            }),
            // The build already staged is the one on offer: say so again rather
            // than throwing the payload away.
            Ok((version, _)) if staged.as_ref().is_some_and(|(v, _)| v.as_deref() == Some(version.as_str())) => {
                let file = staged.and_then(|(_, f)| f);
                self.set(|s| {
                    s.state = UpdateState::Ready;
                    s.version = Some(version);
                    s.file = file;
                })
            }
            Ok((version, release)) => {
                if staged.is_some() {
                    self.reset();
                }
                let assets: Vec<(String, String, u64)> = release["assets"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|x| {
                                (
                                    x["name"].as_str().unwrap_or_default().to_string(),
                                    x["browser_download_url"].as_str().unwrap_or_default().to_string(),
                                    x["size"].as_u64().unwrap_or(0),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let want = wanted_asset(std::env::consts::OS, std::env::consts::ARCH, std::env::var_os("APPIMAGE").is_some());
                let asset = want.as_ref().and_then(|w| pick_asset(w, &assets)).cloned();
                self.digest = asset.as_ref().and_then(|a| asset_digest(&release, &a.0));
                self.set(|s| {
                    s.state = UpdateState::Available;
                    s.version = Some(version);
                    s.url = asset.as_ref().map(|a| a.1.clone());
                    s.size = asset.as_ref().map(|a| a.2).unwrap_or(0);
                    s.received = 0;
                    s.file = None;
                    s.installable = asset.is_some();
                })
            }
            // A check that could not reach the network must not cost a build that
            // is already staged and waiting.
            Err(err) => match staged {
                Some((version, file)) => self.set(|s| {
                    s.state = UpdateState::Ready;
                    s.version = version;
                    s.file = file;
                    s.error = Some(err);
                    s.error_from = Some(UpdateStep::Check);
                }),
                None => self.set(|s| {
                    s.state = UpdateState::Error;
                    s.error = Some(err);
                    s.error_from = Some(UpdateStep::Check);
                }),
            },
        }
        self.status()
    }

    pub fn download(&mut self) -> UpdateStatus {
        if self.status.state != UpdateState::Available {
            return self.status();
        }
        let (Some(url), Some(version)) = (self.status.url.clone(), self.status.version.clone()) else { return self.status() };
        let name = url.rsplit('/').next().unwrap_or("update").to_string();
        let part = self.dir.join(format!("{name}.part"));
        let dest = self.dir.join(&name);
        self.set(|s| {
            s.state = UpdateState::Downloading;
            s.received = 0;
            s.error = None;
            s.error_from = None;
        });

        let result = (|| -> Result<(), String> {
            // Checked before a byte is fetched: a build nobody can verify is not installed.
            let expected = self.digest.clone().ok_or("the release publishes no checksum for this build, so it cannot be verified")?;
            std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
            let client = (self.client)()?;
            let mut res = client.get(&url).send().map_err(|e| e.to_string())?;
            if !res.status().is_success() {
                return Err(format!("download returned {}", res.status().as_u16()));
            }
            let total = res.content_length().unwrap_or(self.status.size);
            if total > 0 {
                self.set(|s| s.size = total);
            }
            let mut out = std::fs::File::create(&part).map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 1 << 16];
            let (mut received, mut painted) = (0u64, 0u64);
            let mut hasher = Sha256::new();
            loop {
                let n = res.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                hasher.update(&buf[..n]);
                received += n as u64;
                // A progress readout cannot usefully change more often than this.
                if received - painted > 262_144 {
                    painted = received;
                    self.set(|s| s.received = received);
                }
            }
            out.sync_all().map_err(|e| e.to_string())?;
            if total > 0 && received != total {
                return Err(format!("expected {total} bytes, got {received}"));
            }
            let actual: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
            if actual != expected {
                return Err(format!("checksum mismatch: expected {expected}, got {actual}. Not installing it."));
            }
            std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
            self.set(|s| {
                s.received = received;
                s.file = Some(dest.clone());
            });
            self.stage(&dest, &version)
        })();

        if let Err(err) = result {
            let _ = std::fs::remove_file(&part);
            self.set(|s| {
                s.state = UpdateState::Error;
                s.error = Some(err);
                s.error_from = Some(UpdateStep::Download);
            });
        }
        self.status()
    }

    /// Turns the artifact into something the swap can move in one step. The .app
    /// comes out of the disk image now, while a failure can still be reported.
    fn stage(&mut self, file: &Path, version: &str) -> Result<(), String> {
        self.set(|s| s.state = UpdateState::Staging);
        let _ = std::fs::remove_dir_all(&self.staged_dir);
        std::fs::create_dir_all(&self.staged_dir).map_err(|e| e.to_string())?;
        let payload = if cfg!(target_os = "macos") {
            let app = self.extract_app(file)?;
            let _ = std::fs::remove_file(file);
            app
        } else if cfg!(target_os = "linux") {
            let dest = self.staged_dir.join("ztorrent.AppImage");
            std::fs::copy(file, &dest).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
            }
            let _ = std::fs::remove_file(file);
            dest
        } else {
            file.to_path_buf()
        };
        let staged = serde_json::json!({ "version": version, "payload": payload });
        std::fs::write(self.dir.join("staged.json"), serde_json::to_string_pretty(&staged).unwrap_or_default()).map_err(|e| e.to_string())?;
        self.set(|s| {
            s.state = UpdateState::Ready;
            s.version = Some(version.to_string());
            s.file = Some(payload);
        });
        Ok(())
    }

    fn extract_app(&self, dmg: &Path) -> Result<PathBuf, String> {
        detach_image(dmg);
        let mnt = std::env::temp_dir().join(format!("ztorrent-update-{}", std::process::id()));
        std::fs::create_dir_all(&mnt).map_err(|e| e.to_string())?;
        let result = (|| {
            run("hdiutil", &["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint", &mnt.to_string_lossy(), &dmg.to_string_lossy()])?;
            let app = std::fs::read_dir(&mnt)
                .map_err(|e| e.to_string())?
                .flatten()
                .find(|e| e.file_name().to_string_lossy().ends_with(".app"))
                .ok_or("no application inside the disk image")?;
            let dest = self.staged_dir.join(app.file_name());
            // ditto keeps a bundle's symlinks, permissions and extended attributes.
            run("ditto", &[&app.path().to_string_lossy(), &dest.to_string_lossy()])?;
            Ok(dest)
        })();
        // Detached before anything else, success or not: an image left attached is
        // what breaks the next attempt.
        let _ = run("hdiutil", &["detach", &mnt.to_string_lossy(), "-force"]);
        let _ = std::fs::remove_dir(&mnt);
        result
    }

    /// Where the running application lives, as something the swap can replace.
    pub fn install_target() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        if cfg!(target_os = "macos") {
            let s = exe.to_string_lossy();
            let i = s.find(".app/")?;
            return Some(PathBuf::from(&s[..i + 4]));
        }
        if cfg!(target_os = "linux") {
            return std::env::var_os("APPIMAGE").map(PathBuf::from);
        }
        Some(exe)
    }

    /// Writes the hand-off script and starts it detached. False when there is
    /// nothing to apply or this is not an installed build.
    pub fn apply_and_restart(&mut self) -> bool {
        // A development build runs out of a build directory; never swap that.
        if !self.packaged || self.status.state != UpdateState::Ready {
            return false;
        }
        let (Some(payload), Some(target)) = (self.status.file.clone(), Self::install_target()) else { return false };
        if cfg!(windows) {
            return self.run_installer(&payload, &target);
        }
        let script = self.dir.join("apply.sh");
        if std::fs::write(&script, swap_script(&payload, &target, &self.staged_dir, &self.dir, cfg!(target_os = "macos"))).is_err() {
            return false;
        }
        Process::new("/bin/sh")
            .arg(&script)
            .arg(std::process::id().to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
    }

    /// The release page, for platforms this cannot install for itself.
    pub fn release_page(&self) -> String {
        match &self.status.version {
            Some(v) => format!("https://github.com/{}/releases/tag/v{v}", self.repo),
            None => format!("https://github.com/{}/releases/latest", self.repo),
        }
    }

    /// Windows: the installer replaces the app itself. This copy has to exit
    /// first -- on the way out it writes its state and resume data, which the
    /// installer's own kill would skip -- so a hidden script waits for it.
    #[cfg(windows)]
    fn run_installer(&self, payload: &Path, target: &Path) -> bool {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let script = self.dir.join("apply.ps1");
        if std::fs::write(&script, installer_script(payload, target, &self.staged_dir, &self.dir)).is_err() {
            return false;
        }
        Process::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File"])
            .arg(&script)
            .arg(std::process::id().to_string())
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .is_ok()
    }

    #[cfg(not(windows))]
    fn run_installer(&self, _payload: &Path, _target: &Path) -> bool {
        false
    }
}

fn ps(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "''"))
}

/// The Windows counterpart of [`swap_script`]: waits for the running copy, runs
/// the installer silently, and cleans up. `/R` tells the installer to start the
/// new version once it is in place.
///
/// An installer carrying a signature that does not check out -- altered after
/// it was signed, or signed by nobody Windows trusts -- is not run, and the
/// version already installed starts again instead. An unsigned one is allowed:
/// releases are signed only where signing is set up, and its digest was checked.
pub fn installer_script(payload: &Path, target: &Path, staged: &Path, dir: &Path) -> String {
    format!(
        r#"# Written by ztorrent to finish an update. Safe to delete.
param([string]$AppPid = '')

# Anything that is not a real pid skips the wait rather than sitting through it.
$id = 0
if ([int]::TryParse($AppPid, [ref]$id) -and $id -gt 0) {{
  Wait-Process -Id $id -Timeout 60 -ErrorAction SilentlyContinue
}}

$installer = {p}
if (Test-Path -LiteralPath $installer) {{
  $status = (Get-AuthenticodeSignature -LiteralPath $installer).Status
  if ($status -eq 'Valid' -or $status -eq 'NotSigned') {{
    Start-Process -FilePath $installer -ArgumentList '/S', '/R' -Wait
  }} else {{
    Start-Process -FilePath {t}
  }}
  Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue
}}
Remove-Item -LiteralPath {staged}, {json} -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath $PSCommandPath -Force -ErrorAction SilentlyContinue
"#,
        p = ps(payload),
        t = ps(target),
        staged = ps(staged),
        json = ps(&dir.join("staged.json"))
    )
}

fn run(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = Process::new(cmd).args(args).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!("{cmd} failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Ejects anything still holding this image; a run killed between attach and
/// detach otherwise wedges every later attempt with "Resource busy".
fn detach_image(dmg: &Path) {
    let Ok(info) = run("hdiutil", &["info"]) else { return };
    let wanted = std::fs::canonicalize(dmg).unwrap_or_else(|_| dmg.to_path_buf());
    for block in info.split("================================================") {
        let image = block.lines().find_map(|l| l.strip_prefix("image-path").map(|r| r.trim_start_matches([' ', ':', '\t']).trim().to_string()));
        if image.map(PathBuf::from).is_some_and(|p| p == wanted) {
            if let Some(dev) = block.lines().find_map(|l| l.split_whitespace().next().filter(|w| w.starts_with("/dev/disk"))) {
                let _ = run("hdiutil", &["detach", dev, "-force"]);
            }
        }
    }
}

fn q(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

/// The same script electron/updater.js wrote.
pub fn swap_script(payload: &Path, target: &Path, staged: &Path, dir: &Path, mac: bool) -> String {
    let swap = if mac {
        format!(
            r#"
backup={t}.old.$$
if mv {t} "$backup" 2>/dev/null; then
  if ditto {p} {t}; then
    /usr/bin/xattr -dr com.apple.quarantine {t} 2>/dev/null || true
    rm -rf "$backup"
  else
    # Put back exactly what was there. A failed update must not cost anyone
    # the version they already had.
    rm -rf {t}
    mv "$backup" {t}
  fi
fi
open {t}
"#,
            t = q(target),
            p = q(payload)
        )
    } else {
        format!(
            r#"
if cp -f {p} {t}.new 2>/dev/null; then
  chmod +x {t}.new
  mv -f {t}.new {t}
fi
{t} >/dev/null 2>&1 &
"#,
            t = q(target),
            p = q(payload)
        )
    };
    format!(
        r#"#!/bin/sh
# Written by ztorrent to finish an update. Safe to delete.
#
# Waits for the running copy to exit -- it cannot be replaced while it holds its
# own files open -- then swaps the new build in and starts it again.
pid="${{1:-}}"
# A kill -0 aimed at pid 0 signals our own process group and always succeeds,
# so anything that is not a real pid has to skip the wait rather than sit
# through the whole of it.
case "$pid" in ''|0|*[!0-9]*) pid="" ;; esac

i=0
while [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null && [ "$i" -lt 600 ]; do
  sleep 0.1
  i=$((i + 1))
done
{swap}
rm -rf {staged} {json}
rm -f "$0"
"#,
        staged = q(staged),
        json = q(&dir.join("staged.json"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets(names: &[&str]) -> Vec<(String, String, u64)> {
        names.iter().map(|n| (n.to_string(), format!("https://x/{n}"), 1)).collect()
    }

    #[test]
    fn matches_every_arch_spelling_and_never_the_blockmap() {
        let list = assets(&[
            "ztorrent-0.5.0-mac-x64.dmg.blockmap",
            "ztorrent-0.5.0-mac-x64.dmg",
            "ztorrent-0.5.0-mac-arm64.dmg",
            "ztorrent-0.5.0-win-x64.exe",
            "ztorrent-0.5.0-linux-x86_64.AppImage",
            "ztorrent-0.5.0-linux-amd64.deb",
        ]);
        let pick = |os, arch, appimage| wanted_asset(os, arch, appimage).and_then(|w| pick_asset(&w, &list).map(|a| a.0.clone()));
        assert_eq!(pick("macos", "x86_64", false).as_deref(), Some("ztorrent-0.5.0-mac-x64.dmg"));
        assert_eq!(pick("macos", "aarch64", false).as_deref(), Some("ztorrent-0.5.0-mac-arm64.dmg"));
        assert_eq!(pick("windows", "x86_64", false).as_deref(), Some("ztorrent-0.5.0-win-x64.exe"));
        assert_eq!(pick("linux", "x86_64", true).as_deref(), Some("ztorrent-0.5.0-linux-x86_64.AppImage"));
        assert_eq!(pick("linux", "x86_64", false), None, "a .deb install is told about the release, not handed one");
        assert_eq!(pick("windows", "aarch64", false), None);
    }

    fn updater(dir: &Path, current: &str) -> (Updater, flume::Receiver<UpdateStatus>) {
        let (tx, rx) = flume::unbounded();
        (Updater::new(Options { dir: dir.to_path_buf(), current: current.into(), events: tx, packaged: false, client: direct_client(current) }), rx)
    }

    #[test]
    fn resume_offers_a_newer_staged_build_and_discards_a_stale_one() {
        let dir = tempfile::tempdir().unwrap();
        let updates = dir.path().join("updates");
        std::fs::create_dir_all(updates.join("staged")).unwrap();
        let payload = updates.join("staged/ztorrent.app");
        std::fs::create_dir_all(&payload).unwrap();
        std::fs::write(updates.join("staged.json"), serde_json::json!({"version": "0.6.0", "payload": payload}).to_string()).unwrap();

        let (mut u, _) = updater(&updates, "0.5.0");
        u.resume();
        assert_eq!(u.status().state, UpdateState::Ready);
        assert_eq!(u.status().version.as_deref(), Some("0.6.0"));
        assert!(!u.apply_and_restart(), "an unpackaged build never swaps");

        let (mut same, _) = updater(&updates, "0.6.0");
        same.resume();
        assert_eq!(same.status().state, UpdateState::Idle);
        assert!(!updates.exists(), "the leftover of an update that went through is removed");
    }

    #[test]
    fn the_swap_script_quotes_paths_and_skips_bad_pids() {
        let s = swap_script(Path::new("/tmp/it's/new.app"), Path::new("/Applications/ztorrent.app"), Path::new("/u/staged"), Path::new("/u"), true);
        assert!(s.contains("'/tmp/it'\\''s/new.app'"));
        assert!(s.contains("case \"$pid\" in ''|0|*[!0-9]*) pid=\"\" ;; esac"));
        assert!(s.contains("/usr/bin/xattr -dr com.apple.quarantine '/Applications/ztorrent.app'"));
        assert!(s.contains("mv \"$backup\" '/Applications/ztorrent.app'"), "rolls back a failed copy");
    }

    #[test]
    fn the_windows_script_quotes_paths_waits_and_restarts() {
        let s = installer_script(
            Path::new(r"C:\Users\o'neil\AppData\Roaming\ztorrent\updates\ztorrent-0.6.0-win-x64.exe"),
            Path::new(r"C:\Users\o'neil\AppData\Local\ztorrent\ztorrent.exe"),
            Path::new(r"C:\u\staged"),
            Path::new(r"C:\u"),
        );
        assert!(s.contains(r"$installer = 'C:\Users\o''neil\AppData\Roaming\ztorrent\updates\ztorrent-0.6.0-win-x64.exe'"));
        assert!(s.contains("[int]::TryParse($AppPid, [ref]$id) -and $id -gt 0"), "a pid that is not one skips the wait");
        assert!(s.contains("Wait-Process -Id $id"), "the app exits, and saves, before the installer runs");
        assert!(s.contains("-ArgumentList '/S', '/R' -Wait"), "silent, and starts the new version");
        assert!(s.contains("$status -eq 'Valid' -or $status -eq 'NotSigned'"), "a broken signature is never run");
        assert!(s.contains(r"Start-Process -FilePath 'C:\Users\o''neil\AppData\Local\ztorrent\ztorrent.exe'"), "the old version starts instead");
    }

    /// The three outcomes the Electron commit was verified against: a swap that
    /// works, a copy that fails and is rolled back, and a pid that is not a pid.
    #[cfg(unix)]
    #[test]
    fn the_linux_swap_script_runs() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("ztorrent.AppImage");
        let payload = dir.path().join("new.AppImage");
        std::fs::write(&target, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::write(&payload, "#!/bin/sh\necho new\n").unwrap();
        let script = dir.path().join("apply.sh");
        std::fs::write(&script, swap_script(&payload, &target, &dir.path().join("staged"), dir.path(), false)).unwrap();
        let status = Process::new("/bin/sh").arg(&script).arg("garbage").status().unwrap();
        assert!(status.success());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "#!/bin/sh\necho new\n", "swapped in");
        assert!(!script.exists(), "the script removes itself");

        let missing = dir.path().join("does-not-exist");
        std::fs::write(&script, swap_script(&missing, &target, &dir.path().join("staged"), dir.path(), false)).unwrap();
        Process::new("/bin/sh").arg(&script).arg("0").status().unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "#!/bin/sh\necho new\n", "a failed copy leaves the old build");
    }

    #[test]
    fn reads_the_published_digest_and_nothing_malformed() {
        let hex = "ab".repeat(32);
        let release = serde_json::json!({ "assets": [
            { "name": "ztorrent-0.6.0-mac-arm64.dmg", "digest": format!("sha256:{}", hex.to_uppercase()) },
            { "name": "ztorrent-0.6.0-mac-x64.dmg", "digest": "sha256:short" },
            { "name": "ztorrent-0.6.0-win-x64.exe" },
        ]});
        assert_eq!(asset_digest(&release, "ztorrent-0.6.0-mac-arm64.dmg"), Some(hex));
        assert_eq!(asset_digest(&release, "ztorrent-0.6.0-mac-x64.dmg"), None);
        assert_eq!(asset_digest(&release, "ztorrent-0.6.0-win-x64.exe"), None);
    }

    /// A tiny release served from loopback: the download passes only when its
    /// bytes match the digest, and never when no digest is published. (What
    /// happens after -- staging -- is per platform; macOS wants a real .dmg.)
    #[test]
    fn a_download_is_accepted_only_when_it_matches_its_digest() {
        use std::net::TcpListener;
        let body = b"#!/bin/sh\necho new\n".to_vec();
        let good: String = Sha256::digest(&body).iter().map(|b| format!("{b:02x}")).collect();
        for (digest, refusal) in [(Some(good), None), (Some("00".repeat(32)), Some("checksum mismatch")), (None, Some("publishes no checksum"))] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/ztorrent-0.6.0-linux-x86_64.AppImage", listener.local_addr().unwrap());
            let served = body.clone();
            std::thread::spawn(move || {
                let (mut conn, _) = listener.accept().unwrap();
                let mut req = [0u8; 1024];
                let _ = conn.read(&mut req);
                let _ = write!(conn, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", served.len());
                let _ = conn.write_all(&served);
            });
            let dir = tempfile::tempdir().unwrap();
            let (mut u, _) = updater(dir.path(), "0.5.0");
            u.status.state = UpdateState::Available;
            u.status.version = Some("0.6.0".into());
            u.status.url = Some(url);
            u.digest = digest;
            let s = u.download();
            let error = s.error.clone().unwrap_or_default();
            match refusal {
                Some(why) => {
                    assert!(error.contains(why), "{error}");
                    assert_eq!(s.error_from, Some(UpdateStep::Download));
                    assert!(!dir.path().join("staged.json").exists(), "nothing staged");
                }
                None => {
                    assert!(!error.contains("checksum"), "{error}");
                    if !cfg!(target_os = "macos") {
                        assert_eq!(s.state, UpdateState::Ready, "{error}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_failed_check_keeps_the_staged_build() {
        // SAFETY: this test alone reads the variable, before any other thread starts.
        unsafe { std::env::set_var("ZTORRENT_UPDATE_FEED", "http://127.0.0.1:1/feed.json") };
        let dir = tempfile::tempdir().unwrap();
        let (mut u, _) = updater(dir.path(), "0.5.0");
        u.status.state = UpdateState::Ready;
        u.status.version = Some("0.6.0".into());
        u.status.file = Some(dir.path().join("x"));
        let s = u.check(true);
        assert_eq!(s.state, UpdateState::Ready);
        assert_eq!(s.error_from, Some(UpdateStep::Check));
        let (mut fresh, _) = updater(dir.path(), "0.5.0");
        assert_eq!(fresh.check(false).state, UpdateState::Error);
    }
}
