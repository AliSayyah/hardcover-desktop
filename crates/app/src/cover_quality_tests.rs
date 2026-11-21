//! Guards the cover decode pipeline: `app_state::decode_scaled` must keep the
//! size and look of the straightforward implementation below (full decode,
//! `image`'s Triangle resize, RGBA to BGRA).
//!
//! Inputs are generated from a fixed seed; set `COVER_QUALITY_SEED` to try
//! others. Set `COVER_QUALITY_DIR` to also check every image in that
//! directory, for example a copy of `~/.cache/hardcover-desktop/covers`.

use std::path::{Path, PathBuf};

use crate::app_state::decode_scaled;

/// The reference output.
fn reference(path: &Path, max_height: u32) -> Option<(u32, u32, Vec<u8>)> {
    let image = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let height = max_height.min(image.height().max(1));
    let width = u32::try_from(
        (u64::from(image.width()) * u64::from(height) / u64::from(image.height().max(1))).max(1),
    )
    .ok()?;
    let mut rgba = image
        .resize(width, height, image::imageops::FilterType::Triangle)
        .into_rgba8();
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

fn candidate(path: &Path, max_height: u32) -> Option<(u32, u32, Vec<u8>)> {
    let image = decode_scaled(path, max_height)?;
    let size = image.size(0);
    Some((
        u32::try_from(size.width.0).ok()?,
        u32::try_from(size.height.0).ok()?,
        image.as_bytes(0)?.to_vec(),
    ))
}

/// PSNR over the colour channels, in dB.
fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let mut sum = 0f64;
    let mut n = 0f64;
    for (pa, pb) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        for c in 0..3 {
            let d = f64::from(pa[c]) - f64::from(pb[c]);
            sum += d * d;
            n += 1.0;
        }
    }
    let mse = sum / n.max(1.0);
    if mse == 0.0 {
        return 99.0;
    }
    10.0 * (255.0 * 255.0 / mse).log10()
}

/// Small xorshift so the test needs no extra dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn range(&mut self, lo: u32, hi: u32) -> u32 {
        lo + u32::try_from(self.next() % u64::from(hi - lo)).unwrap()
    }

    fn color(&mut self) -> [u8; 3] {
        std::array::from_fn(|_| u8::try_from(self.range(0, 255)).unwrap())
    }
}

/// A cover-like picture: gradients, blocks of colour, and some noise.
fn synthetic(rng: &mut Rng, width: u32, height: u32) -> image::RgbImage {
    let [r0, g0, b0] = rng.color();
    let blocks: Vec<(u32, u32, u32, u32, [u8; 3])> = (0..12)
        .map(|_| {
            let x = rng.range(0, width);
            let y = rng.range(0, height);
            (
                x,
                y,
                rng.range(1, width / 2 + 2),
                rng.range(1, height / 2 + 2),
                rng.color(),
            )
        })
        .collect();
    let mut noise = Rng(rng.next() | 1);
    image::RgbImage::from_fn(width, height, |x, y| {
        for &(bx, by, bw, bh, c) in &blocks {
            if x >= bx && x < bx + bw && y >= by && y < by + bh {
                let n = (noise.next() % 9) as u8;
                return image::Rgb([
                    c[0].saturating_add(n),
                    c[1].saturating_add(n),
                    c[2].saturating_add(n),
                ]);
            }
        }
        let fraction = |position, extent| {
            // The fixture covers are smaller than 65536 pixels on each side.
            f32::from(u16::try_from(position).unwrap()) / f32::from(u16::try_from(extent).unwrap())
        };
        let fx = fraction(x, width);
        let fy = fraction(y, height);
        #[expect(
            clippy::cast_sign_loss,
            clippy::cast_possible_truncation,
            reason = "the gradient is quantized to an 8-bit color channel"
        )]
        let pixel = image::Rgb([
            (f32::from(r0) * (1.0 - fx) + 255.0 * fx * fy) as u8,
            (f32::from(g0) * (1.0 - fy) + 128.0 * fx) as u8,
            (f32::from(b0) * fy + 64.0 * (1.0 - fx)) as u8,
        ]);
        pixel
    })
}

fn check(path: &Path, max_height: u32, failures: &mut Vec<String>) -> bool {
    let Some((rw, rh, rpx)) = reference(path, max_height) else {
        return false;
    };
    let Some((cw, ch, cpx)) = candidate(path, max_height) else {
        failures.push(format!("{}: decode failed at {max_height}", path.display()));
        return true;
    };
    if (cw, ch) != (rw, rh) {
        failures.push(format!(
            "{}: {cw}x{ch}, expected {rw}x{rh} at {max_height}",
            path.display()
        ));
        return true;
    }
    let db = psnr(&rpx, &cpx);
    if db < 30.0 {
        failures.push(format!(
            "{}: PSNR {db:.1} dB at {max_height}",
            path.display()
        ));
    }
    true
}

#[test]
fn cover_quality_matches_reference() {
    let seed = std::env::var("COVER_QUALITY_SEED")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0x5eed_c0de)
        | 1;
    eprintln!("cover_quality seed {seed}");
    let mut rng = Rng(seed);
    let dir = tempfile::tempdir().unwrap();
    let mut paths: Vec<PathBuf> = Vec::new();
    let sizes = [
        (60, 90),
        (300, 450),
        (333, 500),
        (500, 750),
        (800, 1200),
        (700, 1050),
        (900, 1350),
        (720, 1080),
        (640, 480),
        (97, 151),
    ];
    for (i, &(w, h)) in sizes.iter().enumerate() {
        let w = w + rng.range(0, 7);
        let h = h + rng.range(0, 7);
        let img = synthetic(&mut rng, w, h);
        let ext = ["jpg", "png", "jpeg"][i % 3];
        let path = dir.path().join(format!("c{i}.{ext}"));
        img.save(&path).unwrap();
        paths.push(path);
    }
    // The four-channel path: a PNG with transparency, and a grayscale JPEG.
    let mut rgba = image::DynamicImage::ImageRgb8(synthetic(&mut rng, 410, 615)).into_rgba8();
    for (x, _, pixel) in rgba.enumerate_pixels_mut() {
        pixel[3] = u8::try_from(x * 255 / 410).unwrap();
    }
    let path = dir.path().join("alpha.png");
    rgba.save(&path).unwrap();
    paths.push(path);
    let gray = image::DynamicImage::ImageRgb8(synthetic(&mut rng, 520, 780)).into_luma8();
    let path = dir.path().join("gray.jpg");
    gray.save(&path).unwrap();
    paths.push(path);

    if let Ok(real) = std::env::var("COVER_QUALITY_DIR") {
        let mut real: Vec<PathBuf> = std::fs::read_dir(real)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file())
            .collect();
        real.sort();
        paths.extend(real);
    }
    let mut failures = Vec::new();
    let mut checked = 0;
    for path in &paths {
        for h in [144, 480] {
            if check(path, h, &mut failures) {
                checked += 1;
            }
        }
    }
    eprintln!("cover_quality checked {checked} decodes");
    assert!(checked >= 20, "only {checked} decodes checked");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
