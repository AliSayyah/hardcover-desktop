//! Application services shared by every feature: the API client, the session,
//! the library cache and the cover cache, plus the lazy cover assets.
//!
//! Every core call blocks, so features run them with `cx.background_spawn` and
//! apply results on the foreground. The services are GPUI globals:
//! [`CoverService`] lives for the whole run, [`Services`] only while signed in.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use async_lock::Semaphore;
use fast_image_resize::{self as fir, PixelType};
use gpui_kit::{App, Asset, Global, RenderImage, Window};
use hardcover_core::auth::{
    DEFAULT_SCOPES, GrantedScopes, KeyringTokenStore, OAuthConfig, Session, StaticToken,
};
use hardcover_core::covers::CoverCache;
use hardcover_core::store::LibraryCache;
use hardcover_core::{Client, Cover, Error};

use crate::format;

/// Services available while signed in.
#[derive(Clone)]
pub struct Services {
    client: Client,
    /// `None` when signed in with the development token.
    session: Option<Arc<Session>>,
    /// `None` if the cache could not be opened; the app then works online only.
    cache: Option<Arc<LibraryCache>>,
    /// A snapshot of the session's grant: asking the session itself can block
    /// behind a token refresh.
    scopes: GrantedScopes,
    /// Scopes Hardcover refused with `insufficient_scope` although the grant
    /// lists them. The server is the authority, so these count as missing.
    refused: Vec<&'static str>,
}

impl Global for Services {}

impl Services {
    pub fn new(
        client: Client,
        session: Option<Arc<Session>>,
        cache: Option<Arc<LibraryCache>>,
        scopes: GrantedScopes,
    ) -> Self {
        Self {
            client,
            session,
            cache,
            scopes,
            refused: Vec::new(),
        }
    }

    /// True if Hardcover granted `scope` (one of `hardcover_core::auth::scopes`).
    /// Cheap: features call it while rendering, to hide or disable what the
    /// session can't do.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.contains(scope) && !self.refused.contains(&scope)
    }

    /// The scopes the app requests that this session lacks. Only a session can
    /// sign in again to get them; a personal token never lacks any it knows of.
    pub fn missing_scopes(&self) -> Vec<&'static str> {
        DEFAULT_SCOPES
            .into_iter()
            .filter(|s| !self.has_scope(s))
            .collect()
    }

    /// Records that Hardcover refused a request for lack of `scope`, so the
    /// feature turns into its "sign in again" state.
    pub fn scope_refused(scope: &'static str, cx: &mut App) {
        if let Some(services) = cx.try_global::<Self>()
            && !services.refused.contains(&scope)
        {
            cx.global_mut::<Self>().refused.push(scope);
            cx.refresh_windows();
        }
    }

    /// The message to show for a failed request. A scope Hardcover refused is
    /// recorded first (the one the error names, else `fallback`), so features
    /// that need it turn off and the permissions notice offers to sign in
    /// again. Where no notice can (a personal token, or a scope the app
    /// doesn't request), the message says to sign out and in again.
    pub fn error_message(error: &Error, fallback: Option<&'static str>, cx: &mut App) -> String {
        if let Error::InsufficientScope { scope } = error
            // The fallback stands in only for an unnamed scope: a named one
            // the app never requests can't be granted by signing in again.
            && let Some(scope) = match scope {
                Some(_) => format::requested_scope(scope.as_deref()),
                None => fallback,
            }
        {
            Self::scope_refused(scope, cx);
            if cx.try_global::<Self>().is_some_and(|s| s.session.is_some()) {
                return format::scope_refused_message(scope);
            }
        }
        format::error_message(error)
    }

    /// After a new sign-in replaced the session's tokens.
    pub fn set_scopes(scopes: GrantedScopes, cx: &mut App) {
        if cx.has_global::<Self>() {
            let services = cx.global_mut::<Self>();
            services.scopes = scopes;
            services.refused.clear();
            cx.refresh_windows();
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn session(&self) -> Option<&Arc<Session>> {
        self.session.as_ref()
    }

    pub fn cache(&self) -> Option<&Arc<LibraryCache>> {
        self.cache.as_ref()
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }
}

/// How the app starts, decided off the UI thread.
pub enum Startup {
    SignedIn {
        client: Client,
        session: Option<Arc<Session>>,
        cache: Option<Arc<LibraryCache>>,
        scopes: GrantedScopes,
    },
    SignedOut {
        cache: Option<Arc<LibraryCache>>,
    },
}

/// Blocking: reads the environment, the keyring and opens the caches.
pub fn startup() -> Startup {
    // Decided before anything real is opened, so a demo build never shows or
    // writes the account's library.
    #[cfg(feature = "demo")]
    return demo_startup();
    #[cfg(not(feature = "demo"))]
    real_startup()
}

/// The demo library, with a cache that lives only in memory.
#[cfg(feature = "demo")]
fn demo_startup() -> Startup {
    eprintln!("hardcover: demo build, showing the demo library");
    Startup::SignedIn {
        client: hardcover_core::demo::client(),
        session: None,
        cache: LibraryCache::open_in_memory().ok().map(Arc::new),
        scopes: GrantedScopes::all(),
    }
}

// Unused in demo builds, which never read the real account.
#[cfg_attr(feature = "demo", allow(dead_code))]
fn real_startup() -> Startup {
    let cache = match LibraryCache::open_default() {
        Ok(cache) => Some(Arc::new(cache)),
        Err(e) => {
            eprintln!("hardcover: library cache unavailable: {e}");
            None
        }
    };
    if let Some(token) = StaticToken::from_env() {
        eprintln!("hardcover: using HARDCOVER_TOKEN (development only)");
        // A personal token carries no scope information and acts for its
        // owner with full access.
        return Startup::SignedIn {
            client: Client::new(Arc::new(token)),
            session: None,
            cache,
            scopes: GrantedScopes::all(),
        };
    }
    let restored = KeyringTokenStore::new()
        .and_then(|store| Session::restore(OAuthConfig::default(), Arc::new(store)));
    match restored {
        Ok(Some(session)) => {
            let session = Arc::new(session);
            Startup::SignedIn {
                client: Client::new(session.clone()),
                scopes: session.granted_scopes(),
                session: Some(session),
                cache,
            }
        }
        Ok(None) => Startup::SignedOut { cache },
        Err(e) => {
            eprintln!("hardcover: could not restore the saved session: {e}");
            Startup::SignedOut { cache }
        }
    }
}

// ----- covers -----

/// Downloads at most this many covers at once.
const COVER_DOWNLOADS: usize = 4;
/// Height of decoded row thumbnails, in image pixels (2x the row's thumbnail).
const THUMBNAIL_HEIGHT: u32 = 144;
/// Height of the decoded detail cover, in image pixels. The cover is 12rem
/// tall, so this is sharp up to 2.5x scale with the default rem.
const COVER_HEIGHT: u32 = 480;

/// The cover cache, the download limiter and the decoder thread.
pub struct CoverService {
    cache: Option<Arc<CoverCache>>,
    downloads: Arc<Semaphore>,
    decoder: CoverDecoder,
    /// URLs whose download failed, so they can be retried after a later sync.
    failed: Arc<Mutex<HashSet<String>>>,
}

impl Global for CoverService {}

impl CoverService {
    /// Blocking: creates the cache directory and clears out stale scaled copies.
    pub fn open() -> Self {
        #[cfg(feature = "demo")]
        let cache = hardcover_core::demo::cover_cache(
            std::env::temp_dir().join("hardcover-desktop-demo-covers"),
        );
        #[cfg(not(feature = "demo"))]
        let cache = CoverCache::open_default();
        let cache = match cache {
            Ok(cache) => Some(Arc::new(cache)),
            Err(e) => {
                eprintln!("hardcover: cover cache unavailable: {e}");
                None
            }
        };
        if let Some(cache) = &cache {
            remove_stale_scaled(cache.dir());
        }
        Self {
            cache,
            downloads: Arc::new(Semaphore::new(COVER_DOWNLOADS)),
            decoder: CoverDecoder::spawn(),
            failed: Arc::default(),
        }
    }

    /// Forgets failed loads so the next render tries them again.
    pub fn retry_failed(cx: &mut App) {
        let Some(service) = cx.try_global::<Self>() else {
            return;
        };
        let failed: Vec<String> =
            std::mem::take(&mut *service.failed.lock().unwrap_or_else(|p| p.into_inner()))
                .into_iter()
                .collect();
        for url in failed {
            cx.remove_asset::<CoverFile>(&url);
            cx.remove_asset::<CoverThumbnail>(&url);
            cx.remove_asset::<CoverImage>(&url);
        }
    }

    fn loader(cx: &App) -> Option<CoverLoader> {
        let service = cx.try_global::<Self>()?;
        Some(CoverLoader {
            cache: service.cache.clone()?,
            downloads: service.downloads.clone(),
            decoder: service.decoder.clone(),
            failed: service.failed.clone(),
        })
    }
}

struct CoverLoader {
    cache: Arc<CoverCache>,
    downloads: Arc<Semaphore>,
    decoder: CoverDecoder,
    failed: Arc<Mutex<HashSet<String>>>,
}

/// One request to [`CoverDecoder`]: the file, the target height and where to
/// send the result.
struct DecodeJob {
    path: Arc<Path>,
    max_height: u32,
    reply: async_channel::Sender<Option<Arc<RenderImage>>>,
}

/// Decodes covers on one dedicated thread.
///
/// Decoding a large cover allocates tens of megabytes that are freed right
/// after scaling. glibc keeps freed memory in the arena of the thread that
/// allocated it, so decoding on GPUI's many background threads left one
/// bloated arena per thread: hundreds of megabytes of resident memory after
/// browsing a few hundred covers. One thread means one arena, which is reused
/// for every decode. A cover takes a few milliseconds, so one thread keeps up.
#[derive(Clone)]
struct CoverDecoder {
    jobs: async_channel::Sender<DecodeJob>,
}

impl CoverDecoder {
    fn spawn() -> Self {
        let (jobs, queue) = async_channel::unbounded::<DecodeJob>();
        let spawned = thread::Builder::new()
            .name("cover-decoder".into())
            .spawn(move || {
                while let Ok(job) = queue.recv_blocking() {
                    // The requester may have gone away; nothing to do then.
                    let _ = job
                        .reply
                        .send_blocking(load_or_decode(&job.path, job.max_height));
                }
            });
        if let Err(e) = spawned {
            // Without the thread every decode below resolves to `None`, which
            // the views show as the placeholder cover.
            eprintln!("hardcover: cover decoder unavailable: {e}");
        }
        Self { jobs }
    }

    async fn decode(&self, path: Arc<Path>, max_height: u32) -> Option<Arc<RenderImage>> {
        let (reply, result) = async_channel::bounded(1);
        self.jobs
            .send(DecodeJob {
                path,
                max_height,
                reply,
            })
            .await
            .ok()?;
        result.recv().await.ok().flatten()
    }
}

impl CoverLoader {
    /// The cover file on disk, downloading it (behind the limiter) if needed.
    async fn file(&self, url: &str) -> Option<Arc<Path>> {
        let cover = Cover::new(url);
        if let Some(path) = self.cache.cached_path(&cover) {
            return Some(path.into());
        }
        let _permit = self.downloads.acquire().await;
        match self.cache.fetch(&cover) {
            Ok(path) => Some(path.into()),
            Err(_) => {
                self.failed
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(url.to_owned());
                None
            }
        }
    }
}

/// A cover image on disk, keyed by its URL. `None` if it could not be fetched.
pub enum CoverFile {}

impl Asset for CoverFile {
    type Source = String;
    type Output = Option<Arc<Path>>;

    fn load(url: String, cx: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        let loader = CoverService::loader(cx);
        async move { loader?.file(&url).await }
    }
}

/// A small decoded cover for list rows. Row thumbnails stay in memory for the
/// whole run, so they are scaled down instead of keeping full-size covers.
pub enum CoverThumbnail {}

impl Asset for CoverThumbnail {
    type Source = String;
    type Output = Option<Arc<RenderImage>>;

    fn load(url: String, cx: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        load_scaled(CoverService::loader(cx), url, THUMBNAIL_HEIGHT)
    }
}

/// The detail pane's cover, scaled to [`COVER_HEIGHT`]. Unlike thumbnails it
/// is released when the pane moves on (see [`release_cover_image`]).
pub enum CoverImage {}

impl Asset for CoverImage {
    type Source = String;
    type Output = Option<Arc<RenderImage>>;

    fn load(url: String, cx: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        load_scaled(CoverService::loader(cx), url, COVER_HEIGHT)
    }
}

async fn load_scaled(
    loader: Option<CoverLoader>,
    url: String,
    max_height: u32,
) -> Option<Arc<RenderImage>> {
    let loader = loader?;
    let path = loader.file(&url).await?;
    let image = loader.decoder.decode(path, max_height).await;
    if image.is_none() {
        loader
            .failed
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(url);
    }
    image
}

/// Decodes an image file, scaled down to at most `max_height` pixels tall.
pub(crate) fn decode_scaled(path: &Path, max_height: u32) -> Option<Arc<RenderImage>> {
    let image = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let (src_width, src_height) = (image.width(), image.height());
    let (width, height) = scaled_size(src_width, src_height, max_height);
    // Covers are mostly JPEG, which decodes to RGB; resize that without
    // adding an alpha channel first.
    let (pixel_type, pixels) = match image {
        image::DynamicImage::ImageRgb8(rgb) => (PixelType::U8x3, rgb.into_raw()),
        other => (PixelType::U8x4, other.into_rgba8().into_raw()),
    };
    let pixels = if (width, height) == (src_width, src_height) {
        pixels
    } else {
        let src =
            fir::images::Image::from_vec_u8(src_width, src_height, pixels, pixel_type).ok()?;
        let mut dst = fir::images::Image::new(width, height, pixel_type);
        // Bilinear convolution is the Triangle filter `image` uses, with SIMD.
        let options = fir::ResizeOptions::new()
            .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Bilinear))
            .use_alpha(false);
        fir::Resizer::new().resize(&src, &mut dst, &options).ok()?;
        dst.into_vec()
    };
    // GPUI's sprite atlas expects BGRA.
    let bgra = match pixel_type {
        PixelType::U8x3 => pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0], u8::MAX])
            .collect(),
        _ => {
            let mut pixels = pixels;
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            pixels
        }
    };
    let frame = image::RgbaImage::from_raw(width, height, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(frame)])))
}

/// The size a `src_width` x `src_height` cover is shown at: at most
/// `max_height` tall, never enlarged.
///
/// This is the size `DynamicImage::resize` picked when it did the scaling: the
/// width is rounded down, then the result is fitted inside that box, so the
/// height can come out a pixel short. Kept so covers look exactly as before.
fn scaled_size(src_width: u32, src_height: u32, max_height: u32) -> (u32, u32) {
    let src_height = src_height.max(1);
    let height = max_height.min(src_height);
    let width = (u64::from(src_width) * u64::from(height) / u64::from(src_height)).max(1) as u32;
    let ratio = f64::min(
        f64::from(width) / f64::from(src_width.max(1)),
        f64::from(height) / f64::from(src_height),
    );
    let fit = |side: u32| ((f64::from(side) * ratio).round() as u32).max(1);
    (fit(src_width), fit(src_height))
}

// ----- scaled copies -----

/// Scaled copies of covers live in `scaled/<version>/` under the cover cache.
const SCALED_DIR: &str = "scaled";
/// Bump whenever [`decode_scaled`] changes its output: copies made by an
/// older build are then ignored, and deleted at the next start.
const SCALED_VERSION: &str = "v1";
/// A temporary file older than this was left by a write that never finished.
const STALE_TEMP_AGE: std::time::Duration = std::time::Duration::from_secs(60 * 60);
/// JPEG quality of saved detail-size copies.
const SCALED_JPEG_QUALITY: u8 = 92;

/// The cover scaled to `max_height`, from its saved copy when there is one.
/// Otherwise the cover is decoded and a copy is saved, so the next run skips
/// the decode: a relaunch then shows covers without decoding any of them.
fn load_or_decode(path: &Path, max_height: u32) -> Option<Arc<RenderImage>> {
    if let Some(image) = read_scaled(path, max_height) {
        return Some(image);
    }
    let image = decode_scaled(path, max_height)?;
    if let Err(e) = write_scaled(path, max_height, &image) {
        // The cover still shows; the next run decodes it again. Said once,
        // since the cause (say, a read-only cache) usually affects them all.
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            eprintln!("hardcover: could not save a scaled cover: {e}");
        }
    }
    Some(image)
}

/// `covers/scaled/v<N>/<cover file>-<height>.<ext>`.
fn scaled_path(cover: &Path, max_height: u32, ext: &str) -> Option<PathBuf> {
    let name = cover.file_name()?.to_str()?;
    Some(
        cover
            .parent()?
            .join(SCALED_DIR)
            .join(SCALED_VERSION)
            .join(format!("{name}-{max_height}.{ext}")),
    )
}

fn read_scaled(cover: &Path, max_height: u32) -> Option<Arc<RenderImage>> {
    [
        ("qoi", image::ImageFormat::Qoi),
        ("jpg", image::ImageFormat::Jpeg),
    ]
    .into_iter()
    .find_map(|(ext, format)| {
        let bytes = std::fs::read(scaled_path(cover, max_height, ext)?).ok()?;
        // The JPEG decoder fills in a truncated file (say, one cut short by a
        // crash) instead of failing, so check that it is complete. A short
        // QOI file fails to decode on its own.
        if format == image::ImageFormat::Jpeg && !bytes.ends_with(&[0xFF, 0xD9]) {
            return None;
        }
        let mut pixels = image::load_from_memory_with_format(&bytes, format)
            .ok()?
            .into_rgba8();
        if format == image::ImageFormat::Jpeg {
            // Saved as RGB; GPUI wants BGRA.
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
        }
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
    })
}

/// Saves a scaled cover next to the originals. Thumbnails are small, so they
/// are kept lossless: QOI of the BGRA pixels as they are. Larger copies are
/// high-quality JPEG when opaque, about a fifth of the size; a cover with
/// transparency stays QOI.
fn write_scaled(cover: &Path, max_height: u32, image: &RenderImage) -> std::io::Result<()> {
    use image::ImageEncoder as _;

    let size = image.size(0);
    let (width, height) = (size.width.0 as u32, size.height.0 as u32);
    let bgra = image
        .as_bytes(0)
        .ok_or_else(|| std::io::Error::other("the image has no frame"))?;
    let opaque = bgra.as_chunks::<4>().0.iter().all(|p| p[3] == u8::MAX);
    let mut encoded = Vec::new();
    let ext = if max_height > THUMBNAIL_HEIGHT && opaque {
        let rgb: Vec<u8> = bgra
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0]])
            .collect();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, SCALED_JPEG_QUALITY)
            .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
            .map_err(std::io::Error::other)?;
        "jpg"
    } else {
        image::codecs::qoi::QoiEncoder::new(&mut encoded)
            .write_image(bgra, width, height, image::ExtendedColorType::Rgba8)
            .map_err(std::io::Error::other)?;
        "qoi"
    };
    let path = scaled_path(cover, max_height, ext)
        .ok_or_else(|| std::io::Error::other("the cover path has no file name"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written aside and renamed, so a reader never sees half a file.
    let mut tmp = path.clone().into_os_string();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let written = std::fs::write(&tmp, encoded).and_then(|()| std::fs::rename(&tmp, &path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Deletes scaled copies made by older builds, and temporary files left by
/// writes that never finished. Blocking; errors are ignored, as the copies
/// are only a cache.
fn remove_stale_scaled(cover_dir: &Path) {
    let scaled = cover_dir.join(SCALED_DIR);
    let Ok(versions) = std::fs::read_dir(&scaled) else {
        return;
    };
    for entry in versions.flatten() {
        if entry.file_name() != SCALED_VERSION {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    let Ok(copies) = std::fs::read_dir(scaled.join(SCALED_VERSION)) else {
        return;
    };
    for entry in copies.flatten() {
        let path = entry.path();
        // Recent ones may belong to another running instance.
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STALE_TEMP_AGE);
        if path.extension().is_some_and(|ext| ext == "tmp") && stale {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The cached file for `url`, undecoded, starting a background load on first
/// use: for an image the cover pipeline can't decode (an SVG avatar), which
/// GPUI's `img` draws itself. The calling view re-renders when it finishes.
pub fn cover_file(url: &str, window: &mut Window, cx: &mut App) -> Option<Arc<Path>> {
    window.use_asset::<CoverFile>(&url.to_owned(), cx).flatten()
}

/// The row thumbnail for `url`, starting a background load on first use. The
/// calling view re-renders when it finishes.
pub fn cover_thumbnail(url: &str, window: &mut Window, cx: &mut App) -> Option<Arc<RenderImage>> {
    window
        .use_asset::<CoverThumbnail>(&url.to_owned(), cx)
        .flatten()
}

/// Forgets a detail cover the pane no longer shows. GPUI keeps loaded assets
/// and their textures for the whole run unless told otherwise, which adds up
/// when browsing hundreds of books.
pub fn release_cover_image(
    url: &str,
    image: Option<Arc<RenderImage>>,
    window: &mut Window,
    cx: &mut App,
) {
    cx.remove_asset::<CoverImage>(&url.to_owned());
    if let Some(image) = image {
        cx.drop_image(image, Some(window));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cover-like test image: a gradient, with an optional transparent corner.
    fn sample(path: &Path, width: u32, height: u32, transparent: bool) {
        let image = image::RgbaImage::from_fn(width, height, |x, y| {
            let alpha = if transparent && x < 10 && y < 10 {
                0
            } else {
                255
            };
            image::Rgba([(x * 255 / width) as u8, (y * 255 / height) as u8, 90, alpha])
        });
        image.save(path).unwrap();
    }

    fn pixels(image: &RenderImage) -> (u32, u32, Vec<u8>) {
        let size = image.size(0);
        (
            size.width.0 as u32,
            size.height.0 as u32,
            image.as_bytes(0).unwrap().to_vec(),
        )
    }

    fn max_channel_diff(a: &[u8], b: &[u8]) -> u8 {
        a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap()
    }

    #[test]
    fn scaled_size_keeps_the_old_rounding_and_never_enlarges() {
        assert_eq!(scaled_size(300, 450, 144), (96, 144));
        // `DynamicImage::resize` fitted 99x143.7 inside 99x144.
        assert_eq!(scaled_size(1000, 1449, 144), (99, 143));
        assert_eq!(scaled_size(300, 450, 480), (300, 450));
        assert_eq!(scaled_size(1, 5000, 144), (1, 144));
    }

    #[test]
    fn thumbnails_are_saved_losslessly_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.png");
        sample(&cover, 300, 450, false);

        let decoded = load_or_decode(&cover, THUMBNAIL_HEIGHT).unwrap();
        let saved = scaled_path(&cover, THUMBNAIL_HEIGHT, "qoi").unwrap();
        assert!(saved.is_file());
        assert_eq!(
            saved.parent().unwrap(),
            dir.path().join(SCALED_DIR).join(SCALED_VERSION)
        );

        let reused = read_scaled(&cover, THUMBNAIL_HEIGHT).unwrap();
        assert_eq!(pixels(&reused), pixels(&decoded));
        // No temporary files left behind.
        assert_eq!(
            std::fs::read_dir(saved.parent().unwrap()).unwrap().count(),
            1
        );
    }

    #[test]
    fn opaque_detail_covers_are_saved_as_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.png");
        sample(&cover, 600, 900, false);

        let decoded = load_or_decode(&cover, COVER_HEIGHT).unwrap();
        assert!(scaled_path(&cover, COVER_HEIGHT, "jpg").unwrap().is_file());
        let (w, h, original) = pixels(&decoded);
        let (rw, rh, reused) = pixels(&read_scaled(&cover, COVER_HEIGHT).unwrap());
        assert_eq!((rw, rh), (w, h));
        // Same channel order as the decode, within JPEG's loss.
        assert!(max_channel_diff(&original, &reused) < 24);
    }

    #[test]
    fn transparent_detail_covers_stay_lossless() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.png");
        sample(&cover, 600, 900, true);

        let decoded = load_or_decode(&cover, COVER_HEIGHT).unwrap();
        assert!(scaled_path(&cover, COVER_HEIGHT, "qoi").unwrap().is_file());
        let reused = read_scaled(&cover, COVER_HEIGHT).unwrap();
        assert_eq!(pixels(&reused), pixels(&decoded));
    }

    #[test]
    fn a_truncated_jpeg_copy_is_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.png");
        sample(&cover, 600, 900, false);
        load_or_decode(&cover, COVER_HEIGHT).unwrap();
        let saved = scaled_path(&cover, COVER_HEIGHT, "jpg").unwrap();
        let bytes = std::fs::read(&saved).unwrap();
        std::fs::write(&saved, &bytes[..bytes.len() / 2]).unwrap();

        assert!(read_scaled(&cover, COVER_HEIGHT).is_none());
    }

    #[test]
    fn stale_versions_and_temp_files_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let scaled = dir.path().join(SCALED_DIR);
        let old = scaled.join("v0");
        let current = scaled.join(SCALED_VERSION);
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(old.join("a-144.qoi"), b"x").unwrap();
        let copy = current.join("a-144.qoi");
        let fresh_tmp = current.join("b-144.qoi.1.tmp");
        let stale_tmp = current.join("c-144.qoi.2.tmp");
        for path in [&copy, &fresh_tmp, &stale_tmp] {
            std::fs::write(path, b"x").unwrap();
        }
        let long_ago = std::time::SystemTime::now() - STALE_TEMP_AGE * 2;
        std::fs::File::options()
            .write(true)
            .open(&stale_tmp)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();

        remove_stale_scaled(dir.path());

        assert!(!old.exists());
        assert!(copy.exists());
        assert!(fresh_tmp.exists());
        assert!(!stale_tmp.exists());
    }

    #[test]
    fn a_corrupt_copy_is_decoded_again_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.png");
        sample(&cover, 300, 450, false);
        let saved = scaled_path(&cover, THUMBNAIL_HEIGHT, "qoi").unwrap();
        std::fs::create_dir_all(saved.parent().unwrap()).unwrap();
        std::fs::write(&saved, b"not an image").unwrap();

        assert!(read_scaled(&cover, THUMBNAIL_HEIGHT).is_none());
        let decoded = load_or_decode(&cover, THUMBNAIL_HEIGHT).unwrap();
        let reused = read_scaled(&cover, THUMBNAIL_HEIGHT).unwrap();
        assert_eq!(pixels(&reused), pixels(&decoded));
    }
}
