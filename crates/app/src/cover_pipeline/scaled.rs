//! Decoding covers and keeping their scaled copies. Everything here blocks,
//! and runs on the cover-decoder thread (see [`super::Decoder`]), apart from
//! the startup tidy's directory walks.
//!
//! A download is decoded once into every size in [`SCALED_HEIGHTS`], saved in
//! `covers/scaled/<SCALED_VERSION>/`, then deleted: the copies are the cache.
//! Copies are stored the same way in every version (QOI of the BGRA pixels,
//! or JPEG of the RGB ones), so a newer version can be rebuilt from an older
//! one's copies instead of downloading every cover again.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use fast_image_resize::{self as fir, PixelType};
use gpui_kit::RenderImage;
use image::ImageDecoder as _;

/// Height of decoded xs and sm covers (list rows, series strips, md
/// avatars), in image pixels. An sm cover is 48 px tall, so this is sharp up
/// to 3x scale.
pub(crate) const THUMBNAIL_HEIGHT: u32 = 144;
/// Height of decoded md and lg covers (Home's rows and shelves, similar
/// books) and of the author page's photo, in image pixels. An lg cover is
/// 120 px tall, so this is sharp up to 2.4x scale, and an md one (96 px) up
/// to 3x.
pub(crate) const MEDIUM_COVER_HEIGHT: u32 = 288;
/// Height of the decoded detail cover, in image pixels. The full book page's
/// xxl cover is 240 design px tall, so this is sharp up to 2.5 device px per
/// design px: a desktop scale of 2 at the interface scale of 1.25 (`ui_scale`,
/// a 27" 5K monitor).
pub(crate) const COVER_HEIGHT: u32 = 600;
/// Every height a cover is scaled to. A download is decoded once into all of
/// them, and then deleted: the scaled copies are the cache.
pub(super) const SCALED_HEIGHTS: [u32; 3] = [THUMBNAIL_HEIGHT, MEDIUM_COVER_HEIGHT, COVER_HEIGHT];
/// A scaled copy is at most this many times as wide as it is tall at most:
/// a cover box is 2:3, so only a freak image (a 60000×150 banner) reaches
/// it, which would otherwise make a copy the GPU atlas refuses.
const MAX_ASPECT: u32 = 2;

/// The largest image decoded, on either side. A cover is a few thousand
/// pixels tall at most.
const MAX_SIDE: u32 = 12_000;
/// The most pixels decoded: 32 million, 128 MB as RGBA. Checked from the
/// header before decoding, so one huge image can't stall the decoder or
/// take a few hundred megabytes.
const MAX_PIXELS: u64 = 32_000_000;
/// What a decoder may allocate, for the formats that honour it.
const MAX_ALLOC: u64 = 192 * 1024 * 1024;

/// Scaled copies of covers live in `scaled/<version>/` under the cover cache.
const SCALED_DIR: &str = "scaled";
/// Bump whenever [`scale`] changes its output. Copies made by an older
/// version are rebuilt from (the cover is decoded from the largest older
/// copy), then deleted, so a bump downloads nothing again, except for a size
/// larger than any older copy (an older copy is never enlarged). Don't bump
/// for a change to how copies are stored: rebuilding assumes the same
/// encoding. v2: the detail copy went from 480 to 600 px ([`COVER_HEIGHT`]).
const SCALED_VERSION: &str = "v2";
/// A temporary file older than this was left by a write that never finished.
const STALE_TEMP_AGE: Duration = Duration::from_hours(1);
/// JPEG quality of saved copies of opaque covers, thumbnails included:
/// on real covers a 144 px JPEG took 8.9 KB against 30.1 KB as QOI, and
/// `cover_quality_tests` checks it stays visually lossless.
pub(crate) const SCALED_JPEG_QUALITY: u8 = 92;
/// The most disk the cover cache may take. Past it, the copies used least
/// recently are deleted, down to [`COVER_CACHE_TARGET`]: at startup and
/// after every [`CHECK_SIZE_EVERY`] bytes of copies saved. A cover takes
/// about 120 KB at all three sizes, so this holds some 2,200 covers.
const COVER_CACHE_LIMIT: u64 = 256 * 1024 * 1024;
const COVER_CACHE_TARGET: u64 = COVER_CACHE_LIMIT / 4 * 3;
/// The size limit is checked again after this many bytes of copies are
/// saved, so a long run can't grow the cache past the limit by more.
const CHECK_SIZE_EVERY: u64 = 16 * 1024 * 1024;
/// A copy is marked as used (its modification time set) at most this often,
/// so the size limit deletes the copies used least recently.
const TOUCH_AFTER: Duration = Duration::from_hours(24);

/// What the decoder made of a cover.
pub(super) enum Loaded {
    Image(Arc<RenderImage>),
    /// Neither a scaled copy nor the download is on disk.
    NoSource,
    /// Something went wrong that a later try may not hit (a download that
    /// didn't decode is deleted, so the next try downloads it again).
    Failed,
    /// The download is not an image this build decodes, again: trying again
    /// won't help.
    Unsupported,
}

/// The cover decoder's own state: what it did this run, and the size limit.
pub(super) struct Decoding {
    /// Downloads deleted this run because they didn't decode.
    deleted: HashSet<PathBuf>,
    /// The cover cache's directory, for the size limit.
    cover_dir: Option<PathBuf>,
    /// Bytes of copies saved since the size limit was last checked.
    written: u64,
    limit: u64,
    target: u64,
    check_every: u64,
}

impl Decoding {
    pub(super) fn new(cover_dir: Option<PathBuf>) -> Self {
        Self {
            deleted: HashSet::new(),
            cover_dir,
            written: 0,
            limit: COVER_CACHE_LIMIT,
            target: COVER_CACHE_TARGET,
            check_every: CHECK_SIZE_EVERY,
        }
    }

    /// The cover scaled to `max_height`, from its saved copy when there is
    /// one.
    ///
    /// Otherwise the download at `path` is decoded once and saved at every
    /// size in [`SCALED_HEIGHTS`], then deleted: a relaunch shows covers
    /// without decoding any, and the cache keeps a few small copies instead
    /// of a large original (most downloads are over 1,000 pixels tall).
    /// Should saving fail, the download stays and the next run decodes it
    /// again.
    ///
    /// A download that doesn't decode is deleted, so the next try downloads
    /// it again (it may have been cut short), but once per run at most: the
    /// second time it is kept, and [`Loaded::Unsupported`], so an image this
    /// build can't read isn't downloaded over and over.
    pub(super) fn load(&mut self, path: &Path, max_height: u32) -> Loaded {
        if let Some(image) = read_scaled(path, max_height) {
            return Loaded::Image(image);
        }
        match decode_source(path) {
            Ok(source) => match self.save_all(path, &source, Some(max_height)) {
                Some(image) => Loaded::Image(image),
                None => Loaded::Failed,
            },
            Err(SourceError::Missing) => self.scale_from_larger_copy(path, max_height),
            Err(SourceError::Unreadable) => Loaded::Failed,
            Err(SourceError::Undecodable) => self.undecodable(path),
        }
    }

    /// Converts a download kept beside some of its copies (as earlier builds
    /// left them): decodes the download itself, not a copy, saves the sizes
    /// it lacks, and deletes it.
    pub(super) fn convert(&mut self, path: &Path) {
        match decode_source(path) {
            Ok(source) => {
                self.save_all(path, &source, None);
            }
            Err(SourceError::Undecodable) => {
                self.undecodable(path);
            }
            Err(SourceError::Missing | SourceError::Unreadable) => {}
        }
    }

    /// Makes the copies of `cover` this version lacks from the first of
    /// `older` (an older version's copies as `(height, path)`, largest
    /// first) that reads. A copy scaled to its full height can't make a
    /// larger one; that size is downloaded again when shown.
    pub(super) fn rebuild(&mut self, cover: &Path, older: &[(u32, PathBuf)]) {
        let Some((source, reach)) = older
            .iter()
            .find_map(|(height, copy)| Some((read_copy(copy)?, *height)))
        else {
            return;
        };
        // Shorter than its height: the whole original, good for any size.
        let reach = if source.height < reach {
            u32::MAX
        } else {
            reach
        };
        for height in SCALED_HEIGHTS {
            if height <= reach
                && !has_scaled(cover, height)
                && let Some(copy) = scale(&source, height)
            {
                self.save(cover, height, copy);
            }
        }
    }

    /// Saves `source` at every size `cover` lacks, and at `want` (returned
    /// as an image), then deletes the download if every copy was saved.
    fn save_all(
        &mut self,
        cover: &Path,
        source: &Source,
        want: Option<u32>,
    ) -> Option<Arc<RenderImage>> {
        let mut wanted = None;
        let mut saved = true;
        for height in SCALED_HEIGHTS {
            if Some(height) == want {
                let copy = scale(source, height)?;
                saved &= self.save(cover, height, copy.clone());
                wanted = to_render_image(copy);
            } else if !has_scaled(cover, height) {
                saved &= scale(source, height).is_some_and(|copy| self.save(cover, height, copy));
            }
        }
        if saved {
            let _ = std::fs::remove_file(cover);
        }
        wanted
    }

    /// The cover scaled to `max_height` from a larger saved copy, when the
    /// download is gone: a cache made before a size was added has the other
    /// copies only, and after a [`SCALED_VERSION`] bump the older version's
    /// copies are there. The copy is saved as usual. [`Loaded::NoSource`]
    /// when there is none.
    fn scale_from_larger_copy(&mut self, cover: &Path, max_height: u32) -> Loaded {
        let current = SCALED_HEIGHTS
            .into_iter()
            .filter(|&h| h > max_height)
            .flat_map(|h| COPY_FORMATS.map(|(ext, _)| scaled_path(cover, h, ext)))
            .flatten();
        let older = || {
            older_copies(cover).into_iter().find_map(|(height, path)| {
                let copy = read_copy(&path)?;
                // Never enlarged: a copy scaled to its full height makes
                // only smaller sizes; one shorter is the whole original.
                (height >= max_height || copy.height < height).then_some(copy)
            })
        };
        let current = current.into_iter().find_map(|copy| read_copy(&copy));
        let Some(source) = current.or_else(older) else {
            return Loaded::NoSource;
        };
        let Some(copy) = scale(&source, max_height) else {
            return Loaded::Failed;
        };
        self.save(cover, max_height, copy.clone());
        to_render_image(copy).map_or(Loaded::Failed, Loaded::Image)
    }

    fn undecodable(&mut self, path: &Path) -> Loaded {
        if self.deleted.insert(path.to_owned()) {
            let _ = std::fs::remove_file(path);
            Loaded::Failed
        } else {
            Loaded::Unsupported
        }
    }

    /// Saves a scaled copy; false if that failed. Past
    /// [`CHECK_SIZE_EVERY`] bytes saved, checks the size limit.
    fn save(&mut self, cover: &Path, max_height: u32, copy: Source) -> bool {
        match write_scaled(cover, max_height, &copy) {
            Ok(bytes) => {
                self.written += bytes;
                if self.written >= self.check_every {
                    self.written = 0;
                    if let Some(dir) = &self.cover_dir {
                        enforce_size_limit(dir, self.limit, self.target);
                    }
                }
                true
            }
            Err(e) => {
                // The cover still shows; the next run decodes it again. Said
                // once, since the cause (say, a read-only cache) usually
                // affects them all.
                static WARNED: AtomicBool = AtomicBool::new(false);
                if !WARNED.swap(true, Ordering::Relaxed) {
                    eprintln!("hardcover: could not save a scaled cover: {e}");
                }
                false
            }
        }
    }
}

/// How a cover's pixels are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layout {
    Rgb,
    Rgba,
    /// What GPUI draws, and how QOI copies store it.
    Bgra,
}

impl Layout {
    fn pixel_type(self) -> PixelType {
        match self {
            Self::Rgb => PixelType::U8x3,
            Self::Rgba | Self::Bgra => PixelType::U8x4,
        }
    }
}

/// Decoded pixels, at full size or scaled.
#[derive(Clone)]
struct Source {
    width: u32,
    height: u32,
    layout: Layout,
    pixels: Vec<u8>,
}

/// Why a download could not be decoded.
enum SourceError {
    /// It isn't on disk.
    Missing,
    /// It is, but can't be read (permissions, say).
    Unreadable,
    /// It was read, but is not an image this build decodes: a download cut
    /// short, an unsupported format, or larger than [`MAX_PIXELS`].
    Undecodable,
}

fn decode_source(path: &Path) -> Result<Source, SourceError> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                SourceError::Missing
            } else {
                SourceError::Unreadable
            }
        })?
        .with_guessed_format()
        .map_err(|_| SourceError::Unreadable)?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(|_| SourceError::Undecodable)?;
    let (width, height) = decoder.dimensions();
    if !within_limits(width, height) {
        return Err(SourceError::Undecodable);
    }
    let image = image::DynamicImage::from_decoder(decoder).map_err(|_| SourceError::Undecodable)?;
    // Covers are mostly JPEG, which decodes to RGB; resize that without
    // adding an alpha channel first.
    let (layout, pixels) = match image {
        image::DynamicImage::ImageRgb8(rgb) => (Layout::Rgb, rgb.into_raw()),
        other => (Layout::Rgba, other.into_rgba8().into_raw()),
    };
    Ok(Source {
        width,
        height,
        layout,
        pixels,
    })
}

/// True if an image of this size is decoded at all: see [`MAX_SIDE`] and
/// [`MAX_PIXELS`].
fn within_limits(width: u32, height: u32) -> bool {
    width <= MAX_SIDE && height <= MAX_SIDE && u64::from(width) * u64::from(height) <= MAX_PIXELS
}

/// `source` scaled down to fit [`scaled_size`]'s box for `max_height`, in
/// the same layout.
fn scale(source: &Source, max_height: u32) -> Option<Source> {
    let (width, height) = scaled_size(source.width, source.height, max_height);
    if (width, height) == (source.width, source.height) {
        return Some(source.clone());
    }
    let pixel_type = source.layout.pixel_type();
    let src =
        fir::images::ImageRef::new(source.width, source.height, &source.pixels, pixel_type).ok()?;
    let mut dst = fir::images::Image::new(width, height, pixel_type);
    // Bilinear convolution is the Triangle filter `image` uses, with SIMD.
    let options = fir::ResizeOptions::new()
        .resize_alg(fir::ResizeAlg::Convolution(fir::FilterType::Bilinear))
        .use_alpha(false);
    fir::Resizer::new().resize(&src, &mut dst, &options).ok()?;
    Some(Source {
        width,
        height,
        layout: source.layout,
        pixels: dst.into_vec(),
    })
}

/// The pixels as GPUI draws them: BGRA.
fn into_bgra(source: Source) -> Vec<u8> {
    match source.layout {
        Layout::Rgb => source
            .pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[2], p[1], p[0], u8::MAX])
            .collect(),
        Layout::Rgba => {
            let mut pixels = source.pixels;
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            pixels
        }
        Layout::Bgra => source.pixels,
    }
}

fn to_render_image(source: Source) -> Option<Arc<RenderImage>> {
    let (width, height) = (source.width, source.height);
    let frame = image::RgbaImage::from_raw(width, height, into_bgra(source))?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(frame)])))
}

/// Decodes an image file, scaled down to at most `max_height` pixels tall.
#[cfg(test)]
pub(crate) fn decode_scaled(path: &Path, max_height: u32) -> Option<Arc<RenderImage>> {
    to_render_image(scale(&decode_source(path).ok()?, max_height)?)
}

/// The size a `src_width` x `src_height` cover is shown at: at most
/// `max_height` tall and [`MAX_ASPECT`] times that wide, never enlarged.
///
/// Within the width this is the size `DynamicImage::resize` picked when it
/// did the scaling: the width is rounded down, then the result is fitted
/// inside that box, so the height can come out a pixel short. Kept so covers
/// look exactly as before.
fn scaled_size(src_width: u32, src_height: u32, max_height: u32) -> (u32, u32) {
    let max_width = max_height.saturating_mul(MAX_ASPECT);
    let (src_width, src_height) = (src_width.max(1), src_height.max(1));
    let height = max_height.min(src_height);
    let width = u32::try_from(u64::from(src_width) * u64::from(height) / u64::from(src_height))
        .expect("downscaled width fits in the source width")
        .max(1);
    if width > max_width {
        let height = u64::from(src_height) * u64::from(max_width) / u64::from(src_width);
        let height = u32::try_from(height).expect("fits in the source height");
        return (max_width, height.max(1));
    }
    let ratio = f64::min(
        f64::from(width) / f64::from(src_width),
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

// ----- the copies on disk -----

/// The formats copies are saved in, by extension: QOI of BGRA pixels, JPEG
/// of RGB ones.
const COPY_FORMATS: [(&str, image::ImageFormat); 2] = [
    ("qoi", image::ImageFormat::Qoi),
    ("jpg", image::ImageFormat::Jpeg),
];

/// `covers/scaled/<SCALED_VERSION>/<cover file>-<height>.<ext>`.
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

/// True if a copy of `cover` scaled to `max_height` is on disk.
pub(super) fn has_scaled(cover: &Path, max_height: u32) -> bool {
    COPY_FORMATS
        .into_iter()
        .any(|(ext, _)| scaled_path(cover, max_height, ext).is_some_and(|p| p.is_file()))
}

/// True if every size of `cover` is on disk, so any size can be shown
/// without its download.
pub(super) fn has_every_copy(cover: &Path) -> bool {
    SCALED_HEIGHTS.iter().all(|&h| has_scaled(cover, h))
}

/// The copy of `cover` at `max_height`, as GPUI draws it. A copy wider than
/// [`scaled_size`] allows (saved before the width was capped) is not used.
fn read_scaled(cover: &Path, max_height: u32) -> Option<Arc<RenderImage>> {
    COPY_FORMATS
        .into_iter()
        .filter_map(|(ext, _)| scaled_path(cover, max_height, ext))
        .find_map(|path| {
            let copy = read_copy(&path)?;
            if copy.width > max_height.saturating_mul(MAX_ASPECT) {
                let _ = std::fs::remove_file(&path);
                return None;
            }
            Some(copy)
        })
        .and_then(to_render_image)
}

/// Reads a saved copy, in the layout it was saved in, and marks it as used.
/// The one reader of copies, for showing one and for scaling from one. A
/// copy that is there but doesn't read is deleted, so a copy on disk is one
/// that can be used: the downloader skips a cover with every copy.
fn read_copy(path: &Path) -> Option<Source> {
    let ext = path.extension()?.to_str()?;
    let (_, format) = COPY_FORMATS.into_iter().find(|(e, _)| *e == ext)?;
    let bytes = std::fs::read(path).ok()?;
    let source = decode_copy(&bytes, format);
    if source.is_some() {
        mark_used(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
    source
}

fn decode_copy(bytes: &[u8], format: image::ImageFormat) -> Option<Source> {
    let source = match format {
        image::ImageFormat::Jpeg => {
            // The JPEG decoder fills in a truncated file (say, one cut short
            // by a crash) instead of failing, so check that it is complete.
            // A short QOI file fails to decode on its own.
            if !bytes.ends_with(&[0xFF, 0xD9]) {
                return None;
            }
            let rgb = image::load_from_memory_with_format(bytes, format)
                .ok()?
                .into_rgb8();
            Source {
                width: rgb.width(),
                height: rgb.height(),
                layout: Layout::Rgb,
                pixels: rgb.into_raw(),
            }
        }
        _ => {
            // Saved as BGRA under QOI's RGBA label.
            let bgra = image::load_from_memory_with_format(bytes, format)
                .ok()?
                .into_rgba8();
            Source {
                width: bgra.width(),
                height: bgra.height(),
                layout: Layout::Bgra,
                pixels: bgra.into_raw(),
            }
        }
    };
    Some(source)
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

/// Saves a scaled cover next to the originals, and returns its size. Opaque
/// covers are high-quality JPEG; a cover with transparency is QOI, lossless,
/// of its BGRA pixels. Synced before it is renamed into place: the copies are
/// the cache, and a crash must not leave half of one.
fn write_scaled(cover: &Path, max_height: u32, copy: &Source) -> std::io::Result<u64> {
    use image::ImageEncoder as _;
    use std::io::Write as _;

    let (width, height) = (copy.width, copy.height);
    let mut encoded = Vec::new();
    let opaque = copy.layout == Layout::Rgb
        || copy
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[3] == u8::MAX);
    let ext = if opaque {
        let rgb: Vec<u8> = match copy.layout {
            Layout::Rgb => copy.pixels.clone(),
            Layout::Rgba => copy
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect(),
            Layout::Bgra => copy
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[2], p[1], p[0]])
                .collect(),
        };
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, SCALED_JPEG_QUALITY)
            .encode(&rgb, width, height, image::ExtendedColorType::Rgb8)
            .map_err(std::io::Error::other)?;
        "jpg"
    } else {
        let bgra = into_bgra(copy.clone());
        image::codecs::qoi::QoiEncoder::new(&mut encoded)
            .write_image(&bgra, width, height, image::ExtendedColorType::Rgba8)
            .map_err(std::io::Error::other)?;
        "qoi"
    };
    let path = scaled_path(cover, max_height, ext)
        .ok_or_else(|| std::io::Error::other("the cover path has no file name"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Written aside and renamed, so a reader never sees half a file. Only
    // the decoder thread writes copies, so the process id is unique enough.
    let mut tmp = path.clone().into_os_string();
    tmp.push(format!(".{}.tmp", std::process::id()));
    let tmp = PathBuf::from(tmp);
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        std::fs::rename(&tmp, &path)?;
        // The other format's copy, if an earlier build saved one, would be
        // read first.
        let other = if ext == "jpg" { "qoi" } else { "jpg" };
        if let Some(other) = scaled_path(cover, max_height, other) {
            let _ = std::fs::remove_file(other);
        }
        Ok(())
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(encoded.len() as u64)
}

// ----- older versions -----

/// The version directories under `scaled/` other than [`SCALED_VERSION`],
/// newest first.
fn older_versions(scaled: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(scaled) else {
        return Vec::new();
    };
    let mut versions: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_name() != SCALED_VERSION && e.path().is_dir())
        .map(|e| {
            let number = e
                .file_name()
                .to_str()
                .and_then(|n| n.strip_prefix('v'))
                .and_then(|n| n.parse().ok())
                .unwrap_or(0);
            (number, e.path())
        })
        .collect();
    versions.sort_by_key(|(number, _)| std::cmp::Reverse(*number));
    versions.into_iter().map(|(_, path)| path).collect()
}

/// Heights earlier versions saved that this one doesn't, looked for when
/// rebuilding from them. Add a height here when one leaves
/// [`SCALED_HEIGHTS`].
const EARLIER_HEIGHTS: [u32; 1] = [480];

/// `cover`'s copies in older versions, as `(height, path)`: the newest
/// version first, the largest copy first within it.
fn older_copies(cover: &Path) -> Vec<(u32, PathBuf)> {
    let (Some(dir), Some(name)) = (cover.parent(), cover.file_name().and_then(|n| n.to_str()))
    else {
        return Vec::new();
    };
    let mut copies = Vec::new();
    for version in older_versions(&dir.join(SCALED_DIR)) {
        let mut found: Vec<(u32, PathBuf)> = SCALED_HEIGHTS
            .into_iter()
            .chain(EARLIER_HEIGHTS)
            .flat_map(|h| COPY_FORMATS.map(|(ext, _)| (h, ext)))
            .map(|(h, ext)| (h, version.join(format!("{name}-{h}.{ext}"))))
            .filter(|(_, path)| path.is_file())
            .collect();
        found.sort_by_key(|(h, _)| std::cmp::Reverse(*h));
        copies.extend(found);
    }
    copies
}

/// What the startup tidy asks of the decoder thread, one at a time.
pub(super) enum TidyJob {
    /// A download kept beside its copies: see [`Decoding::convert`].
    Convert(PathBuf),
    /// A cover with copies in an older version, as `(height, path)`: see
    /// [`Decoding::rebuild`]. The older copies are deleted once it is done.
    Rebuild {
        cover: PathBuf,
        older: Vec<(u32, PathBuf)>,
    },
}

/// Tidies the cover cache at startup. Blocking; errors are ignored, as
/// everything in it can be downloaded again. `run` hands a job to the
/// decoder thread and waits for it, so a cover being shown waits behind one
/// job at most.
pub(super) fn tidy_cover_dir(cover_dir: &Path, mut run: impl FnMut(TidyJob)) {
    remove_stale_scaled(cover_dir);
    for download in sweep_downloads(cover_dir) {
        run(TidyJob::Convert(download));
    }
    let scaled = cover_dir.join(SCALED_DIR);
    for version in older_versions(&scaled) {
        for (cover, older) in older_covers(cover_dir, &version) {
            if !has_every_copy(&cover) {
                run(TidyJob::Rebuild {
                    cover,
                    older: older.clone(),
                });
            }
            for (_, copy) in older {
                let _ = std::fs::remove_file(copy);
            }
        }
        let _ = std::fs::remove_dir_all(version);
    }
    enforce_size_limit(cover_dir, COVER_CACHE_LIMIT, COVER_CACHE_TARGET);
}

/// The covers with copies in the older `version`, each with those copies as
/// `(height, path)`, largest first.
fn older_covers(cover_dir: &Path, version: &Path) -> Vec<(PathBuf, Vec<(u32, PathBuf)>)> {
    let Ok(entries) = std::fs::read_dir(version) else {
        return Vec::new();
    };
    let mut covers: HashMap<String, Vec<(u32, PathBuf)>> = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // `<cover file>-<height>.<ext>`
        let parsed = file.rsplit_once('-').and_then(|(name, rest)| {
            let (height, ext) = rest.split_once('.')?;
            let known = COPY_FORMATS.iter().any(|(e, _)| *e == ext);
            Some((name, height.parse::<u32>().ok().filter(|_| known)?))
        });
        if let Some((name, height)) = parsed {
            covers
                .entry(name.to_owned())
                .or_default()
                .push((height, path));
        }
    }
    let mut covers: Vec<(PathBuf, Vec<(u32, PathBuf)>)> = covers
        .into_iter()
        .map(|(name, mut copies)| {
            copies.sort_by_key(|(h, _)| std::cmp::Reverse(*h));
            (cover_dir.join(name), copies)
        })
        .collect();
    covers.sort();
    covers
}

/// Deletes temporary files left by writes that never finished, and files
/// loose in `scaled/` (an earlier layout). Older versions stay until the
/// tidy has rebuilt from them.
fn remove_stale_scaled(cover_dir: &Path) {
    let scaled = cover_dir.join(SCALED_DIR);
    let Ok(entries) = std::fs::read_dir(&scaled) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::remove_file(entry.path());
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

/// Deletes temporary files left by downloads that never finished, and
/// downloads that have every scaled copy (a crash between saving the copies
/// and deleting it). Returns the other downloads, which earlier builds kept
/// next to their copies, to be converted ([`Decoding::convert`]). SVGs are
/// left alone: they are shown as downloaded, never decoded.
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
        if extension == Some("tmp") || has_every_copy(&path) {
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

#[cfg(test)]
mod tests;
