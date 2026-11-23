//! "For you" and "Top picks" on Discover (`hardcover-design`, Discover):
//! the user's Recommendations and Top Picks as cover shelves, each with
//! "See all 250", which opens the vibe's page. A view of its own, so
//! Discover only places it.
//!
//! - **One lead:** the first section with covers (or loading them) has lg
//!   covers with the cover shadow; the other md covers, flat. As many as
//!   fit the column, in one row.
//! - For you has the muted line "Hardcover's recommendations from your
//!   library." between its title and its shelf.
//! - **Not built yet:** the section keeps its title and place, and shows
//!   one muted sentence and "Open on Hardcover" where the shelf would be.
//! - A book the reader said never to recommend is left out at once.
//! - **States:** skeleton covers while loading; a failure is one muted
//!   sentence with "Try again" (once, under For you, when reading the vibes
//!   failed); a section with nothing to show hides.
//! - Without `read:vibes` the whole block hides.

use std::collections::HashSet;

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, Entity, IntoElement, ParentElement as _, Pixels, Render,
    SharedString, Styled as _, Subscription, Window, div, rems,
};
use hardcover_core::auth::scopes::READ_VIBES;
use hardcover_core::{BookId, BookSummary, Vibe, VibeKind};

use super::model::VibesModel;
use super::vibe_intro;
use crate::catalog::book_list::list_width;
use crate::catalog::navigation::{self, Page};
use crate::cover::{Cover, CoverSize};
use crate::design::{TypeRoles as _, colors};
use crate::preferences::AppPreferences;
use crate::resource::catalog::VibeBooksKey;
use crate::resource::{Catalog, Demand, LoadStatus};
use crate::session;
use crate::ui::{
    CoverShelf, ExternalLink, LoadFailed, SectionHeader, Skeleton, TextButton, books_that_fit,
    region_width, see_all,
};

/// The sections, in order.
const SECTIONS: [VibeKind; 2] = [VibeKind::Recommendations, VibeKind::TopPicks];

/// What a section shows.
enum Section {
    /// Its books, those the reader left out taken off, and how many the
    /// vibe holds (for "See all").
    Covers {
        vibe: Vibe,
        books: Vec<BookSummary>,
        total: usize,
    },
    Loading,
    Failed(SharedString),
    /// Hardcover hasn't built the list.
    NotBuilt(Vibe),
    Hidden,
}

pub struct VibeShelves {
    catalog: Option<Catalog>,
    vibes: Option<Entity<VibesModel>>,
    _subscriptions: Vec<Subscription>,
}

impl VibeShelves {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let catalog = Catalog::global(cx);
        let vibes = VibesModel::global(cx);
        let mut subscriptions = vec![
            cx.observe(&AppPreferences::global(cx).clone(), |_, _, cx| cx.notify()),
            session::observe_grant(cx, |_, cx| cx.notify()),
        ];
        if let Some(vibes) = &vibes {
            // A vibe landed or was built again: its books follow.
            subscriptions.push(cx.observe(vibes, |this, _, cx| {
                this.request_books(Demand::Active, cx);
                cx.notify();
            }));
        }
        if let Some(catalog) = &catalog {
            subscriptions.push(cx.observe(&catalog.vibe_books, |_, _, cx| cx.notify()));
        }
        Self {
            catalog,
            vibes,
            _subscriptions: subscriptions,
        }
    }

    /// Asks for the vibes and their first books as `demand` calls for:
    /// Discover shown (`Shown`), the window active again (`Active`).
    pub fn request(&mut self, demand: Demand, cx: &mut Context<Self>) {
        if let Some(vibes) = &self.vibes {
            vibes.update(cx, |m, cx| m.request_system(demand, cx));
        }
        self.request_books(demand, cx);
    }

    fn request_books(&self, demand: Demand, cx: &mut App) {
        let (Some(vibes), Some(catalog)) = (&self.vibes, &self.catalog) else {
            return;
        };
        let wanted: Vec<(VibeBooksKey, Vec<BookId>)> = SECTIONS
            .iter()
            .filter_map(|kind| vibes.read(cx).system_vibe(*kind))
            .filter(|v| v.is_built())
            .map(|v| (key(v), v.book_ids.clone()))
            .collect();
        for (key, ids) in wanted {
            catalog.request_vibe_books(key, ids, demand, cx);
        }
    }

    fn section(&self, kind: VibeKind, cx: &App) -> Section {
        let (Some(vibes), Some(catalog)) = (&self.vibes, &self.catalog) else {
            return Section::Hidden;
        };
        let model = vibes.read(cx);
        let Some(vibe) = model.system_vibe(kind) else {
            return match model.system_status() {
                LoadStatus::Loading if !model.system_loaded() => Section::Loading,
                // One request reads both: its failure shows once.
                LoadStatus::Failed(message)
                    if !model.system_loaded() && kind == VibeKind::Recommendations =>
                {
                    Section::Failed(message)
                }
                _ => Section::Hidden,
            };
        };
        if !vibe.is_built() {
            return Section::NotBuilt(vibe.clone());
        }
        let shown: Vec<BookId> = VibesModel::shown_books(vibe, cx);
        let store = catalog.vibe_books.read(cx).resource();
        let key = key(vibe);
        match store.value(&key) {
            Some(value) => {
                let shown_set: HashSet<BookId> = shown.iter().copied().collect();
                let books: Vec<BookSummary> = value
                    .books
                    .items
                    .iter()
                    .filter(|b| shown_set.contains(&b.id))
                    .cloned()
                    .collect();
                if books.is_empty() {
                    Section::Hidden
                } else {
                    Section::Covers {
                        vibe: vibe.clone(),
                        books,
                        total: shown.len(),
                    }
                }
            }
            None => match store.error(&key) {
                Some(message) if !store.is_loading(&key) => Section::Failed(message.clone()),
                _ => Section::Loading,
            },
        }
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.request(Demand::Now, cx);
    }

    fn render_section(
        &self,
        kind: VibeKind,
        section: Section,
        lead: bool,
        width: Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let size = if lead { CoverSize::Lg } else { CoverSize::Md };
        let fit = books_that_fit(width, size, window.rem_size()).max(1);
        let title = match kind {
            VibeKind::Recommendations => "For you",
            _ => "Top picks",
        };
        let id = match kind {
            VibeKind::Recommendations => "for-you",
            _ => "top-picks",
        };
        let mut header = SectionHeader::new(title);
        let content: AnyElement = match section {
            Section::Hidden => return None,
            Section::Covers { vibe, books, total } => {
                let vibe_id = vibe.id;
                if total > books.len().min(fit) {
                    header = header.action(see_all(
                        SharedString::from(format!("{id}-see-all")),
                        Some(u32::try_from(total).unwrap_or(u32::MAX)),
                        move |_, _, cx| navigation::open(Page::Vibe(vibe_id), cx),
                    ));
                }
                let shelf = CoverShelf::new(size)
                    .one_row()
                    .books(books.iter().take(fit).map(|book| {
                        let raw = book.id.get().cast_unsigned();
                        (
                            Cover::new(size, book.title.clone())
                                .author(book.authors.first().cloned())
                                .thumbnail(book.cover.as_ref().map(|c| c.url.clone()))
                                .shadow(lead)
                                .opens(
                                    SharedString::from(format!("{id}-cover-{raw}")),
                                    Page::Book(book.clone()),
                                ),
                            TextButton::book(SharedString::from(format!("{id}-title-{raw}")), book),
                        )
                    }));
                match (kind == VibeKind::Recommendations)
                    .then(|| vibe_intro(&vibe))
                    .flatten()
                {
                    Some(intro) => v_flex()
                        .gap_3()
                        .child(muted(intro, cx))
                        .child(shelf)
                        .into_any_element(),
                    None => shelf.into_any_element(),
                }
            }
            Section::Loading => h_flex()
                .gap_4()
                .children((0..fit.min(8)).map(|_| Skeleton::cover(size)))
                .into_any_element(),
            Section::Failed(message) => {
                LoadFailed::new(SharedString::from(format!("{id}-retry")), message)
                    .on_retry(cx.listener(|this, _, _, cx| this.retry(cx)))
                    .into_any_element()
            }
            Section::NotBuilt(vibe) => v_flex()
                .gap_2()
                .items_start()
                .child(muted(
                    "Hardcover builds this list when you open it on hardcover.app.",
                    cx,
                ))
                .child(ExternalLink::new(
                    SharedString::from(format!("{id}-on-hardcover")),
                    "Open on Hardcover",
                    vibe.web_url(),
                ))
                .into_any_element(),
        };
        Some(
            v_flex()
                .gap_3()
                .child(header)
                .child(content)
                .into_any_element(),
        )
    }
}

/// Which page of the catalogue holds a vibe's books.
fn key(vibe: &Vibe) -> VibeBooksKey {
    VibeBooksKey {
        vibe: vibe.id,
        built: vibe.built_at,
    }
}

/// A muted line in `body`.
fn muted(text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .body()
        .text_color(colors(cx).fg_muted)
        .child(text.into())
}

impl Render for VibeShelves {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !session::has_scope(cx, READ_VIBES) {
            return div().into_any_element();
        }
        // Discover's column: the region, or the list's width beside the
        // book pane, inside its 32 insets.
        let rem = window.rem_size();
        let width = list_width(region_width(window, rems(0.)), rem) - rems(4.).to_pixels(rem);
        let sections: Vec<(VibeKind, Section)> = SECTIONS
            .iter()
            .map(|k| (*k, self.section(*k, cx)))
            .collect();
        let lead = sections
            .iter()
            .position(|(_, s)| matches!(s, Section::Covers { .. } | Section::Loading));
        let shown: Vec<AnyElement> = sections
            .into_iter()
            .enumerate()
            .filter_map(|(ix, (kind, section))| {
                self.render_section(kind, section, Some(ix) == lead, width, window, cx)
            })
            .collect();
        if shown.is_empty() {
            return div().into_any_element();
        }
        // 24 under the page title, the sections 32 apart, Trending too
        // (8 here, then Discover's 16 and 8).
        v_flex()
            .pt_6()
            .pb_2()
            .gap_8()
            .children(shown)
            .into_any_element()
    }
}
