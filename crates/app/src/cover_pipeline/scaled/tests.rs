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

/// `path` decoded and scaled, as the decoder keeps it before saving.
fn scaled_source(path: &Path, max_height: u32) -> Source {
    scale(&decode_source(path).ok().unwrap(), max_height).unwrap()
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

/// [`Decoding::load`] with a fresh decoder.
fn load(cover: &Path, max_height: u32) -> Option<Arc<RenderImage>> {
    match Decoding::new(None).load(cover, max_height) {
        Loaded::Image(image) => Some(image),
        Loaded::NoSource | Loaded::Failed | Loaded::Unsupported => None,
    }
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
fn scaled_size_keeps_the_old_rounding_and_never_enlarges() {
    assert_eq!(scaled_size(300, 450, 144), (96, 144));
    // `DynamicImage::resize` fitted 99x143.7 inside 99x144.
    assert_eq!(scaled_size(1000, 1449, 144), (99, 143));
    assert_eq!(scaled_size(300, 450, 480), (300, 450));
    assert_eq!(scaled_size(1, 5000, 144), (1, 144));
    // A landscape picture is still scaled by its height.
    assert_eq!(scaled_size(640, 480, 144), (192, 144));
}

#[test]
fn scaled_size_caps_the_width_too() {
    // Scaled by height alone this was 57600 x 144: a 33 MB thumbnail.
    assert_eq!(scaled_size(60_000, 150, 144), (288, 1));
    assert_eq!(scaled_size(3000, 1000, 144), (288, 96));
    assert_eq!(scaled_size(200, 50, 144), (200, 50), "never enlarged");
}

#[test]
fn images_past_the_limits_are_refused_from_their_header() {
    assert!(within_limits(3000, 4500));
    assert!(!within_limits(MAX_SIDE + 1, 10), "too wide");
    assert!(!within_limits(7000, 7000), "too many pixels");

    // A PNG whose header claims 100000 x 100000: refused, not decoded.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huge.png");
    std::fs::write(&path, png_header(100_000, 100_000)).unwrap();
    assert!(matches!(
        decode_source(&path),
        Err(SourceError::Undecodable)
    ));
}

/// A PNG signature and an IHDR chunk claiming `width` x `height`, RGB.
fn png_header(width: u32, height: u32) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let mut chunk = b"IHDR".to_vec();
    chunk.extend(width.to_be_bytes());
    chunk.extend(height.to_be_bytes());
    chunk.extend([8, 2, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(13u32.to_be_bytes());
    png.extend(&chunk);
    png.extend(crc32(&chunk).to_be_bytes());
    png
}

#[test]
fn opaque_copies_are_saved_as_jpeg_and_reused() {
    let dir = tempfile::tempdir().unwrap();
    let cover = dir.path().join("cover.png");
    sample(&cover, 300, 450, false);

    let decoded = load(&cover, THUMBNAIL_HEIGHT).unwrap();
    let saved = scaled_path(&cover, THUMBNAIL_HEIGHT, "jpg").unwrap();
    assert!(saved.is_file());
    assert_eq!(
        saved.parent().unwrap(),
        dir.path().join(SCALED_DIR).join(SCALED_VERSION)
    );

    let reused = read_scaled(&cover, THUMBNAIL_HEIGHT).unwrap();
    let (w, h, original) = pixels(&decoded);
    let (rw, rh, again) = pixels(&reused);
    assert_eq!((rw, rh), (w, h));
    // Same channel order as the decode, within JPEG's loss.
    assert!(max_channel_diff(&original, &again) < 24);
    // The thumbnail, the medium and the detail copy, and no temporary
    // files.
    assert_eq!(
        std::fs::read_dir(saved.parent().unwrap()).unwrap().count(),
        3
    );
}

#[test]
fn transparent_copies_stay_lossless() {
    let dir = tempfile::tempdir().unwrap();
    let cover = dir.path().join("cover.png");
    sample(&cover, 600, 900, true);

    for height in SCALED_HEIGHTS {
        let decoded = load(&cover, height).unwrap();
        assert!(scaled_path(&cover, height, "qoi").unwrap().is_file());
        let reused = read_scaled(&cover, height).unwrap();
        assert_eq!(pixels(&reused), pixels(&decoded));
    }
}

#[test]
fn a_download_is_decoded_once_into_every_size_then_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let cover = dir.path().join("cover.png");
    sample(&cover, 600, 900, false);
    let thumbnail = load(&cover, THUMBNAIL_HEIGHT).unwrap();
    assert!(!cover.exists(), "the download is deleted");
    assert!(has_every_copy(&cover));

    // Every size comes from the copies now, within JPEG's loss.
    let reused = pixels(&load(&cover, THUMBNAIL_HEIGHT).unwrap());
    assert!(max_channel_diff(&reused.2, &pixels(&thumbnail).2) < 24);
    let detail = load(&cover, COVER_HEIGHT).unwrap();
    assert_eq!(pixels(&detail).0, 400);
    assert_eq!(pixels(&detail).1, 600);
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
        write_scaled(&cover, height, &scaled_source(&source, height)).unwrap();
    }

    let medium = load(&cover, MEDIUM_COVER_HEIGHT).unwrap();
    assert_eq!((pixels(&medium).0, pixels(&medium).1), (192, 288));
    assert!(
        has_scaled(&cover, MEDIUM_COVER_HEIGHT),
        "saved for next time"
    );
    // Close to scaling the original: the detail copy is a JPEG.
    let direct = decode_scaled(&source, MEDIUM_COVER_HEIGHT).unwrap();
    let worst = max_channel_diff(&pixels(&medium).2, &pixels(&direct).2);
    assert!(worst <= 8, "off by {worst}");
    assert!(read_scaled(&cover, THUMBNAIL_HEIGHT).is_some());
}

#[test]
fn a_transparent_cover_made_from_a_larger_copy_keeps_its_colours() {
    let dir = tempfile::tempdir().unwrap();
    let cover = dir.path().join("cover.png");
    let source = dir.path().join("source.png");
    // Red on the left, blue on the right, a transparent corner: its copies
    // are QOI, which store BGRA.
    let image = image::RgbaImage::from_fn(600, 900, |x, y| {
        let alpha = if x < 10 && y < 10 { 0 } else { 255 };
        if x < 300 {
            image::Rgba([220, 20, 30, alpha])
        } else {
            image::Rgba([20, 30, 220, alpha])
        }
    });
    image.save(&source).unwrap();
    write_scaled(&cover, COVER_HEIGHT, &scaled_source(&source, COVER_HEIGHT)).unwrap();

    let made = pixels(&load(&cover, MEDIUM_COVER_HEIGHT).unwrap());
    let direct = pixels(&decode_scaled(&source, MEDIUM_COVER_HEIGHT).unwrap());
    assert_eq!((made.0, made.1), (direct.0, direct.1));
    // Away from the edge between the halves, which scales differently.
    let at = |image: &(u32, u32, Vec<u8>), x: u32| {
        let i = ((image.1 / 2 * image.0 + x) * 4) as usize;
        image.2[i..i + 4].to_vec()
    };
    for x in [made.0 / 4, made.0 * 3 / 4] {
        assert!(
            max_channel_diff(&at(&made, x), &at(&direct, x)) <= 2,
            "red and blue swapped"
        );
    }
}

#[test]
fn a_copy_wider_than_the_cap_is_not_used() {
    let dir = tempfile::tempdir().unwrap();
    let cover = dir.path().join("cover.jpg");
    // As a build before the width cap saved a banner's thumbnail.
    let wide = Source {
        width: 1000,
        height: 144,
        layout: Layout::Rgb,
        pixels: vec![128; 1000 * 144 * 3],
    };
    write_scaled(&cover, THUMBNAIL_HEIGHT, &wide).unwrap();
    assert!(read_scaled(&cover, THUMBNAIL_HEIGHT).is_none());
}

#[test]
fn nothing_on_disk_asks_for_a_download() {
    let dir = tempfile::tempdir().unwrap();
    let cover = dir.path().join("cover.jpg");
    assert!(matches!(
        Decoding::new(None).load(&cover, THUMBNAIL_HEIGHT),
        Loaded::NoSource
    ));
}

#[test]
fn a_broken_download_is_deleted_once_per_run() {
    let dir = tempfile::tempdir().unwrap();
    let cover = dir.path().join("cover.jpg");
    let mut decoding = Decoding::new(None);
    std::fs::write(&cover, b"not an image").unwrap();
    assert!(matches!(
        decoding.load(&cover, THUMBNAIL_HEIGHT),
        Loaded::Failed
    ));
    assert!(!cover.exists(), "deleted, so the retry downloads it again");

    // Downloaded again and still broken: kept, so it isn't downloaded
    // over and over, and not worth trying again this run.
    std::fs::write(&cover, b"not an image").unwrap();
    assert!(matches!(
        decoding.load(&cover, THUMBNAIL_HEIGHT),
        Loaded::Unsupported
    ));
    assert!(cover.exists());
}

#[test]
fn copies_are_marked_as_used_at_most_daily() {
    let dir = tempfile::tempdir().unwrap();
    let cover = dir.path().join("cover.png");
    sample(&cover, 300, 450, false);
    load(&cover, THUMBNAIL_HEIGHT).unwrap();
    let copy = scaled_path(&cover, THUMBNAIL_HEIGHT, "jpg").unwrap();
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
    write_scaled(
        &kept_before,
        THUMBNAIL_HEIGHT,
        &scaled_source(&kept_before, THUMBNAIL_HEIGHT),
    )
    .unwrap();
    set_age(&kept_before, old);
    // One with every copy (a crash before it was deleted), a recent one
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

    // Converting saves the copies it lacked, keeps the thumbnail as it
    // was, and deletes the download.
    let thumbnail = pixels(&read_scaled(&kept_before, THUMBNAIL_HEIGHT).unwrap());
    Decoding::new(None).convert(&kept_before);
    assert!(!kept_before.exists());
    assert!(has_every_copy(&kept_before));
    let reused = read_scaled(&kept_before, THUMBNAIL_HEIGHT).unwrap();
    assert_eq!(pixels(&reused), thumbnail);
}

#[test]
fn a_download_that_already_has_its_detail_copy_is_converted_too() {
    let dir = tempfile::tempdir().unwrap();
    let download = dir.path().join("cover.png");
    sample(&download, 600, 900, false);
    // An earlier build kept it beside its 144 and 480 copies; the 288 one
    // came later. Decoding at 480 used to read the 480 copy and stop there.
    for height in [THUMBNAIL_HEIGHT, COVER_HEIGHT] {
        write_scaled(&download, height, &scaled_source(&download, height)).unwrap();
    }
    set_age(&download, Duration::from_hours(2));

    let dir_path = dir.path().to_owned();
    let mut decoding = Decoding::new(None);
    tidy_cover_dir(&dir_path, |job| match job {
        TidyJob::Convert(path) => decoding.convert(&path),
        TidyJob::Rebuild { cover, older } => decoding.rebuild(&cover, &older),
    });
    assert!(has_scaled(&download, MEDIUM_COVER_HEIGHT));
    assert!(!download.exists(), "the download is deleted");
}

/// Moves `cover`'s current copies into `scaled/<version>/`, as if an older
/// build had saved them.
fn age_copies(cover: &Path, version: &str) {
    let scaled = cover.parent().unwrap().join(SCALED_DIR);
    std::fs::create_dir_all(scaled.join(version)).unwrap();
    for entry in std::fs::read_dir(scaled.join(SCALED_VERSION)).unwrap() {
        let path = entry.unwrap().path();
        std::fs::rename(&path, scaled.join(version).join(path.file_name().unwrap())).unwrap();
    }
}

#[test]
fn after_a_version_bump_covers_are_rebuilt_from_the_older_copies() {
    let dir = tempfile::tempdir().unwrap();
    let cover = dir.path().join("cover.jpg");
    let source = dir.path().join("source.png");
    sample(&source, 600, 900, false);
    for height in SCALED_HEIGHTS {
        write_scaled(&cover, height, &scaled_source(&source, height)).unwrap();
    }
    age_copies(&cover, "v0");

    // Shown before the tidy gets to it: made from the older copy, with no
    // download.
    let thumbnail = load(&cover, THUMBNAIL_HEIGHT).unwrap();
    assert_eq!((pixels(&thumbnail).0, pixels(&thumbnail).1), (96, 144));
    assert!(has_scaled(&cover, THUMBNAIL_HEIGHT));

    // The tidy makes the rest and deletes the older version.
    let dir_path = dir.path().to_owned();
    let mut decoding = Decoding::new(None);
    let mut jobs = 0;
    tidy_cover_dir(&dir_path, |job| {
        jobs += 1;
        match job {
            TidyJob::Convert(path) => decoding.convert(&path),
            TidyJob::Rebuild { cover, older } => decoding.rebuild(&cover, &older),
        }
    });
    assert_eq!(jobs, 1);
    assert!(has_every_copy(&cover));
    assert!(!dir.path().join(SCALED_DIR).join("v0").exists());
    let detail = pixels(&read_scaled(&cover, COVER_HEIGHT).unwrap());
    let direct = pixels(&decode_scaled(&source, COVER_HEIGHT).unwrap());
    assert!(max_channel_diff(&detail.2, &direct.2) <= 12);
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
fn the_size_limit_holds_during_a_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut decoding = Decoding::new(Some(dir.path().to_owned()));
    // Tiny, so a few covers pass it: one cover's copies take some 26 KB.
    (decoding.limit, decoding.target, decoding.check_every) = (70_000, 50_000, 1);
    let covers: Vec<PathBuf> = (0..4)
        .map(|i| dir.path().join(format!("cover-{i}.png")))
        .collect();
    for (i, cover) in covers.iter().enumerate() {
        // Downloaded just now, as each one is shown.
        sample(cover, 600, 900, false);
        assert!(matches!(
            decoding.load(cover, THUMBNAIL_HEIGHT),
            Loaded::Image(_)
        ));
        // Older covers were used longer ago.
        for height in SCALED_HEIGHTS {
            if let Some(copy) = scaled_path(cover, height, "jpg").filter(|p| p.is_file()) {
                set_age(&copy, Duration::from_hours(10 - i as u64));
            }
        }
    }
    let total: u64 = std::fs::read_dir(dir.path().join(SCALED_DIR).join(SCALED_VERSION))
        .unwrap()
        .map(|e| e.unwrap().metadata().unwrap().len())
        .sum();
    assert!(total <= 70_000, "{total} bytes kept");
    assert!(!has_scaled(&covers[0], COVER_HEIGHT), "the oldest went");
    assert!(has_every_copy(&covers[3]), "the newest stayed");
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
    // Nor to make a smaller size from.
    std::fs::remove_file(scaled_path(&cover, MEDIUM_COVER_HEIGHT, "jpg").unwrap()).unwrap();
    std::fs::remove_file(scaled_path(&cover, THUMBNAIL_HEIGHT, "jpg").unwrap()).unwrap();
    assert!(matches!(
        Decoding::new(None).load(&cover, THUMBNAIL_HEIGHT),
        Loaded::NoSource
    ));
}

#[test]
fn stale_temp_files_and_loose_files_are_removed_and_older_versions_kept() {
    let dir = tempfile::tempdir().unwrap();
    let scaled = dir.path().join(SCALED_DIR);
    let old = scaled.join("v0");
    let current = scaled.join(SCALED_VERSION);
    std::fs::create_dir_all(&old).unwrap();
    std::fs::create_dir_all(&current).unwrap();
    std::fs::write(old.join("a-144.qoi"), b"x").unwrap();
    // An earlier layout's copy, loose in `scaled/`.
    let loose = scaled.join("a.jpg.144.qoi");
    let copy = current.join("a-144.qoi");
    let fresh_tmp = current.join("b-144.qoi.1.tmp");
    let stale_tmp = current.join("c-144.qoi.2.tmp");
    for path in [&loose, &copy, &fresh_tmp, &stale_tmp] {
        std::fs::write(path, b"x").unwrap();
    }
    set_age(&stale_tmp, STALE_TEMP_AGE * 2);

    remove_stale_scaled(dir.path());

    assert!(old.exists(), "kept until rebuilt from");
    assert!(!loose.exists());
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
    assert!(!saved.exists(), "replaced by the new copy");
    let reused = read_scaled(&cover, THUMBNAIL_HEIGHT).unwrap();
    assert!(max_channel_diff(&pixels(&reused).2, &pixels(&decoded).2) < 24);
}

/// Saves `source` scaled to `height` as an older version's copy at that
/// height, as v1 saved its 480 px detail copy.
fn older_copy(cover: &Path, version: &str, source: &Path, height: u32) {
    let copy = scaled_source(source, height);
    write_scaled(cover, height, &copy).unwrap();
    let scaled = cover.parent().unwrap().join(SCALED_DIR);
    std::fs::create_dir_all(scaled.join(version)).unwrap();
    let name = cover.file_name().unwrap().to_str().unwrap();
    std::fs::rename(
        scaled_path(cover, height, "jpg").unwrap(),
        scaled.join(version).join(format!("{name}-{height}.jpg")),
    )
    .unwrap();
}

#[test]
fn an_older_copy_is_never_enlarged_into_a_larger_size() {
    let dir = tempfile::tempdir().unwrap();
    let cover = dir.path().join("cover.jpg");
    let source = dir.path().join("source.png");
    sample(&source, 600, 900, false);
    // v1 saved 144, 288 and 480; this version's detail copy is 600.
    for height in [THUMBNAIL_HEIGHT, MEDIUM_COVER_HEIGHT, 480] {
        older_copy(&cover, "v1", &source, height);
    }

    // The detail copy needs the download: the 480 one would be enlarged.
    assert!(matches!(
        Decoding::new(None).load(&cover, COVER_HEIGHT),
        Loaded::NoSource
    ));
    assert!(!has_scaled(&cover, COVER_HEIGHT));

    // The tidy makes the smaller sizes from it, then drops v1.
    let dir_path = dir.path().to_owned();
    let mut decoding = Decoding::new(None);
    tidy_cover_dir(&dir_path, |job| match job {
        TidyJob::Convert(path) => decoding.convert(&path),
        TidyJob::Rebuild { cover, older } => decoding.rebuild(&cover, &older),
    });
    assert!(has_scaled(&cover, THUMBNAIL_HEIGHT));
    assert!(has_scaled(&cover, MEDIUM_COVER_HEIGHT));
    assert!(!has_scaled(&cover, COVER_HEIGHT));
    assert!(!dir.path().join(SCALED_DIR).join("v1").exists());
}

#[test]
fn an_older_copy_of_a_small_cover_makes_every_size() {
    let dir = tempfile::tempdir().unwrap();
    let cover = dir.path().join("cover.jpg");
    let source = dir.path().join("source.png");
    // Smaller than 480: its v1 detail copy is the whole original.
    sample(&source, 300, 450, false);
    older_copy(&cover, "v1", &source, 480);

    let detail = pixels(&load(&cover, COVER_HEIGHT).unwrap());
    assert_eq!((detail.0, detail.1), (300, 450));
}
