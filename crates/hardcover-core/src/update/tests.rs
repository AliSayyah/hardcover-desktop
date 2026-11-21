//! Update tests. Releases, tarballs and signatures are made up per test, signed
//! with a throwaway key.

use serde_json::{Value, json};

use super::*;
use crate::http::fake::FakeTransport;

const API: &str = "https://api.test/releases/latest";

fn v(s: &str) -> Version {
    Version::parse(s).unwrap()
}

struct Key {
    pair: minisign::KeyPair,
}

impl Key {
    fn new() -> Self {
        Self {
            pair: minisign::KeyPair::generate_unencrypted_keypair().unwrap(),
        }
    }
    fn public(&self) -> String {
        self.pair.pk.to_base64()
    }
    /// Signs like `minisign -S` does: prehashed, `file:<name>` in the trusted comment.
    fn sign(&self, data: &[u8], file_name: &str) -> Vec<u8> {
        let comment = format!("timestamp:1\tfile:{file_name}\thashed");
        minisign::sign(None, &self.pair.sk, data, Some(&comment), None)
            .unwrap()
            .into_string()
            .into_bytes()
    }
}

/// A gzipped tarball of `(path, contents)` regular files.
fn tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    for (path, data) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(tar::EntryType::Regular);
        header.set_path(path).unwrap();
        header.set_cksum();
        builder.append(&header, *data).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

/// A gzipped tarball with one raw entry whose name bypasses `set_path` checks.
fn raw_tarball(name: &str, kind: tar::EntryType, link: Option<&str>) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_size(if link.is_some() { 0 } else { 3 });
    header.set_mode(0o644);
    header.set_entry_type(kind);
    let gnu = header.as_gnu_mut().unwrap();
    gnu.name[..name.len()].copy_from_slice(name.as_bytes());
    if let Some(target) = link {
        gnu.linkname[..target.len()].copy_from_slice(target.as_bytes());
    }
    header.set_cksum();
    let data: &[u8] = if link.is_some() { b"" } else { b"bad" };
    builder.append(&header, data).unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}

fn top(version: &str) -> String {
    tarball_name(&v(version))
        .trim_end_matches(".tar.gz")
        .to_owned()
}

fn release_tarball(version: &str) -> Vec<u8> {
    let t = top(version);
    let bin = format!("{t}/bin/hardcover-desktop");
    let desktop = format!("{t}/share/applications/{APP_ID}.desktop");
    let body = format!("binary {version}");
    tarball(&[(&bin, body.as_bytes()), (&desktop, b"[Desktop Entry]\n")])
}

fn release(version: &str) -> Release {
    let name = tarball_name(&v(version));
    Release {
        version: v(version),
        notes: String::new(),
        page_url: "https://example.test/release".into(),
        tarball: Some(Asset {
            url: format!("https://dl.test/{name}"),
            signature_url: format!("https://dl.test/{name}.minisig"),
            name,
        }),
        appimage: None,
    }
}

fn tarball_target(prefix: &Path) -> InstallKind {
    InstallKind::Tarball {
        prefix: prefix.to_owned(),
    }
}

fn updater(key: &Key) -> (Updater, Arc<FakeTransport>) {
    let fake = Arc::new(FakeTransport::new());
    let updater = Updater::with_transport(fake.clone(), API.into(), key.public());
    (updater, fake)
}

/// A prefix laid out like an unpacked older release.
fn old_install() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    fs::create_dir_all(p.join("bin")).unwrap();
    fs::create_dir_all(p.join("share/applications")).unwrap();
    fs::write(p.join("bin/hardcover-desktop"), "binary 0.1.0").unwrap();
    fs::write(
        p.join(format!("share/applications/{APP_ID}.desktop")),
        "old",
    )
    .unwrap();
    dir
}

fn binary(prefix: &Path) -> String {
    fs::read_to_string(prefix.join("bin/hardcover-desktop")).unwrap()
}

/// Every install temporary file under `dir`, at any depth, through links.
fn temp_files(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            found.extend(temp_files(&path));
        } else if entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
            found.push(path.display().to_string());
        }
    }
    found
}

/// An updater that answers with `version`'s tarball, signed.
fn serving_tarball(version: &str) -> (Updater, Release) {
    let key = Key::new();
    let (u, fake) = updater(&key);
    let r = release(version);
    let data = release_tarball(version);
    fake.push_bytes(200, &key.sign(&data, &r.tarball.as_ref().unwrap().name));
    fake.push_bytes(200, &data);
    (u, r)
}

// ----- versions -----

#[test]
fn versions_parse_and_order() {
    assert_eq!(v("v1.2.3"), v("1.2.3"));
    assert_eq!(v("1.2.3+build.5"), v("1.2.3"));
    assert!(v("0.10.0") > v("0.9.9"));
    assert!(v("1.0.0") > v("1.0.0-rc.1"));
    assert!(v("1.0.0-rc.2") > v("1.0.0-rc.1"));
    assert_eq!(v("2.0.1-beta").to_string(), "2.0.1-beta");
    for bad in [
        "",
        "1.2",
        "1.2.3.4",
        "1.x.3",
        "1.2.3-",
        "-1.2.3",
        "1..3",
        "1.2.3-rc..1",
        "1.2.3-rc.01",
        "1.2.3-rc_1",
    ] {
        assert_eq!(Version::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn pre_releases_order_as_semantic_versioning_says() {
    // The example from semver.org, section 11, in order.
    let ordered = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0-rc.1",
        "1.0.0",
    ];
    for pair in ordered.windows(2) {
        assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
    }
    assert!(v("1.0.0-rc.10") > v("1.0.0-rc.9"));
    assert!(v("1.0.0-rc.1") > v("1.0.0-rc.0"));
    // Too long for any integer type, still ordered by value.
    assert!(v("1.0.0-99999999999999999999999") > v("1.0.0-9999999999999999999999"));
    assert_eq!(v("1.0.0-rc.1").cmp(&v("1.0.0-rc.1")), Ordering::Equal);
}

#[test]
fn the_bundled_public_key_parses() {
    assert!(minisign_verify::PublicKey::from_base64(release_public_key()).is_ok());
}

// ----- checking -----

fn github_release(tag: &str, assets: &[&str]) -> String {
    json!({
        "tag_name": tag,
        "body": "Notes",
        "html_url": format!("https://github.test/{tag}"),
        "draft": false,
        "prerelease": false,
        "assets": assets.iter().map(|n| json!({
            "name": n,
            "browser_download_url": format!("https://dl.test/{n}"),
        })).collect::<Vec<_>>(),
    })
    .to_string()
}

#[test]
fn check_finds_a_newer_release_with_its_assets() {
    let key = Key::new();
    let (u, fake) = updater(&key);
    let name = tarball_name(&v("0.2.0"));
    let sig = format!("{name}.minisig");
    let image = appimage_name(&v("0.2.0"));
    let image_sig = format!("{image}.minisig");
    let zsync = format!("{image}.zsync");
    fake.push(
        200,
        &github_release(
            "v0.2.0",
            &[&name, &sig, &image, &image_sig, &zsync, "other.zip"],
        ),
    );

    let r = u.check(&v("0.1.0")).unwrap().unwrap();
    assert_eq!(r.version, v("0.2.0"));
    assert_eq!(r.notes, "Notes");
    let tarball = r.tarball.as_ref().unwrap();
    assert_eq!(tarball.url, format!("https://dl.test/{name}"));
    assert_eq!(tarball.signature_url, format!("https://dl.test/{sig}"));
    let appimage = r.appimage.as_ref().unwrap();
    assert_eq!(appimage.url, format!("https://dl.test/{image}"));
    assert_eq!(
        appimage.signature_url,
        format!("https://dl.test/{image_sig}")
    );

    // Each install updates from its own file.
    let prefix = tarball_target(Path::new("/opt/x"));
    assert_eq!(prefix.asset(&r), Some(tarball));
    let path = PathBuf::from("/opt/x.AppImage");
    assert_eq!(InstallKind::AppImage { path }.asset(&r), Some(appimage));
    assert_eq!(InstallKind::Other.asset(&r), None);

    let req = &fake.requests()[0];
    assert_eq!(req.url, API);
    assert_eq!(req.header("accept"), Some("application/vnd.github+json"));
}

#[test]
fn check_ignores_same_older_draft_and_prerelease() {
    let key = Key::new();
    let (u, fake) = updater(&key);
    fake.push(200, &github_release("v0.1.0", &[]));
    fake.push(200, &github_release("v0.0.9", &[]));
    let mut draft: Value = serde_json::from_str(&github_release("v0.2.0", &[])).unwrap();
    draft["draft"] = json!(true);
    fake.push(200, &draft.to_string());
    let mut pre: Value = serde_json::from_str(&github_release("v0.2.0", &[])).unwrap();
    pre["prerelease"] = json!(true);
    fake.push(200, &pre.to_string());
    for _ in 0..4 {
        assert_eq!(u.check(&v("0.1.0")).unwrap(), None);
    }
}

#[test]
fn check_without_releases_or_build_for_this_arch() {
    let key = Key::new();
    let (u, fake) = updater(&key);
    fake.push(404, r#"{"message":"Not Found"}"#);
    assert_eq!(u.check(&v("0.1.0")).unwrap(), None);

    // Newer, but only the tarball: no signature means nothing to install.
    fake.push(
        200,
        &github_release("v0.2.0", &[&tarball_name(&v("0.2.0"))]),
    );
    let r = u.check(&v("0.1.0")).unwrap().unwrap();
    assert_eq!(r.tarball, None);

    fake.push(403, "{}");
    assert!(u.check(&v("0.1.0")).is_err());
}

// ----- installing -----

#[test]
fn install_replaces_the_files_and_cleans_up() {
    let key = Key::new();
    let (u, fake) = updater(&key);
    let r = release("0.2.0");
    let data = release_tarball("0.2.0");
    let name = &r.tarball.as_ref().unwrap().name;
    fake.push_bytes(200, &key.sign(&data, name));
    fake.push_bytes(200, &data);
    let prefix = old_install();

    u.install(&r, &v("0.1.0"), &tarball_target(prefix.path()))
        .unwrap();

    assert_eq!(binary(prefix.path()), "binary 0.2.0");
    let desktop = prefix
        .path()
        .join(format!("share/applications/{APP_ID}.desktop"));
    assert_eq!(fs::read_to_string(desktop).unwrap(), "[Desktop Entry]\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(prefix.path().join("bin/hardcover-desktop"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }
    assert_eq!(temp_files(prefix.path()), [] as [String; 0]);
}

#[test]
fn install_waits_for_the_installer_lock_on_the_prefix() {
    let (u, r) = serving_tarball("0.2.0");
    let prefix = old_install();
    // `install.sh` itself takes the lock, as it does while it installs, and
    // holds it until its input closes.
    let mut held = std::process::Command::new("bash")
        .args([
            "-c",
            r#"exec 9<"$1" && flock 9 && echo locked && read -r _"#,
            "_",
        ])
        .arg(prefix.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut said = String::new();
    std::io::BufRead::read_line(
        &mut std::io::BufReader::new(held.stdout.take().unwrap()),
        &mut said,
    )
    .unwrap();
    assert_eq!(said, "locked\n");

    let target = tarball_target(prefix.path());
    let install = std::thread::spawn(move || u.install(&r, &v("0.1.0"), &target));
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(binary(prefix.path()), "binary 0.1.0");
    assert_eq!(temp_files(prefix.path()), [] as [String; 0]);

    drop(held.stdin.take());
    held.wait().unwrap();
    install.join().unwrap().unwrap();
    assert_eq!(binary(prefix.path()), "binary 0.2.0");
}

#[test]
fn install_removes_what_a_crashed_install_left() {
    let (u, r) = serving_tarball("0.2.0");
    let prefix = old_install();
    let p = prefix.path();
    let stale = [
        p.join(format!("bin/{TEMP_PREFIX}4242-0")),
        p.join(format!("share/applications/{TEMP_PREFIX}abcdefgh")),
    ];
    for file in &stale {
        fs::write(file, "half a binary").unwrap();
    }
    // Not ours, or not a file: left alone.
    fs::write(p.join("bin/.hardcover-desktop-probe-1"), "").unwrap();
    fs::create_dir(p.join(format!("bin/{TEMP_PREFIX}dir"))).unwrap();

    u.install(&r, &v("0.1.0"), &tarball_target(p)).unwrap();

    assert_eq!(binary(p), "binary 0.2.0");
    for file in &stale {
        assert!(!file.exists(), "{} is still there", file.display());
    }
    assert!(p.join("bin/.hardcover-desktop-probe-1").exists());
    assert!(p.join(format!("bin/{TEMP_PREFIX}dir")).is_dir());
}

#[test]
fn install_works_when_bin_and_share_are_links_to_another_filesystem() {
    // /dev/shm and the temporary directory are separate mounts here; a rename
    // between them fails with EXDEV.
    let Ok(elsewhere) = tempfile::tempdir_in("/dev/shm") else {
        eprintln!("skipped: no /dev/shm");
        return;
    };
    let prefix = old_install();
    let p = prefix.path();
    {
        use std::os::unix::fs::MetadataExt as _;
        let dev = |path: &Path| fs::metadata(path).unwrap().dev();
        if dev(p) == dev(elsewhere.path()) {
            eprintln!("skipped: /dev/shm is on the same filesystem");
            return;
        }
    }
    for dir in ["bin", "share"] {
        fs::rename(p.join(dir), elsewhere.path().join(dir)).unwrap_or_else(|_| {
            // Moving across filesystems: copy the old layout instead.
            copy_tree(&p.join(dir), &elsewhere.path().join(dir));
            fs::remove_dir_all(p.join(dir)).unwrap();
        });
        std::os::unix::fs::symlink(elsewhere.path().join(dir), p.join(dir)).unwrap();
    }
    let (u, r) = serving_tarball("0.2.0");

    u.install(&r, &v("0.1.0"), &tarball_target(p)).unwrap();

    assert_eq!(binary(p), "binary 0.2.0");
    let desktop = p.join(format!("share/applications/{APP_ID}.desktop"));
    assert_eq!(fs::read_to_string(desktop).unwrap(), "[Desktop Entry]\n");
    assert!(p.join("bin").is_symlink());
    assert_eq!(temp_files(elsewhere.path()), [] as [String; 0]);
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

#[test]
fn a_failed_install_changes_nothing() {
    // The binary comes first and the desktop entry's bytes are cut short, so
    // the failure comes after a file was already written.
    let key = Key::new();
    let (u, fake) = updater(&key);
    let r = release("0.2.0");
    let t = top("0.2.0");
    let mut data = tarball(&[
        (&format!("{t}/bin/hardcover-desktop"), b"binary 0.2.0"),
        (
            &format!("{t}/share/applications/{APP_ID}.desktop"),
            &[b'x'; 4096],
        ),
    ]);
    // Undo the compression to cut the archive inside the second file.
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(data.as_slice())
        .read_to_end(&mut raw)
        .unwrap();
    raw.truncate(512 * 3 + 100);
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&raw).unwrap();
    data = gz.finish().unwrap();
    fake.push_bytes(200, &key.sign(&data, &r.tarball.as_ref().unwrap().name));
    fake.push_bytes(200, &data);
    let prefix = old_install();

    let err = u
        .install(&r, &v("0.1.0"), &tarball_target(prefix.path()))
        .unwrap_err();

    assert!(err.to_string().contains("not valid"), "{err}");
    assert_eq!(binary(prefix.path()), "binary 0.1.0");
    let desktop = prefix
        .path()
        .join(format!("share/applications/{APP_ID}.desktop"));
    assert_eq!(fs::read_to_string(desktop).unwrap(), "old");
    assert_eq!(temp_files(prefix.path()), [] as [String; 0]);
}

fn install_fails(sig: &[u8], data: &[u8], r: &Release, key: &Key) -> (String, String) {
    let (u, fake) = updater(key);
    fake.push_bytes(200, sig);
    fake.push_bytes(200, data);
    let prefix = old_install();
    let err = u
        .install(r, &v("0.1.0"), &tarball_target(prefix.path()))
        .unwrap_err();
    assert_eq!(temp_files(prefix.path()), [] as [String; 0]);
    (err.to_string(), binary(prefix.path()))
}

#[test]
fn install_rejects_a_signature_from_another_key() {
    let key = Key::new();
    let r = release("0.2.0");
    let data = release_tarball("0.2.0");
    let sig = Key::new().sign(&data, &r.tarball.as_ref().unwrap().name);
    let (err, bin) = install_fails(&sig, &data, &r, &key);
    assert!(err.contains("signature is not valid"), "{err}");
    assert_eq!(bin, "binary 0.1.0");
}

#[test]
fn install_rejects_tampered_bytes() {
    let key = Key::new();
    let r = release("0.2.0");
    let data = release_tarball("0.2.0");
    let sig = key.sign(&data, &r.tarball.as_ref().unwrap().name);
    let mut tampered = data.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    let (err, bin) = install_fails(&sig, &tampered, &r, &key);
    assert!(err.contains("signature is not valid"), "{err}");
    assert_eq!(bin, "binary 0.1.0");
}

#[test]
fn install_rejects_an_old_signed_release_served_as_new() {
    // A genuinely signed 0.1.5 tarball, advertised as 0.2.0.
    let key = Key::new();
    let old = release_tarball("0.1.5");
    let sig = key.sign(&old, &tarball_name(&v("0.1.5")));
    let (err, bin) = install_fails(&sig, &old, &release("0.2.0"), &key);
    assert!(err.contains("different file"), "{err}");
    assert_eq!(bin, "binary 0.1.0");
}

#[test]
fn install_rejects_a_mismatched_top_directory() {
    // Signed for the right name, but the archive holds another version's tree.
    let key = Key::new();
    let r = release("0.2.0");
    let data = release_tarball("0.1.5");
    let sig = key.sign(&data, &r.tarball.as_ref().unwrap().name);
    let (err, bin) = install_fails(&sig, &data, &r, &key);
    assert!(err.contains("top-level"), "{err}");
    assert_eq!(bin, "binary 0.1.0");
}

#[test]
fn install_rejects_unsafe_entries() {
    let t = top("0.2.0");
    let cases = [
        (
            raw_tarball(&format!("{t}/../evil"), tar::EntryType::Regular, None),
            "escapes",
        ),
        (
            raw_tarball(
                &format!("{t}/share/link"),
                tar::EntryType::Symlink,
                Some("/etc/passwd"),
            ),
            "links",
        ),
        (
            raw_tarball(&format!("{t}/etc/evil"), tar::EntryType::Regular, None),
            "unexpected file",
        ),
        (
            raw_tarball(&format!("{t}/share/readme"), tar::EntryType::Regular, None),
            "no executable",
        ),
    ];
    for (data, expected) in cases {
        let key = Key::new();
        let r = release("0.2.0");
        let sig = key.sign(&data, &r.tarball.as_ref().unwrap().name);
        let (err, bin) = install_fails(&sig, &data, &r, &key);
        assert!(err.contains(expected), "{expected}: {err}");
        assert_eq!(bin, "binary 0.1.0");
    }
}

#[test]
fn install_refuses_a_release_that_is_not_newer() {
    let key = Key::new();
    let (u, fake) = updater(&key);
    let prefix = old_install();
    let err = u
        .install(
            &release("0.1.0"),
            &v("0.1.0"),
            &tarball_target(prefix.path()),
        )
        .unwrap_err();
    assert!(err.to_string().contains("not newer"));
    assert_eq!(fake.requests().len(), 0);
}

// ----- install kind -----

#[test]
fn every_copy_of_the_public_key_is_the_release_key() {
    // The installer and the install instructions carry the key themselves.
    let copies = [
        ("install.sh", include_str!("../../../../install.sh")),
        ("README.md", include_str!("../../../../README.md")),
        (
            "packaging/linux/release-notes.md",
            include_str!("../../../../packaging/linux/release-notes.md"),
        ),
    ];
    let key = release_public_key();
    for (file, text) in copies {
        assert!(text.contains(key), "{file} lacks the release key");
        let others = text
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '+' || c == '/'))
            .filter(|w| w.len() == key.len() && w.starts_with("RW") && *w != key)
            .collect::<Vec<_>>();
        assert!(others.is_empty(), "{file} names another key: {others:?}");
    }
}

/// An `AppImage` file's first bytes: ELF, then the type 2 magic at offset 8.
const APPIMAGE_HEAD: &[u8] = b"\x7fELF\x02\x01\x01\x00AI\x02";

fn appimage_bytes(body: &str) -> Vec<u8> {
    [APPIMAGE_HEAD, body.as_bytes()].concat()
}

/// A running `AppImage`: the file in `apps/`, and its contents mounted at
/// `mount/` (a tarball's layout under `usr/`, next to `AppRun`).
struct RunningAppImage {
    dir: tempfile::TempDir,
}

impl RunningAppImage {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let usr = dir.path().join("mount/usr");
        fs::create_dir_all(usr.join("bin")).unwrap();
        fs::create_dir_all(usr.join("share/applications")).unwrap();
        fs::write(usr.join(format!("share/applications/{APP_ID}.desktop")), "").unwrap();
        fs::write(usr.join("bin").join(BINARY), "").unwrap();
        fs::write(dir.path().join("mount/AppRun"), "").unwrap();
        fs::create_dir_all(dir.path().join("apps")).unwrap();
        fs::write(self_path(&dir), appimage_bytes("0.1.0")).unwrap();
        Self { dir }
    }
    fn exe(&self) -> PathBuf {
        self.dir.path().join("mount/usr/bin").join(BINARY)
    }
    fn file(&self) -> PathBuf {
        self_path(&self.dir)
    }
    fn env(&self) -> LaunchEnv {
        LaunchEnv {
            appimage: Some(self.file()),
            appdir: Some(self.dir.path().join("mount")),
        }
    }
}

fn self_path(dir: &tempfile::TempDir) -> PathBuf {
    dir.path().join("apps/Hardcover.AppImage")
}

#[test]
fn detects_a_running_appimage_from_its_runtime_variables() {
    let app = RunningAppImage::new();
    let kind = InstallKind::detect_in(&app.exe(), &app.env());
    let path = app.file().canonicalize().unwrap();
    assert_eq!(kind, InstallKind::AppImage { path: path.clone() });
    assert_eq!(kind.restart_path(), Some(path.as_path()));
    assert!(kind.can_self_update());
}

#[test]
fn appimage_variables_inherited_from_another_app_are_ignored() {
    // Started from a terminal inside another AppImage: the variables name that
    // one, and this executable is not inside its APPDIR.
    let other = RunningAppImage::new();
    let prefix = old_install();
    let exe = prefix.path().join("bin").join(BINARY);
    assert_eq!(
        InstallKind::detect_in(&exe, &other.env()),
        tarball_target(&prefix.path().canonicalize().unwrap())
    );
    let dev = tempfile::tempdir().unwrap();
    fs::write(dev.path().join(BINARY), "").unwrap();
    assert_eq!(
        InstallKind::detect_in(&dev.path().join(BINARY), &other.env()),
        InstallKind::Other
    );
    assert_eq!(fs::read(other.file()).unwrap(), appimage_bytes("0.1.0"));
}

#[test]
fn an_appimage_that_cannot_be_replaced_only_announces() {
    let app = RunningAppImage::new();
    // Inside APPDIR it is never a tarball, even though the extracted tree
    // looks like one and is writable.
    let missing = LaunchEnv {
        appimage: None,
        ..app.env()
    };
    assert_eq!(
        InstallKind::detect_in(&app.exe(), &missing),
        InstallKind::Other
    );

    let not_an_appimage = app.dir.path().join("apps/script.sh");
    fs::write(&not_an_appimage, "#!/bin/sh\n").unwrap();
    let wrong = LaunchEnv {
        appimage: Some(not_an_appimage),
        ..app.env()
    };
    assert_eq!(
        InstallKind::detect_in(&app.exe(), &wrong),
        InstallKind::Other
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let apps = app.dir.path().join("apps");
        fs::set_permissions(&apps, fs::Permissions::from_mode(0o555)).unwrap();
        // Root writes anyway, so only check where the directory is read-only.
        if !is_writable_dir(&apps) {
            assert_eq!(
                InstallKind::detect_in(&app.exe(), &app.env()),
                InstallKind::Other
            );
        }
        fs::set_permissions(&apps, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn an_appdir_extracted_by_hand_is_not_a_tarball() {
    // `--appimage-extract`, then run without the runtime: no variables.
    let app = RunningAppImage::new();
    assert_eq!(
        InstallKind::detect_in(&app.exe(), &LaunchEnv::default()),
        InstallKind::Other
    );
}

fn appimage_release(version: &str) -> Release {
    let name = appimage_name(&v(version));
    Release {
        tarball: None,
        appimage: Some(Asset {
            url: format!("https://dl.test/{name}"),
            signature_url: format!("https://dl.test/{name}.minisig"),
            name,
        }),
        ..release(version)
    }
}

fn leftovers(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(TEMP_PREFIX))
        .collect()
}

#[test]
fn install_replaces_the_appimage_in_place() {
    let key = Key::new();
    let (u, fake) = updater(&key);
    let r = appimage_release("0.2.0");
    let data = appimage_bytes("0.2.0");
    fake.push_bytes(200, &key.sign(&data, &appimage_name(&v("0.2.0"))));
    fake.push_bytes(200, &data);
    let app = RunningAppImage::new();
    let target = InstallKind::detect_in(&app.exe(), &app.env());

    u.install(&r, &v("0.1.0"), &target).unwrap();

    assert_eq!(fs::read(app.file()).unwrap(), data);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(app.file()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }
    assert_eq!(leftovers(&app.dir.path().join("apps")), [] as [String; 0]);
}

#[test]
fn install_rejects_a_bad_appimage_and_keeps_the_old_one() {
    let key = Key::new();
    let r = appimage_release("0.2.0");
    let name = appimage_name(&v("0.2.0"));
    let good = appimage_bytes("0.2.0");
    let cases: Vec<(Vec<u8>, Vec<u8>, &str)> = vec![
        (
            Key::new().sign(&good, &name),
            good.clone(),
            "signature is not valid",
        ),
        (
            key.sign(&good, &name),
            appimage_bytes("0.2.1"),
            "signature is not valid",
        ),
        (
            key.sign(&appimage_bytes("0.1.5"), &appimage_name(&v("0.1.5"))),
            appimage_bytes("0.1.5"),
            "different file",
        ),
        (
            key.sign(b"#!/bin/sh\n", &name),
            b"#!/bin/sh\n".to_vec(),
            "not an AppImage",
        ),
    ];
    for (sig, data, expected) in cases {
        let (u, fake) = updater(&key);
        fake.push_bytes(200, &sig);
        fake.push_bytes(200, &data);
        let app = RunningAppImage::new();
        let target = InstallKind::detect_in(&app.exe(), &app.env());
        let err = u.install(&r, &v("0.1.0"), &target).unwrap_err().to_string();
        assert!(err.contains(expected), "{err}");
        assert_eq!(fs::read(app.file()).unwrap(), appimage_bytes("0.1.0"));
        assert_eq!(leftovers(&app.dir.path().join("apps")), [] as [String; 0]);
    }

    // A release without an AppImage has nothing for this install.
    let app = RunningAppImage::new();
    let target = InstallKind::detect_in(&app.exe(), &app.env());
    let (u, _) = updater(&key);
    let err = u
        .install(&release("0.2.0"), &v("0.1.0"), &target)
        .unwrap_err();
    assert!(err.to_string().contains("no build"), "{err}");
}

#[test]
fn detects_an_unpacked_tarball_in_a_writable_prefix() {
    let prefix = old_install();
    let exe = prefix.path().join("bin/hardcover-desktop");
    assert_eq!(
        InstallKind::detect(&exe),
        InstallKind::Tarball {
            prefix: prefix.path().to_owned()
        }
    );

    fs::remove_file(
        prefix
            .path()
            .join(format!("share/applications/{APP_ID}.desktop")),
    )
    .unwrap();
    assert_eq!(InstallKind::detect(&exe), InstallKind::Other);

    let dev = tempfile::tempdir().unwrap();
    let mut f = fs::File::create(dev.path().join("hardcover-desktop")).unwrap();
    f.write_all(b"x").unwrap();
    assert_eq!(
        InstallKind::detect(&dev.path().join("hardcover-desktop")),
        InstallKind::Other
    );
}
