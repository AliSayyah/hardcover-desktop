//! Self-update from GitHub releases.
//!
//! The app updates itself in two kinds of install ([`InstallKind`]):
//! - an unpacked release tarball (`<prefix>/bin/hardcover-desktop` next to
//!   `<prefix>/share/applications/…`) in a prefix it can write to, such as
//!   `~/.local`, where `install.sh` puts it;
//! - an `AppImage` in a directory it can write to.
//!
//! Other installs only learn that a newer version exists: a distribution
//! package, a development build, or files this user can't replace.
//!
//! Trust chain for an install:
//! 1. The release file (tarball or `AppImage`) must be signed by the key in
//!    `packaging/minisign.pub`.
//! 2. The signature's trusted comment names the signed file, and that name must
//!    be the file name for the advertised version and this architecture. A
//!    valid signature for an older release therefore can't be replayed as a
//!    newer one.
//! 3. The advertised version must be newer than the running one.
//! 4. A tarball's top-level directory must carry the same name as the tarball,
//!    and only regular files under `bin/hardcover-desktop` and `share/` are
//!    unpacked, with plain relative paths and a total size cap. An `AppImage`
//!    must carry the `AppImage` type 2 magic bytes.
//!
//! Installing holds the same `flock` that `install.sh` takes, on the prefix
//! (or the `AppImage`'s directory), so two installs never interleave. Every
//! file is first written beside its destination, in the destination's own
//! directory, so the rename that replaces it stays on one filesystem even when
//! `bin/` or `share/` is a link to another one; only once all of them are
//! written are they renamed into place, the binary last, and the directories
//! synced. A failure before then changes nothing. The temporary files share a
//! name prefix and are made only under the lock, so the next install removes
//! any that a crash left behind. An `AppImage` is replaced the same way,
//! keeping its path (and so its name, launchers and the restart path).
//! Replacing a running executable or a mounted `AppImage` is safe on Linux;
//! the new version runs after a restart.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::fmt;
use std::fs;
use std::io::{Read, Write as _};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::http::{HttpRequest, HttpTransport, Method, UreqTransport};
use crate::{Error, Result};

/// The GitHub repository releases are published to.
pub const REPOSITORY: &str = "AliSayyah/hardcover-desktop";

/// The release signing public key (minisign, base64).
///
/// # Panics
///
/// Panics if the bundled signing-key file has no key line.
pub fn release_public_key() -> &'static str {
    let file = include_str!("../../../packaging/minisign.pub");
    file.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
        .expect("packaging/minisign.pub has a key line")
}

const APP_ID: &str = "io.github.alisayyah.HardcoverDesktop";
const BINARY: &str = "hardcover-desktop";
const MAX_TARBALL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_APPIMAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;
/// The name prefix of the files an install writes beside their destinations.
/// `install.sh` uses it too, and both make such files only under the lock.
const TEMP_PREFIX: &str = ".hardcover-desktop-update-";
/// How long an install waits for another one (or `install.sh`) to finish.
const LOCK_WAIT: Duration = Duration::from_secs(60);

// ----- versions -----

/// A `major.minor.patch` version with an optional pre-release suffix, which
/// sorts before the release it precedes. Build metadata is ignored.
///
/// Pre-releases order as Semantic Versioning says: identifier by identifier,
/// numeric ones as numbers (`rc.2` before `rc.10`) and before alphanumeric
/// ones, and a longer list after a shorter one it starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
}

impl Version {
    /// Parses `1.2.3`, `v1.2.3` or `1.2.3-rc.1`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().strip_prefix('v').unwrap_or(s.trim());
        let s = s.split('+').next()?;
        let (core, pre) = match s.split_once('-') {
            Some((core, pre)) if pre.split('.').all(is_pre_identifier) => {
                (core, Some(pre.to_owned()))
            }
            Some(_) => return None,
            None => (s, None),
        };
        let mut parts = core.split('.');
        let mut next = || -> Option<u64> {
            let p = parts.next()?;
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            p.parse().ok()
        };
        let v = Self {
            major: next()?,
            minor: next()?,
            patch: next()?,
            pre,
        };
        parts.next().is_none().then_some(v)
    }

    /// The version of this build of `hardcover-core`'s caller, from its Cargo
    /// version (pass `env!("CARGO_PKG_VERSION")`).
    ///
    /// # Panics
    ///
    /// Panics if the supplied Cargo version cannot be parsed.
    #[must_use]
    pub fn current(cargo_version: &str) -> Self {
        Self::parse(cargo_version).expect("CARGO_PKG_VERSION is a valid version")
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a
                    .split('.')
                    .map(PreIdentifier::of)
                    .cmp(b.split('.').map(PreIdentifier::of)),
            })
    }
}

/// A pre-release identifier: ASCII letters, digits and `-`, not empty, and
/// without a leading zero when numeric. Leading zeros are refused so that
/// equal versions are also equal strings (`rc.01` is not `rc.1`).
fn is_pre_identifier(id: &str) -> bool {
    !id.is_empty()
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !(id.len() > 1 && id.starts_with('0') && id.bytes().all(|b| b.is_ascii_digit()))
}

/// One pre-release identifier, ordered as Semantic Versioning says: numeric
/// identifiers by value (compared by length, then digits, so any length
/// works), before every alphanumeric one, which compare as ASCII.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum PreIdentifier<'a> {
    Numeric(usize, &'a str),
    Alphanumeric(&'a str),
}

impl<'a> PreIdentifier<'a> {
    fn of(id: &'a str) -> Self {
        if id.bytes().all(|b| b.is_ascii_digit()) {
            Self::Numeric(id.len(), id)
        } else {
            Self::Alphanumeric(id)
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

// ----- install kind -----

/// How the running copy was installed, which decides whether it can update itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    /// Unpacked from a release tarball into a writable prefix.
    Tarball { prefix: PathBuf },
    /// Run from an `AppImage`, the file at `path`, in a writable directory.
    AppImage { path: PathBuf },
    /// Anything else: a package, a development build, or a prefix or
    /// `AppImage` this user can't write to. Updates are announced, not installed.
    Other,
}

/// What the `AppImage` runtime tells the program it starts, from the environment.
#[derive(Debug, Default)]
pub(crate) struct LaunchEnv {
    /// `APPIMAGE`: the `AppImage` file.
    pub appimage: Option<PathBuf>,
    /// `APPDIR`: where the `AppImage` is mounted or extracted.
    pub appdir: Option<PathBuf>,
}

impl LaunchEnv {
    fn current() -> Self {
        Self {
            appimage: std::env::var_os("APPIMAGE").map(PathBuf::from),
            appdir: std::env::var_os("APPDIR").map(PathBuf::from),
        }
    }
}

impl InstallKind {
    /// Classifies the executable at `exe` (resolve it once, at startup).
    #[must_use]
    pub fn detect(exe: &Path) -> Self {
        Self::detect_in(exe, &LaunchEnv::current())
    }

    pub(crate) fn detect_in(exe: &Path, env: &LaunchEnv) -> Self {
        let exe = exe.canonicalize().unwrap_or_else(|_| exe.to_owned());
        // `APPIMAGE` and `APPDIR` are inherited by everything an AppImage
        // starts (a terminal, an editor), so they describe this program only
        // when its own executable is inside `APPDIR`. Without that check, an
        // update could overwrite another application's AppImage.
        let appdir = env.appdir.as_deref().and_then(|d| d.canonicalize().ok());
        if appdir.is_some_and(|dir| exe.starts_with(dir)) {
            return match env.appimage.as_deref().and_then(|f| f.canonicalize().ok()) {
                Some(file)
                    if is_appimage_file(&file) && file.parent().is_some_and(is_writable_dir) =>
                {
                    Self::AppImage { path: file }
                }
                _ => Self::Other,
            };
        }
        let Some(bin) = exe.parent() else {
            return Self::Other;
        };
        let Some(prefix) = bin.parent() else {
            return Self::Other;
        };
        // An AppDir extracted by hand (`--appimage-extract`) and run without
        // the runtime has the tarball's layout under `usr/`, next to
        // `AppRun`; updating it as a tarball would leave its bundled
        // libraries behind.
        if prefix.file_name().is_some_and(|n| n == "usr")
            && prefix
                .parent()
                .is_some_and(|dir| dir.join("AppRun").exists())
        {
            return Self::Other;
        }
        let packaged = exe.file_name().is_some_and(|n| n == BINARY)
            && bin.file_name().is_some_and(|n| n == "bin")
            && prefix
                .join("share/applications")
                .join(format!("{APP_ID}.desktop"))
                .is_file();
        if packaged && is_writable_dir(bin) && is_writable_dir(&prefix.join("share")) {
            Self::Tarball {
                prefix: prefix.to_owned(),
            }
        } else {
            Self::Other
        }
    }

    #[must_use]
    pub fn can_self_update(&self) -> bool {
        !matches!(self, Self::Other)
    }

    /// The release file this install updates from, if the release has one.
    #[must_use]
    pub fn asset<'a>(&self, release: &'a Release) -> Option<&'a Asset> {
        match self {
            Self::Tarball { .. } => release.tarball.as_ref(),
            Self::AppImage { .. } => release.appimage.as_ref(),
            Self::Other => None,
        }
    }

    /// What a restart must run instead of the executable, if anything. An
    /// `AppImage`'s executable is gone once it exits (the runtime unmounts or
    /// deletes it), so a restart runs the `AppImage` file.
    #[must_use]
    pub fn restart_path(&self) -> Option<&Path> {
        match self {
            Self::AppImage { path } => Some(path),
            Self::Tarball { .. } | Self::Other => None,
        }
    }
}

/// Whether `path` is an `AppImage`: an ELF file with the type 2 magic bytes
/// (`AI` and 2 at offset 8), as the `AppImage` specification defines.
fn is_appimage_file(path: &Path) -> bool {
    let mut head = [0u8; 11];
    fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && is_appimage(&head)
}

fn is_appimage(bytes: &[u8]) -> bool {
    bytes.len() >= 11 && bytes[..4] == *b"\x7fELF" && bytes[8..11] == *b"AI\x02"
}

fn is_writable_dir(dir: &Path) -> bool {
    let probe = dir.join(format!(".hardcover-desktop-probe-{}", std::process::id()));
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

// ----- releases -----

/// A published release newer than the running version.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: Version,
    /// Release notes (Markdown).
    pub notes: String,
    /// The release page on GitHub.
    pub page_url: String,
    /// The tarball for this architecture, if the release has one.
    pub tarball: Option<Asset>,
    /// The `AppImage` for this architecture, if the release has one.
    pub appimage: Option<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub signature_url: String,
}

/// The tarball name a release of `version` uses on this architecture.
#[must_use]
pub fn tarball_name(version: &Version) -> String {
    format!("{BINARY}-{version}-{}-linux.tar.gz", std::env::consts::ARCH)
}

/// The `AppImage` name a release of `version` uses on this architecture.
#[must_use]
pub fn appimage_name(version: &Version) -> String {
    format!(
        "{BINARY}-{version}-{}-linux.AppImage",
        std::env::consts::ARCH
    )
}

#[derive(Deserialize)]
struct WireRelease {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<WireAsset>,
}

#[derive(Deserialize)]
struct WireAsset {
    name: String,
    browser_download_url: String,
}

/// Checks for, downloads and installs updates. Blocking: call it off the UI thread.
#[derive(Clone)]
pub struct Updater {
    http: Arc<dyn HttpTransport>,
    api_url: String,
    public_key: String,
}

impl fmt::Debug for Updater {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Updater")
            .field("api_url", &self.api_url)
            .finish_non_exhaustive()
    }
}

impl Default for Updater {
    fn default() -> Self {
        Self::new()
    }
}

impl Updater {
    /// Checks [`REPOSITORY`] and verifies against the release signing key.
    #[must_use]
    pub fn new() -> Self {
        Self::with_transport(
            Arc::new(UreqTransport::new()),
            format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"),
            release_public_key().to_owned(),
        )
    }

    pub(crate) fn with_transport(
        http: Arc<dyn HttpTransport>,
        api_url: String,
        public_key: String,
    ) -> Self {
        Self {
            http,
            api_url,
            public_key,
        }
    }

    /// The latest release, if it is newer than `current`. `Ok(None)` also when
    /// there is no published release (or the repository isn't public).
    ///
    /// # Errors
    ///
    /// Returns an error if the release request fails or the response cannot be
    /// decoded.
    pub fn check(&self, current: &Version) -> Result<Option<Release>> {
        let resp = self.http.send(
            HttpRequest::new(Method::Get, &self.api_url)
                .header("accept", "application/vnd.github+json")
                .header("x-github-api-version", "2022-11-28")
                .timeout(Duration::from_secs(20))
                .max_response_bytes(4 * 1024 * 1024),
        )?;
        match resp.status {
            200 => {}
            404 => return Ok(None),
            403 | 429 => {
                return Err(Error::Network(
                    "GitHub's rate limit was reached; try again later".into(),
                ));
            }
            s => return Err(Error::Network(format!("GitHub answered {s}"))),
        }
        let wire: WireRelease = serde_json::from_slice(&resp.body)
            .map_err(|e| Error::Update(format!("unexpected answer from GitHub: {e}")))?;
        if wire.draft || wire.prerelease {
            return Ok(None);
        }
        let Some(version) = Version::parse(&wire.tag_name) else {
            return Ok(None);
        };
        if version <= *current {
            return Ok(None);
        }
        let url_of = |n: &str| {
            wire.assets
                .iter()
                .find(|a| a.name == n)
                .map(|a| a.browser_download_url.clone())
        };
        // A file counts only with its signature beside it.
        let signed = |name: String| match (url_of(&name), url_of(&format!("{name}.minisig"))) {
            (Some(url), Some(signature_url)) => Some(Asset {
                name,
                url,
                signature_url,
            }),
            _ => None,
        };
        Ok(Some(Release {
            tarball: signed(tarball_name(&version)),
            appimage: signed(appimage_name(&version)),
            version,
            notes: wire.body.unwrap_or_default(),
            page_url: wire.html_url,
        }))
    }

    /// Downloads, verifies and installs `release` over the install `target`.
    /// The running app keeps its old code until it restarts.
    ///
    /// # Errors
    ///
    /// Returns an error if the release is ineligible, verification fails or the
    /// installation cannot be replaced.
    pub fn install(
        &self,
        release: &Release,
        current: &Version,
        target: &InstallKind,
    ) -> Result<()> {
        if release.version <= *current {
            return Err(update_error("the release is not newer than this version"));
        }
        match target {
            InstallKind::Tarball { prefix } => {
                let expected_name = tarball_name(&release.version);
                let tarball = self.fetch_verified(
                    release.tarball.as_ref(),
                    &expected_name,
                    MAX_TARBALL_BYTES,
                )?;
                let top = expected_name.trim_end_matches(".tar.gz");
                let _lock = lock_dir(prefix)?;
                stage_tarball(&tarball, top, prefix)?.commit()
            }
            InstallKind::AppImage { path } => {
                let image = self.fetch_verified(
                    release.appimage.as_ref(),
                    &appimage_name(&release.version),
                    MAX_APPIMAGE_BYTES,
                )?;
                if !is_appimage(&image) {
                    return Err(update_error("the update is not an AppImage"));
                }
                let dir = path
                    .parent()
                    .ok_or_else(|| update_error("the AppImage has no directory"))?;
                let _lock = lock_dir(dir)?;
                let mut staged = Staged::default();
                staged.write(path, 0o755, &mut image.as_slice())?;
                staged.commit()
            }
            InstallKind::Other => Err(update_error("this copy can't update itself")),
        }
    }

    /// Downloads `asset` and its signature, and checks the signature was made
    /// with the release key for `expected_name`.
    fn fetch_verified(
        &self,
        asset: Option<&Asset>,
        expected_name: &str,
        limit: u64,
    ) -> Result<Vec<u8>> {
        let asset =
            asset.ok_or_else(|| update_error("the release has no build for this computer"))?;
        if asset.name != expected_name {
            return Err(update_error(
                "the release's file name doesn't match its version",
            ));
        }
        let signature = self.download(&asset.signature_url, 64 * 1024)?;
        let data = self.download(&asset.url, limit)?;
        verify(&self.public_key, &data, &signature, expected_name)?;
        Ok(data)
    }

    fn download(&self, url: &str, limit: u64) -> Result<Vec<u8>> {
        let resp = self.http.send(
            HttpRequest::new(Method::Get, url)
                .header("accept", "application/octet-stream")
                .timeout(Duration::from_secs(300))
                .max_response_bytes(limit),
        )?;
        if resp.status != 200 {
            return Err(Error::Network(format!(
                "downloading the update failed ({})",
                resp.status
            )));
        }
        Ok(resp.body)
    }
}

fn update_error(msg: &str) -> Error {
    Error::Update(msg.to_owned())
}

/// Checks the minisign signature over `data` and that it was made for `file_name`.
fn verify(public_key: &str, data: &[u8], signature: &[u8], file_name: &str) -> Result<()> {
    let bad = || update_error("the update's signature is not valid");
    let key = minisign_verify::PublicKey::from_base64(public_key)
        .map_err(|_| update_error("the release signing key is invalid"))?;
    let text = std::str::from_utf8(signature).map_err(|_| bad())?;
    let sig = minisign_verify::Signature::decode(text).map_err(|_| bad())?;
    key.verify(data, &sig, false).map_err(|_| bad())?;
    let signed_file = sig
        .trusted_comment()
        .split('\t')
        .find_map(|field| field.strip_prefix("file:"));
    if signed_file != Some(file_name) {
        return Err(update_error(
            "the update's signature was made for a different file",
        ));
    }
    Ok(())
}

/// Takes the exclusive `flock` on `dir` that `install.sh` takes on the prefix,
/// waiting up to [`LOCK_WAIT`] for another install to finish. The lock lasts
/// as long as the returned file is open.
fn lock_dir(dir: &Path) -> Result<fs::File> {
    let file = fs::File::open(dir).map_err(io_error)?;
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(fs::TryLockError::WouldBlock) => {
                return Err(update_error(
                    "another installation of Hardcover Desktop is still running",
                ));
            }
            Err(fs::TryLockError::Error(e)) => return Err(io_error(e)),
        }
    }
}

/// Writes the allowed files under `top/` beside their destinations in
/// `prefix`, ready to be renamed into place with the binary last.
fn stage_tarball(tarball: &[u8], top: &str, prefix: &Path) -> Result<Staged> {
    let corrupt = |what: &str| update_error(&format!("the update archive is not valid: {what}"));
    let binary = Path::new("bin").join(BINARY);

    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(tarball));
    let mut staged = Staged::default();
    let mut total: u64 = 0;
    for entry in archive.entries().map_err(|_| corrupt("unreadable"))? {
        let mut entry = entry.map_err(|_| corrupt("unreadable entry"))?;
        let path = entry.path().map_err(|_| corrupt("bad path"))?.into_owned();
        let mut parts = path.components();
        match parts.next() {
            Some(Component::Normal(first)) if first == top => {}
            _ => return Err(corrupt("unexpected top-level directory")),
        }
        let rel: PathBuf = parts.as_path().to_owned();
        if rel.as_os_str().is_empty() {
            continue;
        }
        if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(corrupt("path escapes the archive"));
        }
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        if !kind.is_file() {
            return Err(corrupt("links and special files are not allowed"));
        }
        if rel != binary && !rel.starts_with("share") {
            return Err(corrupt("unexpected file"));
        }
        total = total.saturating_add(entry.size());
        if total > MAX_UNPACKED_BYTES {
            return Err(corrupt("too large"));
        }
        let mode = if rel == binary { 0o755 } else { 0o644 };
        staged.write(&prefix.join(&rel), mode, &mut entry)?;
    }
    let binary = prefix.join(binary);
    if !staged.files.iter().any(|(_, dest)| *dest == binary) {
        return Err(corrupt("no executable"));
    }
    // A stable sort: the binary last, everything else in archive order.
    staged.files.sort_by_key(|(_, dest)| *dest == binary);
    Ok(staged)
}

/// Files written beside their destinations, waiting to be renamed over them.
/// Dropping it removes every file not renamed yet.
#[derive(Default)]
struct Staged {
    /// `(temporary file, destination)`, in the order they are renamed.
    files: Vec<(PathBuf, PathBuf)>,
    /// The destination directories, each swept once and synced at the end.
    dirs: BTreeSet<PathBuf>,
}

impl Staged {
    /// Copies `data` into a new file beside `dest`, with `mode`. The first
    /// file in a directory first removes what a crashed install left there.
    fn write(&mut self, dest: &Path, mode: u32, data: &mut dyn Read) -> Result<()> {
        let dir = dest
            .parent()
            .ok_or_else(|| update_error("installing the update failed: no directory"))?;
        fs::create_dir_all(dir).map_err(io_error)?;
        if self.dirs.insert(dir.to_owned()) {
            remove_leftovers(dir);
        }
        let (temp, mut out) = create_temp(dir)?;
        self.files.push((temp.clone(), dest.to_owned()));
        let mut buf = vec![0; 64 * 1024];
        loop {
            let n = data
                .read(&mut buf)
                .map_err(|_| update_error("the update archive is not valid: truncated"))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(io_error)?;
        }
        out.sync_all().map_err(io_error)?;
        set_mode(&temp, mode)
    }

    /// Renames every file over its destination, in order, then syncs their
    /// directories so the renames survive a crash.
    fn commit(mut self) -> Result<()> {
        while !self.files.is_empty() {
            let (temp, dest) = &self.files[0];
            fs::rename(temp, dest).map_err(io_error)?;
            self.files.remove(0);
        }
        for dir in &self.dirs {
            // Best effort: the files are in place by now, and a filesystem
            // that can't sync a directory has nothing more to offer.
            if let Ok(dir) = fs::File::open(dir) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        for (temp, _) in &self.files {
            let _ = fs::remove_file(temp);
        }
    }
}

/// A new file in `dir` named with [`TEMP_PREFIX`].
fn create_temp(dir: &Path) -> Result<(PathBuf, fs::File)> {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    loop {
        let serial = SERIAL.fetch_add(1, AtomicOrdering::Relaxed);
        let path = dir.join(format!("{TEMP_PREFIX}{}-{serial}", std::process::id()));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(io_error(e)),
        }
    }
}

/// Removes the temporary files a crashed install left in `dir`. Run under the
/// lock, where no other install can be writing one.
fn remove_leftovers(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let ours = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(TEMP_PREFIX));
        // Not through a link, and never a directory.
        if ours && entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(io_error)
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32) -> Result<()> {
    Ok(())
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err passes the owned error to this conversion"
)]
fn io_error(e: std::io::Error) -> Error {
    update_error(&format!("installing the update failed: {e}"))
}

#[cfg(test)]
mod tests;
