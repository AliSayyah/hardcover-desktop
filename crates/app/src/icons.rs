//! Lucide icons the app uses beyond the kit's default set, and the asset
//! source that serves them with the defaults.
//!
//! The kit bundles only the icons its own components use
//! (`gpui_kit::assets::Assets`). An icon from the rest of the Lucide
//! catalogue is listed here, embedded from `gpui-kit-assets` (no copy of the
//! SVG lives in this repository), and named through
//! `gpui_kit::assets::IconName`. Add one to [`ExtraIcons`]' list and give it
//! a function below (or, for the genre icons, an entry in
//! `chip::genre::GENRES`, whose test checks that each one loads).
//!
//! Lucide is under the ISC licence; its notice ships with `gpui-kit-assets`
//! (`LICENSE-LUCIDE`), the crate the SVGs are embedded from.

use std::borrow::Cow;

use gpui_kit::assets::IconName;
use gpui_kit::{AssetSource, SharedString};

gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        Library,
        Clock,
        Target,
        // Following a series: Follow, Following, Unfollow.
        BookmarkPlus,
        BookmarkCheck,
        BookmarkMinus,
        // The genre icons (`chip::genre`); search, heart and palette are
        // among the kit's defaults.
        Backpack,
        Blocks,
        BookCopy,
        BookImage,
        BookMarked,
        BookText,
        BookUser,
        Brain,
        Briefcase,
        Castle,
        ChefHat,
        Church,
        Coffee,
        Compass,
        FaceGrinning,
        Feather,
        FingerprintPattern,
        FlaskConical,
        Ghost,
        Hourglass,
        Landmark,
        Lightbulb,
        Music,
        Newspaper,
        Orbit,
        Plane,
        Rainbow,
        Rocket,
        ScanEye,
        ScrollText,
        Siren,
        Skull,
        Sparkles,
        Sprout,
        Swords,
        Theater,
        Trophy,
        Vote,
        WandSparkles,
    ]
);

/// The kit's default icons plus [`ExtraIcons`]. Registered in `main`.
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// The Series destination: its sidebar row and its empty state.
pub fn library() -> IconName {
    IconName::Library
}

/// A book not out yet, in a series' strip.
pub fn clock() -> IconName {
    IconName::Clock
}

/// A vibe not built yet: its page's empty state.
pub fn sparkles() -> IconName {
    IconName::Sparkles
}

/// The reading goal: the sidebar's goal row.
pub fn target() -> IconName {
    IconName::Target
}

/// "Follow": a series not followed yet.
pub fn follow() -> IconName {
    IconName::BookmarkPlus
}

/// "Following": the pressed Follow button of a followed series.
pub fn following() -> IconName {
    IconName::BookmarkCheck
}

/// "Unfollow series", in a series row's menu.
pub fn unfollow() -> IconName {
    IconName::BookmarkMinus
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extra_icons_and_the_defaults_both_load() {
        for icon in [
            library(),
            clock(),
            target(),
            sparkles(),
            follow(),
            following(),
            unfollow(),
        ] {
            let bytes = AppAssets.load(&icon.path()).unwrap().unwrap();
            assert!(bytes.starts_with(b"<svg"));
        }
        let default = gpui_kit::component::IconName::Search;
        assert!(
            AppAssets
                .load(&gpui_kit::assets::IconNamed::path(default))
                .unwrap()
                .is_some()
        );
    }
}
