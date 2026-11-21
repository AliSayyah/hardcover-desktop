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
use std::time::{Duration, SystemTime};

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
/// Height of decoded xs and sm covers (list rows, series strips), in image
/// pixels. An sm cover is 48 px tall, so this is sharp up to 3x scale.
pub(crate) const THUMBNAIL_HEIGHT: u32 = 144;
/// Height of decoded md and lg covers (Home's rows and shelves, similar
/// books), in image pixels. An lg cover is 120 px tall, so this is sharp up
/// to 2.4x scale, and an md one (96 px) up to 3x.
pub(crate) const MEDIUM_COVER_HEIGHT: u32 = 288;
/// Height of the decoded detail cover, in image pixels. The cover is 12rem
/// tall, so this is sharp up to 2.5x scale with the default rem.
const COVER_HEIGHT: u32 = 480;
/// Every height a cover is scaled to. A download is decoded once into all of
/// them, and then deleted: the scaled copies are the cache.
const SCALED_HEIGHTS: [u32; 3] = [THUMBNAIL_HEIGHT, MEDIUM_COVER_HEIGHT, COVER_HEIGHT];
/// The most disk the cover cache may take. Past it, the copies used least
/// recently are deleted at the next start, down to [`COVER_CACHE_TARGET`].
/// A cover takes about 110 KB at all three sizes, so this holds some 2,300
/// covers.
const COVER_CACHE_LIMIT: u64 = 256 * 1024 * 1024;
const COVER_CACHE_TARGET: u64 = COVER_CACHE_LIMIT / 4 * 3;
/// A copy is marked as used (its modification time set) at most this often,
/// so the size limit deletes the copies used least recently.
const TOUCH_AFTER: Duration = Duration::from_hours(24);

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
    /// Blocking: creates the cache directory, and tidies it on another thread.
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
        let decoder = CoverDecoder::spawn();
        if let Some(cache) = &cache {
            // A walk of the whole cache: not on the way to the first screen.
            let dir = cache.dir().to_owned();
            let decoder = decoder.clone();
            let spawned = thread::Builder::new()
                .name("cover-tidy".into())
                .spawn(move || tidy_cover_dir(&dir, &decoder));
            if let Err(e) = spawned {
                eprintln!("hardcover: could not tidy the cover cache: {e}");
            }
        }
        Self {
            cache,
            downloads: Arc::new(Semaphore::new(COVER_DOWNLOADS)),
            decoder,
            failed: Arc::default(),
        }
    }

    /// Forgets failed loads so the next render tries them again.
    pub fn retry_failed(cx: &mut App) {
        let Some(service) = cx.try_global::<Self>() else {
            return;
        };
        let failed: Vec<String> = std::mem::take(
            &mut *service
                .failed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
        .into_iter()
        .collect();
        for url in failed {
            cx.remove_asset::<CoverFile>(&url);
            cx.remove_asset::<CoverThumbnail>(&url);
            cx.remove_asset::<CoverMedium>(&url);
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

/// What the decoder made of a cover.
enum Loaded {
    Image(Arc<RenderImage>),
    /// Neither a scaled copy nor the download is on disk.
    NoSource,
    Failed,
}

/// One request to [`CoverDecoder`]: the download's path, the target height
/// and where to send the result.
struct DecodeJob {
    path: Arc<Path>,
    max_height: u32,
    reply: async_channel::Sender<Loaded>,
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
                let mut deleted = HashSet::new();
                while let Ok(job) = queue.recv_blocking() {
                    // The requester may have gone away; nothing to do then.
                    let _ = job.reply.send_blocking(load_or_decode(
                        &job.path,
                        job.max_height,
                        &mut deleted,
                    ));
                }
            });
        if let Err(e) = spawned {
            // Without the thread every decode below fails, which the views
            // show as the placeholder cover.
            eprintln!("hardcover: cover decoder unavailable: {e}");
        }
        Self { jobs }
    }

    async fn decode(&self, path: Arc<Path>, max_height: u32) -> Loaded {
        let (reply, result) = async_channel::bounded(1);
        let sent = self
            .jobs
            .send(DecodeJob {
                path,
                max_height,
                reply,
            })
            .await;
        if sent.is_err() {
            return Loaded::Failed;
        }
        result.recv().await.unwrap_or(Loaded::Failed)
    }

    /// [`Self::decode`], blocking the calling thread until it is done.
    fn decode_blocking(&self, path: Arc<Path>, max_height: u32) -> Loaded {
        let (reply, result) = async_channel::bounded(1);
        let sent = self.jobs.send_blocking(DecodeJob {
            path,
            max_height,
            reply,
        });
        if sent.is_err() {
            return Loaded::Failed;
        }
        result.recv_blocking().unwrap_or(Loaded::Failed)
    }
}

impl CoverLoader {
    /// The cover file on disk, downloading it (behind the limiter) if needed.
    /// For images the decoder can't read (an SVG avatar), which are kept as
    /// downloaded.
    async fn file(&self, url: &str) -> Option<Arc<Path>> {
        let cover = Cover::new(url);
        if let Some(path) = self.cache.cached_path(&cover) {
            return Some(path.into());
        }
        let _permit = self.downloads.acquire().await;
        if let Ok(path) = self.cache.fetch(&cover) {
            Some(path.into())
        } else {
            self.failed(url.to_owned());
            None
        }
    }

    /// Downloads the cover, behind the limiter.
    async fn download(&self, url: &str) -> bool {
        let _permit = self.downloads.acquire().await;
        self.cache.fetch(&Cover::new(url)).is_ok()
    }

    /// Records a failed load, tried again after the next sync.
    fn failed(&self, url: String) {
        self.failed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(url);
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

/// An md or lg cover, scaled to [`MEDIUM_COVER_HEIGHT`]. Home's are library
/// books and stay for the run; similar books are released when the book page
/// moves on (see [`release_cover_medium`]).
pub enum CoverMedium {}

impl Asset for CoverMedium {
    type Source = String;
    type Output = Option<Arc<RenderImage>>;

    fn load(url: String, cx: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        load_scaled(CoverService::loader(cx), url, MEDIUM_COVER_HEIGHT)
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

/// The cover scaled to `max_height`: its scaled copy, else the download
/// decoded, else downloaded first.
async fn load_scaled(
    loader: Option<CoverLoader>,
    url: String,
    max_height: u32,
) -> Option<Arc<RenderImage>> {
    let loader = loader?;
    let path: Arc<Path> = loader.cache.file_path(&Cover::new(&url)).ok()?.into();
    let mut loaded = loader.decoder.decode(path.clone(), max_height).await;
    if matches!(loaded, Loaded::NoSource) && loader.download(&url).await {
        loaded = loader.decoder.decode(path, max_height).await;
    }
    if let Loaded::Image(image) = loaded {
        Some(image)
    } else {
        loader.failed(url);
        None
    }
}

/// A decoded cover at full size, in the pixel layout it decoded to.
struct Source {
    width: u32,
    height: u32,
    pixel_type: PixelType,
    pixels: Vec<u8>,
}

/// Why a download could not be decoded.
enum SourceError {
    /// It isn't on disk.
    Missing,
    /// It is, but can't be read (permissions, say).
    Unreadable,
    /// It was read, but is not an image this build decodes: a download cut
    /// short, or an unsupported format.
    Undecodable,
}

fn decode_source(path: &Path) -> Result<Source, SourceError> {
    let reader = image::ImageReader::open(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SourceError::Missing
        } else {
            SourceError::Unreadable
        }
    })?;
    let image = reader
        .with_guessed_format()
        .map_err(|_| SourceError::Unreadable)?
        .decode()
        .map_err(|_| SourceError::Undecodable)?;
    let (width, height) = (image.width(), image.height());
    // Covers are mostly JPEG, which decodes to RGB; resize that without
    // adding an alpha channel first.
    let (pixel_type, pixels) = match image {
        image::DynamicImage::ImageRgb8(rgb) => (PixelType::U8x3, rgb.into_raw()),
        other => (PixelType::U8x4, other.into_rgba8().into_raw()),
    };
    Ok(Source {
        width,
        height,
        pixel_type,
        pixels,
    })
}

/// `source` scaled down to at most `max_height` pixels tall, as BGRA.
fn scale(source: &Source, max_height: u32) -> Option<Arc<RenderImage>> {
    let (width, height) = scaled_size(source.width, source.height, max_height);
    let pixel_type = source.pixel_type;
    let pixels = if (width, height) == (source.width, source.height) {
        source.pixels.clone()
    } else {
        let src =
            fir::images::ImageRef::new(source.width, source.height, &source.pixels, pixel_type)
                .ok()?;
        let mut dst = fir::images::Image::new(width, height, pixel_type);
        // Bilinear convolution is the Triangle filter `image` uses, with SIMD.
        let options = fir::ResizeOptions::new()
            .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Bilinear))
            .use_alpha(false);
        fir::Resizer::new().resize(&src, &mut dst, &options).ok()?;
        dst.into_vec()
    };
    // GPUI's sprite atlas expects BGRA.
    let bgra = if pixel_type == PixelType::U8x3 {
        pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0], u8::MAX])
            .collect()
    } else {
        let mut pixels = pixels;
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        pixels
    };
    let frame = image::RgbaImage::from_raw(width, height, bgra)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(frame)])))
}

/// Decodes an image file, scaled down to at most `max_height` pixels tall.
#[cfg(test)]
pub(crate) fn decode_scaled(path: &Path, max_height: u32) -> Option<Arc<RenderImage>> {
    scale(&decode_source(path).ok()?, max_height)
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
    let width = u32::try_from(u64::from(src_width) * u64::from(height) / u64::from(src_height))
        .expect("downscaled width fits in the source width")
        .max(1);
    let ratio = f64::min(
        f64::from(width) / f64::from(src_width.max(1)),
        f64::from(height) / f64::from(src_height),
    );
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the positive downscale ratio never exceeds one; rounded dimensions fit in u32"
    )]
    let fit = |side: u32| ((f64::from(side) * ratio).round() as u32).max(1);
    (fit(src_width), fit(src_height))
}

// ----- scaled copies -----

/// Scaled copies of covers live in `scaled/<version>/` under the cover cache.
const SCALED_DIR: &str = "scaled";
/// Bump whenever [`scale`] changes its output: copies made by an
/// older build are then ignored, and deleted at the next start.
const SCALED_VERSION: &str = "v1";
/// A temporary file older than this was left by a write that never finished.
const STALE_TEMP_AGE: std::time::Duration = std::time::Duration::from_hours(1);
/// JPEG quality of saved medium and detail copies.
const SCALED_JPEG_QUALITY: u8 = 92;

/// The cover scaled to `max_height`, from its saved copy when there is one.
///
/// Otherwise the download at `path` is decoded once and saved at every size
/// in [`SCALED_HEIGHTS`], then deleted: a relaunch shows covers without
/// decoding any, and the cache keeps a few small copies instead of a large
/// original (most downloads are over 1,000 pixels tall). Should saving fail,
/// the download stays and the next run decodes it again.
///
/// A download that doesn't decode is deleted, so the next try downloads it
/// again (it may have been cut short), but once per run at most: `deleted`
/// remembers it, so an image this build can't read isn't downloaded over and
/// over.
fn load_or_decode(path: &Path, max_height: u32, deleted: &mut HashSet<PathBuf>) -> Loaded {
    if let Some(image) = read_scaled(path, max_height) {
        return Loaded::Image(image);
    }
    let source = match decode_source(path) {
        Ok(source) => source,
        Err(SourceError::Missing) => return from_larger_copy(path, max_height),
        Err(SourceError::Unreadable) => return Loaded::Failed,
        Err(SourceError::Undecodable) => {
            if deleted.insert(path.to_owned()) {
                let _ = std::fs::remove_file(path);
            }
            return Loaded::Failed;
        }
    };
    let Some(image) = scale(&source, max_height) else {
        return Loaded::Failed;
    };
    let mut saved = save_scaled(path, max_height, &image);
    for other in SCALED_HEIGHTS {
        if other != max_height && !has_scaled(path, other) {
            saved &= scale(&source, other).is_some_and(|copy| save_scaled(path, other, &copy));
        }
    }
    if saved {
        let _ = std::fs::remove_file(path);
    }
    Loaded::Image(image)
}

/// The cover scaled to `max_height` from a larger saved copy, when the
/// download is gone. A cache made before a size was added has the other
/// copies only; this makes the new one without downloading the cover again.
/// The copy is saved as usual. [`Loaded::NoSource`] when there is none.
fn from_larger_copy(path: &Path, max_height: u32) -> Loaded {
    let larger = SCALED_HEIGHTS
        .into_iter()
        .filter(|&h| h > max_height)
        .flat_map(|h| ["qoi", "jpg"].map(|ext| scaled_path(path, h, ext)))
        .flatten()
        .find_map(|copy| decode_source(&copy).ok());
    let Some(source) = larger else {
        return Loaded::NoSource;
    };
    let Some(image) = scale(&source, max_height) else {
        return Loaded::Failed;
    };
    save_scaled(path, max_height, &image);
    Loaded::Image(image)
}

/// Saves a scaled copy; false if that failed.
fn save_scaled(cover: &Path, max_height: u32, image: &RenderImage) -> bool {
    let Err(e) = write_scaled(cover, max_height, image) else {
        return true;
    };
    // The cover still shows; the next run decodes it again. Said once,
    // since the cause (say, a read-only cache) usually affects them all.
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        eprintln!("hardcover: could not save a scaled cover: {e}");
    }
    false
}

/// True if a copy of `cover` scaled to `max_height` is on disk.
fn has_scaled(cover: &Path, max_height: u32) -> bool {
    ["qoi", "jpg"]
        .into_iter()
        .any(|ext| scaled_path(cover, max_height, ext).is_some_and(|p| p.is_file()))
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
        let path = scaled_path(cover, max_height, ext)?;
        let bytes = std::fs::read(&path).ok()?;
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
        mark_used(&path);
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
    })
}

/// Sets the file's modification time to now, unless that was done within
/// [`TOUCH_AFTER`]: the size limit deletes the files untouched the longest.
fn mark_used(path: &Path) {
    let now = SystemTime::now();
    let recent = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_none_or(|age| age < TOUCH_AFTER);
    if !recent && let Ok(file) = std::fs::File::options().write(true).open(path) {
        let _ = file.set_modified(now);
    }
}

/// Saves a scaled cover next to the originals. Row thumbnails are small, so
/// they are kept lossless: QOI of the BGRA pixels as they are. Larger copies
/// (medium and detail) are high-quality JPEG when opaque, about a fifth of the
/// size; a cover with transparency stays QOI.
fn write_scaled(cover: &Path, max_height: u32, image: &RenderImage) -> std::io::Result<()> {
    use image::ImageEncoder as _;

    let size = image.size(0);
    let dimension =
        |n| u32::try_from(n).map_err(|_| std::io::Error::other("negative image dimension"));
    let (width, height) = (dimension(size.width.0)?, dimension(size.height.0)?);
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

/// Tidies the cover cache at startup. Blocking; errors are ignored, as
/// everything in it can be downloaded again.
fn tidy_cover_dir(cover_dir: &Path, decoder: &CoverDecoder) {
    remove_stale_scaled(cover_dir);
    // One at a time, each awaited, so a cover being shown waits behind one
    // conversion at most. On the decoder's thread, for its one malloc arena.
    for download in sweep_downloads(cover_dir) {
        decoder.decode_blocking(download.into(), COVER_HEIGHT);
    }
    enforce_size_limit(cover_dir, COVER_CACHE_LIMIT, COVER_CACHE_TARGET);
}

/// Deletes temporary files left by downloads that never finished, and
/// downloads that have every scaled copy (a crash between saving the copies
/// and deleting it). Returns the other downloads, which earlier builds kept
/// next to their copies, to be converted: decoding one at
/// [`COVER_HEIGHT`] saves the copies it lacks and deletes it. SVGs are left
/// alone: they are shown as downloaded, never decoded.
fn sweep_downloads(cover_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(cover_dir) else {
        return Vec::new();
    };
    let mut convert = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        // Recent ones may still be in use, here or by another instance.
        let stale = meta
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STALE_TEMP_AGE);
        if !meta.is_file() || !stale {
            continue;
        }
        let extension = path.extension().and_then(|ext| ext.to_str());
        if extension == Some("tmp") || SCALED_HEIGHTS.iter().all(|&h| has_scaled(&path, h)) {
            let _ = std::fs::remove_file(path);
        } else if extension != Some("svg") {
            convert.push(path);
        }
    }
    convert
}

/// Past `limit` bytes, deletes the files used least recently (downloads and
/// current scaled copies) until at most `target` bytes are left.
fn enforce_size_limit(cover_dir: &Path, limit: u64, target: u64) {
    let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    for dir in [
        cover_dir.to_owned(),
        cover_dir.join(SCALED_DIR).join(SCALED_VERSION),
    ] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_file() {
                let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                files.push((modified, meta.len(), entry.path()));
            }
        }
    }
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    if total <= limit {
        return;
    }
    files.sort_unstable_by_key(|(modified, _, _)| *modified);
    for (_, size, path) in files {
        if total <= target {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total -= size;
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

/// Forgets an md or lg cover no view shows any more, and its texture. A view
/// that still shows it loads it again from the saved copy. `window` is the
/// window being updated, if any: GPUI can't reach it through `cx` then.
pub fn release_cover_medium(url: &str, window: Option<&mut Window>, cx: &mut App) {
    let url = url.to_owned();
    // Already in the cache, since a view showed it: this starts no load.
    let image = cx.fetch_asset::<CoverMedium>(&url).flatten();
    cx.remove_asset::<CoverMedium>(&url);
    if let Some(image) = image {
        cx.drop_image(image, window);
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
            image::Rgba([
                u8::try_from(x * 255 / width).unwrap(),
                u8::try_from(y * 255 / height).unwrap(),
                90,
                alpha,
            ])
        });
        image.save(path).unwrap();
    }

    fn pixels(image: &RenderImage) -> (u32, u32, Vec<u8>) {
        let size = image.size(0);
        (
            u32::try_from(size.width.0).unwrap(),
            u32::try_from(size.height.0).unwrap(),
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

    /// [`load_or_decode`] with a fresh record of deleted downloads.
    fn load(cover: &Path, max_height: u32) -> Option<Arc<RenderImage>> {
        match load_or_decode(cover, max_height, &mut HashSet::new()) {
            Loaded::Image(image) => Some(image),
            Loaded::NoSource | Loaded::Failed => None,
        }
    }

    #[test]
    fn thumbnails_are_saved_losslessly_and_reused() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.png");
        sample(&cover, 300, 450, false);

        let decoded = load(&cover, THUMBNAIL_HEIGHT).unwrap();
        let saved = scaled_path(&cover, THUMBNAIL_HEIGHT, "qoi").unwrap();
        assert!(saved.is_file());
        assert_eq!(
            saved.parent().unwrap(),
            dir.path().join(SCALED_DIR).join(SCALED_VERSION)
        );

        let reused = read_scaled(&cover, THUMBNAIL_HEIGHT).unwrap();
        assert_eq!(pixels(&reused), pixels(&decoded));
        // The thumbnail, the medium and the detail copy, and no temporary
        // files.
        assert_eq!(
            std::fs::read_dir(saved.parent().unwrap()).unwrap().count(),
            3
        );
    }

    #[test]
    fn a_download_is_decoded_once_into_every_size_then_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.png");
        sample(&cover, 600, 900, false);
        let thumbnail = load(&cover, THUMBNAIL_HEIGHT).unwrap();
        assert!(!cover.exists(), "the download is deleted");
        for height in SCALED_HEIGHTS {
            assert!(has_scaled(&cover, height));
        }

        // Every size come from the copies now, as the source decoded them.
        assert_eq!(
            pixels(&load(&cover, THUMBNAIL_HEIGHT).unwrap()),
            pixels(&thumbnail)
        );
        let detail = load(&cover, COVER_HEIGHT).unwrap();
        assert_eq!(pixels(&detail).0, 320);
        assert_eq!(pixels(&detail).1, 480);
    }

    #[test]
    fn a_download_stays_when_its_copies_cannot_be_saved() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.png");
        sample(&cover, 300, 450, false);
        // A file where the copies' directory should be.
        std::fs::write(dir.path().join(SCALED_DIR), b"x").unwrap();
        assert!(load(&cover, THUMBNAIL_HEIGHT).is_some());
        assert!(cover.exists());
    }

    #[test]
    fn a_size_missing_from_an_older_cache_is_made_from_a_larger_copy() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.jpg");
        let source = dir.path().join("source.png");
        sample(&source, 600, 900, false);
        // The copies an older build saved, with the download deleted.
        for height in [THUMBNAIL_HEIGHT, COVER_HEIGHT] {
            write_scaled(&cover, height, &decode_scaled(&source, height).unwrap()).unwrap();
        }

        let medium = load(&cover, MEDIUM_COVER_HEIGHT).unwrap();
        assert_eq!((pixels(&medium).0, pixels(&medium).1), (192, 288));
        assert!(
            has_scaled(&cover, MEDIUM_COVER_HEIGHT),
            "saved for next time"
        );
        // Close to scaling the original: the detail copy is a JPEG.
        let direct = decode_scaled(&source, MEDIUM_COVER_HEIGHT).unwrap();
        let (_, _, made) = pixels(&medium);
        let (_, _, expected) = pixels(&direct);
        let worst = made
            .iter()
            .zip(&expected)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(worst <= 8, "off by {worst}");
        // The thumbnail is still read from its own copy.
        assert!(read_scaled(&cover, THUMBNAIL_HEIGHT).is_some());
    }

    #[test]
    fn nothing_on_disk_asks_for_a_download() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.jpg");
        assert!(matches!(
            load_or_decode(&cover, THUMBNAIL_HEIGHT, &mut HashSet::new()),
            Loaded::NoSource
        ));
    }

    #[test]
    fn a_broken_download_is_deleted_once_per_run() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.jpg");
        let mut deleted = HashSet::new();
        std::fs::write(&cover, b"not an image").unwrap();
        assert!(matches!(
            load_or_decode(&cover, THUMBNAIL_HEIGHT, &mut deleted),
            Loaded::Failed
        ));
        assert!(!cover.exists(), "deleted, so the retry downloads it again");

        // Downloaded again and still broken: kept, so it isn't downloaded
        // over and over.
        std::fs::write(&cover, b"not an image").unwrap();
        assert!(matches!(
            load_or_decode(&cover, THUMBNAIL_HEIGHT, &mut deleted),
            Loaded::Failed
        ));
        assert!(cover.exists());
    }

    fn set_age(path: &Path, age: Duration) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - age)
            .unwrap();
    }

    #[test]
    fn copies_are_marked_as_used_at_most_daily() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.png");
        sample(&cover, 300, 450, false);
        load(&cover, THUMBNAIL_HEIGHT).unwrap();
        let copy = scaled_path(&cover, THUMBNAIL_HEIGHT, "qoi").unwrap();
        let modified = || std::fs::metadata(&copy).unwrap().modified().unwrap();

        set_age(&copy, Duration::from_hours(2));
        let before = modified();
        read_scaled(&cover, THUMBNAIL_HEIGHT).unwrap();
        assert_eq!(modified(), before, "used within a day: left alone");

        set_age(&copy, Duration::from_hours(48));
        read_scaled(&cover, THUMBNAIL_HEIGHT).unwrap();
        assert!(modified().elapsed().unwrap() < Duration::from_hours(1));
    }

    #[test]
    fn old_downloads_are_converted_and_leftovers_removed() {
        let dir = tempfile::tempdir().unwrap();
        let old = Duration::from_hours(2);
        // As earlier builds left it: a download next to its thumbnail.
        let kept_before = dir.path().join("kept.png");
        sample(&kept_before, 600, 900, false);
        let thumbnail = decode_scaled(&kept_before, THUMBNAIL_HEIGHT).unwrap();
        write_scaled(&kept_before, THUMBNAIL_HEIGHT, &thumbnail).unwrap();
        set_age(&kept_before, old);
        // One with both copies (a crash before it was deleted), a recent one
        // (it may be being decoded), an SVG avatar and temporary files.
        let both = dir.path().join("both.png");
        sample(&both, 300, 450, false);
        load(&both, THUMBNAIL_HEIGHT).unwrap();
        sample(&both, 300, 450, false);
        let recent = dir.path().join("recent.png");
        sample(&recent, 300, 450, false);
        let svg = dir.path().join("avatar.svg");
        let stale_tmp = dir.path().join(".a.jpg.x.tmp");
        let fresh_tmp = dir.path().join(".b.jpg.y.tmp");
        for path in [&svg, &stale_tmp, &fresh_tmp] {
            std::fs::write(path, b"x").unwrap();
        }
        for path in [&both, &svg, &stale_tmp] {
            set_age(path, old);
        }

        let convert = sweep_downloads(dir.path());
        assert_eq!(convert, std::slice::from_ref(&kept_before));
        assert!(!both.exists());
        assert!(!stale_tmp.exists());
        for kept in [&recent, &svg, &fresh_tmp, &kept_before] {
            assert!(kept.exists(), "{}", kept.display());
        }

        // Converting saves the detail copy it lacked, keeps the thumbnail as
        // it was, and deletes the download.
        let thumbnail = pixels(&thumbnail);
        load(&kept_before, COVER_HEIGHT).unwrap();
        assert!(!kept_before.exists());
        assert!(has_scaled(&kept_before, COVER_HEIGHT));
        let reused = read_scaled(&kept_before, THUMBNAIL_HEIGHT).unwrap();
        assert_eq!(pixels(&reused), thumbnail);
    }

    #[test]
    fn past_the_limit_the_least_recently_used_files_go() {
        let dir = tempfile::tempdir().unwrap();
        let scaled = dir.path().join(SCALED_DIR).join(SCALED_VERSION);
        std::fs::create_dir_all(&scaled).unwrap();
        let files: Vec<PathBuf> = (0..4)
            .map(|i| scaled.join(format!("{i}-144.qoi")))
            .chain([dir.path().join("download.jpg")])
            .collect();
        for (age, path) in files.iter().enumerate() {
            std::fs::write(path, [0u8; 100]).unwrap();
            // download.jpg is the oldest, 0-144.qoi the newest.
            set_age(path, Duration::from_hours(1 + age as u64));
        }

        enforce_size_limit(dir.path(), 500, 200);
        assert!(
            files.iter().all(|f| f.exists()),
            "at the limit: nothing goes"
        );

        enforce_size_limit(dir.path(), 499, 200);
        let left: Vec<bool> = files.iter().map(|f| f.exists()).collect();
        assert_eq!(left, [true, true, false, false, false]);
    }

    #[test]
    fn opaque_detail_covers_are_saved_as_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.png");
        sample(&cover, 600, 900, false);

        let decoded = load(&cover, COVER_HEIGHT).unwrap();
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

        let decoded = load(&cover, COVER_HEIGHT).unwrap();
        assert!(scaled_path(&cover, COVER_HEIGHT, "qoi").unwrap().is_file());
        let reused = read_scaled(&cover, COVER_HEIGHT).unwrap();
        assert_eq!(pixels(&reused), pixels(&decoded));
    }

    #[test]
    fn a_truncated_jpeg_copy_is_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let cover = dir.path().join("cover.png");
        sample(&cover, 600, 900, false);
        load(&cover, COVER_HEIGHT).unwrap();
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
        let decoded = load(&cover, THUMBNAIL_HEIGHT).unwrap();
        let reused = read_scaled(&cover, THUMBNAIL_HEIGHT).unwrap();
        assert_eq!(pixels(&reused), pixels(&decoded));
    }
}
