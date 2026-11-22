//! The cover pipeline: covers (and avatars) from URL to GPUI image.
//!
//! - **Assets.** Covers are GPUI `Asset`s at three heights ([`CoverThumbnail`],
//!   [`CoverMedium`], [`CoverImage`]) plus [`CoverFile`] for an SVG, loaded
//!   with [`load_cover`] or [`cover_file`], which also stamp the cover as
//!   drawn ([`usage`]).
//! - **Threads.** Nothing here blocks a GPUI thread. Downloads run on four
//!   `cover-download` threads, one per URL however many sizes ask
//!   ([`queue::Downloader`]); decoding, scaling and saving copies run on one
//!   `cover-decoder` thread ([`Decoder`]). Both serve the cover drawn most
//!   recently first, and skip a cover nobody waits for any more.
//! - **Copies.** A download is decoded once into every size and deleted; the
//!   scaled copies are the cache ([`scaled`]).
//! - **Memory.** Loaded thumbnails and medium covers past a budget are
//!   released, those drawn longest ago first ([`CoverService::release_unused`]),
//!   and every window is drawn again, since a cached view that sits still
//!   stamps nothing. The detail cover is released by the pane that shows it.

mod queue;
mod scaled;
mod usage;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use gpui_kit::{App, Asset, Global, RenderImage, Window};
use hardcover_core::covers::CoverCache;
use hardcover_core::{Cover, Error};

use queue::{Downloader, Fetched, Job, Queue, lock};
pub(crate) use scaled::{COVER_HEIGHT, MEDIUM_COVER_HEIGHT, THUMBNAIL_HEIGHT};
use scaled::{Decoding, Loaded, TidyJob};
#[cfg(test)]
pub(crate) use scaled::{SCALED_JPEG_QUALITY, decode_scaled};
use usage::{Kind, Usage};

/// Threads downloading covers.
const COVER_DOWNLOADS: usize = 4;

/// The cover cache, the download and decoder threads, and what is drawn.
/// A GPUI global for the whole run.
pub struct CoverService {
    cache: Option<Arc<CoverCache>>,
    usage: Arc<Mutex<Usage>>,
    downloader: Downloader,
    decoder: Decoder,
    /// URLs whose load failed, so they can be retried after a later sync.
    /// Not those that can't succeed ([`Loaded::Unsupported`]).
    failed: Arc<Mutex<HashSet<String>>>,
    /// A release of unused covers is already on its way.
    release_scheduled: AtomicBool,
}

impl Global for CoverService {}

impl CoverService {
    /// Blocking: creates the cache directory, and tidies it on another thread.
    pub fn open() -> Self {
        #[cfg(feature = "demo")]
        let cache = hardcover_core::demo::cover_cache(
            std::env::temp_dir().join("hardcover-desktop-demo-covers"),
            hardcover_core::demo::env_faults(),
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
        let service = Self::with_cache(cache, Usage::default());
        if let Some(cache) = &service.cache {
            // A walk of the whole cache: not on the way to the first screen.
            let dir = cache.dir().to_owned();
            let decoder = service.decoder.clone();
            let spawned = thread::Builder::new()
                .name("cover-tidy".into())
                .spawn(move || scaled::tidy_cover_dir(&dir, |job| decoder.tidy(job)));
            if let Err(e) = spawned {
                eprintln!("hardcover: could not tidy the cover cache: {e}");
            }
        }
        service
    }

    fn with_cache(cache: Option<Arc<CoverCache>>, usage: Usage) -> Self {
        let usage = Arc::new(Mutex::new(usage));
        let decoder = Decoder::spawn(cache.as_ref().map(|c| c.dir().to_owned()), &usage);
        let workers = if cache.is_some() { COVER_DOWNLOADS } else { 0 };
        let downloader = Downloader::spawn(workers, usage.clone(), {
            let cache = cache.clone();
            move |url| cache.as_deref().map_or(Fetched::Failed, |c| fetch(c, url))
        });
        Self {
            cache,
            usage,
            downloader,
            decoder,
            failed: Arc::default(),
            release_scheduled: AtomicBool::new(false),
        }
    }

    /// Forgets failed loads so the next render tries them again.
    pub fn retry_failed(cx: &mut App) {
        let Some(service) = cx.try_global::<Self>() else {
            return;
        };
        let failed: Vec<String> = std::mem::take(&mut *lock(&service.failed))
            .into_iter()
            .collect();
        {
            let mut usage = lock(&service.usage);
            for url in &failed {
                usage.forget_all(url);
            }
        }
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
            downloader: service.downloader.clone(),
            decoder: service.decoder.clone(),
            failed: service.failed.clone(),
        })
    }

    /// Records that `url` is being drawn at `kind`; past a budget, schedules
    /// [`Self::release_unused`].
    fn stamp(url: &str, kind: Kind, cx: &mut App) {
        let Some(service) = cx.try_global::<Self>() else {
            return;
        };
        let over = lock(&service.usage).stamp(url, kind);
        if over && !service.release_scheduled.swap(true, Ordering::Relaxed) {
            cx.defer(Self::release_unused);
        }
    }

    /// Releases the loaded thumbnails and medium covers past their budget
    /// ([`usage::THUMBNAIL_BUDGET`], [`usage::MEDIUM_BUDGET`]), those drawn
    /// longest ago first, with their textures, so a long session's memory
    /// stays bounded. One drawn again loads again from its saved copy.
    pub(crate) fn release_unused(cx: &mut App) {
        let Some(service) = cx.try_global::<Self>() else {
            return;
        };
        service.release_scheduled.store(false, Ordering::Relaxed);
        let (thumbnails, medium) = {
            let mut usage = lock(&service.usage);
            (usage.evict(Kind::Thumbnail), usage.evict(Kind::Medium))
        };
        let released = !thumbnails.is_empty() || !medium.is_empty();
        for url in thumbnails {
            release::<CoverThumbnail>(&url, None, cx);
        }
        for url in medium {
            release::<CoverMedium>(&url, None, cx);
        }
        // A cached view that sits still shows its covers without drawing
        // them, so it stamps none: draw every view again, so one still
        // showing a released cover loads it again, and the others' covers
        // count as drawn now.
        if released {
            cx.refresh_windows();
        }
    }
}

/// A download thread's work: nothing if the file or every copy is already on
/// disk (another size of the cover got there first), else the download.
fn fetch(cache: &CoverCache, url: &str) -> Fetched {
    let cover = Cover::new(url);
    let Ok(path) = cache.file_path(&cover) else {
        return Fetched::Unsupported;
    };
    if path.is_file() || scaled::has_every_copy(&path) {
        return Fetched::Done;
    }
    match cache.fetch(&cover) {
        Ok(_) => Fetched::Done,
        // Not an image the app can show, or a URL it doesn't fetch.
        Err(Error::Decode(_) | Error::InvalidInput(_)) => Fetched::Unsupported,
        Err(_) => Fetched::Failed,
    }
}

/// What an asset's load needs, taken from the global when it starts.
struct CoverLoader {
    cache: Arc<CoverCache>,
    downloader: Downloader,
    decoder: Decoder,
    failed: Arc<Mutex<HashSet<String>>>,
}

impl CoverLoader {
    /// The cover file on disk, downloading it if needed. For images the
    /// decoder can't read (an SVG avatar), which are kept as downloaded.
    async fn file(&self, url: &str) -> Option<Arc<Path>> {
        let cover = Cover::new(url);
        if let Some(path) = self.cache.cached_path(&cover) {
            return Some(path.into());
        }
        match self.downloader.download(url).await {
            Fetched::Done => {
                let path = self.cache.cached_path(&cover);
                if path.is_none() {
                    self.failed(url.to_owned());
                }
                path.map(Into::into)
            }
            Fetched::Failed => {
                self.failed(url.to_owned());
                None
            }
            Fetched::Unsupported => None,
        }
    }

    /// The cover scaled to `max_height`: its scaled copy, else the download
    /// decoded, else downloaded first.
    async fn scaled(&self, url: String, max_height: u32) -> Option<Arc<RenderImage>> {
        let path: Arc<Path> = self.cache.file_path(&Cover::new(&url)).ok()?.into();
        let mut loaded = self.decoder.load(path.clone(), max_height, &url).await;
        if matches!(loaded, Loaded::NoSource) {
            loaded = match self.downloader.download(&url).await {
                Fetched::Done => self.decoder.load(path, max_height, &url).await,
                Fetched::Failed => Loaded::Failed,
                Fetched::Unsupported => Loaded::Unsupported,
            };
        }
        match loaded {
            Loaded::Image(image) => Some(image),
            Loaded::Unsupported => None,
            Loaded::NoSource | Loaded::Failed => {
                self.failed(url);
                None
            }
        }
    }

    /// Records a failed load, tried again after the next sync.
    fn failed(&self, url: String) {
        lock(&self.failed).insert(url);
    }
}

// ----- the decoder thread -----

/// What the decoder thread is asked to do.
enum Work {
    Load { path: Arc<Path>, max_height: u32 },
    Tidy(TidyJob),
}

struct DecodeJob {
    work: Work,
    /// The cover it is for, which gives its priority; none for the tidy,
    /// which goes last.
    url: Option<String>,
    reply: async_channel::Sender<Loaded>,
}

impl Job for DecodeJob {
    fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
}

/// Decodes covers, and saves and reads their copies, on one dedicated thread.
///
/// Decoding a large cover allocates tens of megabytes that are freed right
/// after scaling. glibc keeps freed memory in the arena of the thread that
/// allocated it, so decoding on GPUI's many background threads left one
/// bloated arena per thread: hundreds of megabytes of resident memory after
/// browsing a few hundred covers. One thread means one arena, which is reused
/// for every decode. A cover takes a few milliseconds, so one thread keeps up.
#[derive(Clone)]
struct Decoder {
    queue: Arc<Queue<DecodeJob>>,
}

impl Decoder {
    fn spawn(cover_dir: Option<PathBuf>, usage: &Arc<Mutex<Usage>>) -> Self {
        let decoding = Mutex::new(Decoding::new(cover_dir));
        let queue = Queue::spawn("cover-decoder", 1, usage.clone(), move |job: DecodeJob| {
            if job.reply.is_closed() {
                // Released before its turn came: nobody waits for it.
                return;
            }
            let mut decoding = lock(&decoding);
            let loaded = match job.work {
                Work::Load { path, max_height } => decoding.load(&path, max_height),
                Work::Tidy(TidyJob::Convert(download)) => {
                    decoding.convert(&download);
                    Loaded::NoSource
                }
                Work::Tidy(TidyJob::Rebuild { cover, older }) => {
                    decoding.rebuild(&cover, &older);
                    Loaded::NoSource
                }
            };
            let _ = job.reply.try_send(loaded);
        });
        Self { queue }
    }

    async fn load(&self, path: Arc<Path>, max_height: u32, url: &str) -> Loaded {
        let (reply, result) = async_channel::bounded(1);
        let job = DecodeJob {
            work: Work::Load { path, max_height },
            url: Some(url.to_owned()),
            reply,
        };
        if self.queue.push(job).is_err() {
            // Without the thread every decode fails, which the views show
            // as the cloth cover.
            return Loaded::Failed;
        }
        result.recv().await.unwrap_or(Loaded::Failed)
    }

    /// Runs a tidy job, blocking the calling thread until it is done.
    fn tidy(&self, work: TidyJob) {
        let (reply, result) = async_channel::bounded(1);
        let job = DecodeJob {
            work: Work::Tidy(work),
            url: None,
            reply,
        };
        if self.queue.push(job).is_ok() {
            let _ = result.recv_blocking();
        }
    }
}

// ----- assets -----

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

/// A cover scaled to [`THUMBNAIL_HEIGHT`], for xs and sm covers and md
/// avatars. Released past [`usage::THUMBNAIL_BUDGET`].
pub enum CoverThumbnail {}

impl Asset for CoverThumbnail {
    type Source = String;
    type Output = Option<Arc<RenderImage>>;

    fn load(url: String, cx: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        let loader = CoverService::loader(cx);
        async move { loader?.scaled(url, THUMBNAIL_HEIGHT).await }
    }
}

/// An md or lg cover or the xl avatar, scaled to [`MEDIUM_COVER_HEIGHT`].
/// Released past [`usage::MEDIUM_BUDGET`], or by a view that no longer shows
/// it ([`release_cover_medium`]).
pub enum CoverMedium {}

impl Asset for CoverMedium {
    type Source = String;
    type Output = Option<Arc<RenderImage>>;

    fn load(url: String, cx: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        let loader = CoverService::loader(cx);
        async move { loader?.scaled(url, MEDIUM_COVER_HEIGHT).await }
    }
}

/// The detail cover, scaled to [`COVER_HEIGHT`]. Released when the pane
/// moves on (see [`release_cover_image`]).
pub enum CoverImage {}

impl Asset for CoverImage {
    type Source = String;
    type Output = Option<Arc<RenderImage>>;

    fn load(url: String, cx: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        let loader = CoverService::loader(cx);
        async move { loader?.scaled(url, COVER_HEIGHT).await }
    }
}

/// The cover at `url` scaled to `height` ([`THUMBNAIL_HEIGHT`],
/// [`MEDIUM_COVER_HEIGHT`] or [`COVER_HEIGHT`]), starting a background load
/// on first use, and stamped as drawn. `None` while it loads, `Some(None)`
/// if it can't be. The calling view re-renders when it finishes.
pub fn load_cover(
    url: &str,
    height: u32,
    window: &mut Window,
    cx: &mut App,
) -> Option<Option<Arc<RenderImage>>> {
    match height {
        THUMBNAIL_HEIGHT => draw::<CoverThumbnail>(url, Kind::Thumbnail, window, cx),
        MEDIUM_COVER_HEIGHT => draw::<CoverMedium>(url, Kind::Medium, window, cx),
        _ => draw::<CoverImage>(url, Kind::Detail, window, cx),
    }
}

/// The cached file for `url`, undecoded, starting a background load on first
/// use: for an image the cover pipeline can't decode (an SVG avatar), which
/// GPUI's `img` draws itself. The calling view re-renders when it finishes.
pub fn cover_file(url: &str, window: &mut Window, cx: &mut App) -> Option<Arc<Path>> {
    draw::<CoverFile>(url, Kind::File, window, cx).flatten()
}

/// The one way covers are drawn: stamps the cover, then asks for its asset.
fn draw<A: Asset<Source = String>>(
    url: &str,
    kind: Kind,
    window: &mut Window,
    cx: &mut App,
) -> Option<A::Output> {
    CoverService::stamp(url, kind, cx);
    window.use_asset::<A>(&url.to_owned(), cx)
}

/// Forgets a loaded cover and drops its texture. `window` is the window being
/// updated, if any: GPUI can't reach it through `cx` then.
fn release<A: Asset<Source = String, Output = Option<Arc<RenderImage>>>>(
    url: &str,
    window: Option<&mut Window>,
    cx: &mut App,
) {
    let url = url.to_owned();
    // Already in GPUI's cache, since a view drew it (the usage map says so):
    // this starts no load.
    let image = cx.fetch_asset::<A>(&url).flatten();
    cx.remove_asset::<A>(&url);
    if let Some(image) = image {
        cx.drop_image(image, window);
    }
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
    if let Some(service) = cx.try_global::<CoverService>() {
        lock(&service.usage).forget(url, Kind::Detail);
    }
    cx.remove_asset::<CoverImage>(&url.to_owned());
    if let Some(image) = image {
        cx.drop_image(image, Some(window));
    }
}

/// Forgets an md or lg cover no view shows any more, and its texture. A view
/// that still shows it loads it again from the saved copy. `window` is the
/// window being updated, if any: GPUI can't reach it through `cx` then.
pub fn release_cover_medium(url: &str, window: Option<&mut Window>, cx: &mut App) {
    let Some(service) = cx.try_global::<CoverService>() else {
        return;
    };
    let drawn = lock(&service.usage).forget(url, Kind::Medium);
    if drawn {
        release::<CoverMedium>(url, window, cx);
    }
}

#[cfg(test)]
mod tests;
