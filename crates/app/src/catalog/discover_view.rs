//! Discover (`hardcover-design`, Discover): the hub for books the reader
//! doesn't know yet. One page, inset 32, filling the region and scrolling
//! as one: the page title, then the sections ([`super::discover`] decides
//! which show, their order and the lead):
//!
//! 1. **For you:** Hardcover's Recommendations vibe, in its order, with the
//!    muted line "Hardcover's recommendations from your library." and "See
//!    all 250", which opens the vibe's page.
//! 2. **Browse:** three groups of page chips, each row one Tab stop
//!    (`ui::chip_keys`): "By mood" (the 14 moods, `Mood::ALL`), "By genre"
//!    (`BROWSE_GENRES`, with their icons), each opening its tag page, and
//!    "Featured on Hardcover" (`VibesModel`'s featured vibes), each opening
//!    its vibe's page.
//! 3. **Top picks:** Hardcover's Top Picks vibe, with "See all 35".
//! 4. **Because you loved <title>:** the similar books of the reader's latest
//!    5-star read ([`super::discover::loved_seed`]), from the catalogue (the
//!    book's details, then `BooksByIds`, as the book page's Similar books),
//!    less the books in the library, For you's loaded books and Top picks'
//!    books while Top picks shows. No "See all".
//! 5. **Trending this month:** one book per series
//!    ([`super::discover::one_per_series`]), with "See all", which opens
//!    Trending (`Page::Trending`).
//!
//! - **One lead:** the first shelf with covers (or loading them) has lg
//!   covers with the cover shadow; every other md, flat. Each shelf holds as
//!   many as fit the column, in one row. Browse comes second.
//! - **Without `read:vibes`:** For you, Top picks and the featured group
//!   hide, and Because you loved leads; without a 5-star read, or similar
//!   books for it, Trending does.
//! - **A vibe not built yet:** its section keeps its title and place, and
//!   shows one muted sentence and "Open on Hardcover" where the shelf
//!   would be.
//! - A book the reader said never to recommend is left out of For you and
//!   Top picks at once (`VibesModel::shown_books`).
//! - **States:** skeleton covers while a shelf loads; a failure is one
//!   muted sentence with "Try again" in the shelf's place, the other
//!   sections staying; a section with nothing to show hides.
//! - **Keys:** Page Up and Page Down scroll the page wherever focus is in
//!   it; with the page itself focused, Home, End, Space and the arrows too
//!   (`ui::page_scroll`). Focus moved by the keyboard is kept in view.
//!
//! Not a cached view: it draws covers, whose loads redraw it anyway.

use std::collections::HashSet;

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::v_flex;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use hardcover_core::auth::scopes::READ_VIBES;
use hardcover_core::tags::{BROWSE_GENRES, Mood};
use hardcover_core::{BookId, BookSummary, TrendingPeriod, Vibe, VibeKind};

use super::discover::{self, Section, Shows};
use super::navigation::{self, Page};
use super::tag::Tag;
use crate::chip::{TagChip, TagGroup, TagRow, tag_groups};
use crate::cover::{Cover, CoverSize};
use crate::design::size;
use crate::library::model::LibraryModel;
use crate::preferences::AppPreferences;
use crate::resource::catalog::VibeBooksKey;
use crate::resource::{Catalog, Demand, LoadStatus, SharedChanged};
use crate::session;
use crate::ui::chip_keys::ChipKeys;
use crate::ui::page_scroll::scroll_page;
use crate::ui::reveal::RevealScroll as _;
use crate::ui::{
    CoverShelf, ExternalLink, LoadFailed, PageHeader, SectionHeader, ShelfSection, TextButton,
    books_that_fit, region_width, see_all,
};
use crate::vibes::model::VibesModel;
use crate::vibes::{FOR_YOU_INTRO, vibe_name};

/// Trending this month's period.
const TRENDING: TrendingPeriod = TrendingPeriod::Month;

/// The sentence in a vibe's place while Hardcover hasn't built it.
const NOT_BUILT: &str = "Hardcover builds this list when you open it on hardcover.app.";

/// The book "Because you loved" goes from.
#[derive(Clone, PartialEq)]
struct Seed {
    book: BookId,
    title: String,
}

/// What a shelf shows.
enum Shelf {
    /// Its books (as many as fit are drawn), and how many there are in all
    /// when "See all" opens more.
    Books {
        books: Vec<BookSummary>,
        see_all: Option<SeeAll>,
    },
    Loading,
    Failed(SharedString),
    /// A vibe Hardcover hasn't built: a sentence and the link.
    NotBuilt(Vibe),
    Hidden,
}

impl Shelf {
    fn shows(&self) -> Shows {
        match self {
            Shelf::Books { .. } | Shelf::Loading => Shows::Covers,
            Shelf::Failed(_) | Shelf::NotBuilt(_) => Shows::Note,
            Shelf::Hidden => Shows::Hidden,
        }
    }
}

/// Where "See all" goes, and the count it names.
#[derive(Clone)]
struct SeeAll {
    page: Page,
    /// `None`: "See all" without a count (Trending).
    count: Option<usize>,
}

pub struct DiscoverView {
    model: Entity<LibraryModel>,
    /// The session's catalogue; `None` once signed out.
    catalog: Option<Catalog>,
    vibes: Option<Entity<VibesModel>>,
    /// "Because you loved"'s book, and the library revision it was worked
    /// out from.
    seed: Option<(u64, Option<Seed>)>,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
    /// Browse's chip rows, each one Tab stop.
    mood_keys: Entity<ChipKeys>,
    genre_keys: Entity<ChipKeys>,
    featured_keys: Entity<ChipKeys>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for DiscoverView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl DiscoverView {
    pub fn new(model: &Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let catalog = Catalog::global(cx);
        let vibes = VibesModel::global(cx);
        let mut subscriptions = vec![
            cx.observe(&AppPreferences::global(cx).clone(), |_, _, cx| cx.notify()),
            session::observe_grant(cx, |_, cx| cx.notify()),
            cx.observe(model, |this, _, cx| {
                // A new 5-star read changes "Because you loved"'s book: ask
                // for its similar books now, not at the next Refresh. Only
                // once the page has worked a seed out, so a Discover never
                // opened asks for nothing.
                if let Some((_, before)) = this.seed.clone() {
                    let before = before.map(|s| s.book);
                    let after = this.seed_book(cx);
                    if after != before
                        && let (Some(book), Some(catalog)) = (after, this.catalog.clone())
                    {
                        catalog.request_details(book, Demand::Active, cx);
                        catalog.request_similar(book, Demand::Active, cx);
                    }
                }
                cx.notify();
            }),
        ];
        if let Some(vibes) = &vibes {
            // A vibe landed or was built again: its books follow.
            subscriptions.push(cx.observe(vibes, |this, _, cx| {
                this.request_vibe_books(Demand::Active, cx);
                cx.notify();
            }));
        }
        if let Some(catalog) = &catalog {
            subscriptions.push(cx.observe(&catalog.vibe_books, |_, _, cx| cx.notify()));
            subscriptions.push(cx.subscribe(
                &catalog.trending,
                |_, _, SharedChanged(period): &SharedChanged<TrendingPeriod>, cx| {
                    if *period == TRENDING {
                        cx.notify();
                    }
                },
            ));
            subscriptions.push(cx.subscribe(
                &catalog.details,
                |this, _, SharedChanged(id): &SharedChanged<BookId>, cx| {
                    if this.seed_book(cx) == Some(*id) {
                        // The details name the similar books: ask for them.
                        if let Some(catalog) = &this.catalog {
                            catalog.request_similar(*id, Demand::Active, cx);
                        }
                        cx.notify();
                    }
                },
            ));
            subscriptions.push(cx.subscribe(
                &catalog.similar,
                |this, _, SharedChanged(id): &SharedChanged<BookId>, cx| {
                    if this.seed_book(cx) == Some(*id) {
                        cx.notify();
                    }
                },
            ));
        }
        Self {
            model: model.clone(),
            catalog,
            vibes,
            seed: None,
            scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            mood_keys: cx.new(|cx| ChipKeys::new(window, cx)),
            genre_keys: cx.new(|cx| ChipKeys::new(window, cx)),
            featured_keys: cx.new(|cx| ChipKeys::new(window, cx)),
            _subscriptions: subscriptions,
        }
    }

    /// Discover was opened from the sidebar: asks for what it shows that
    /// this session hasn't yet, or again if it is stale or failed.
    pub fn show(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.request(Demand::Shown, cx);
        cx.notify();
    }

    /// Asks for what the page shows again if it is stale (`resource`; a
    /// failed one only when `shown_again`).
    pub fn revalidate(&mut self, shown_again: bool, cx: &mut Context<Self>) {
        self.request(super::book_list::demand(shown_again), cx);
    }

    /// Focuses the page itself: Tab goes on to its first control, and the
    /// keys scroll it.
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
    }

    fn request(&mut self, demand: Demand, cx: &mut Context<Self>) {
        if let Some(vibes) = &self.vibes {
            vibes.update(cx, |m, cx| {
                m.request_system(demand, cx);
                m.request_featured(demand, cx);
            });
        }
        self.request_vibe_books(demand, cx);
        if let Some(catalog) = self.catalog.clone() {
            catalog.request_trending(TRENDING, demand, cx);
            if let Some(seed) = self.seed_book(cx) {
                catalog.request_details(seed, demand, cx);
                // At once when the details are in; else once they land.
                catalog.request_similar(seed, demand, cx);
            }
        }
    }

    fn request_vibe_books(&self, demand: Demand, cx: &mut App) {
        let (Some(vibes), Some(catalog)) = (&self.vibes, &self.catalog) else {
            return;
        };
        let wanted: Vec<(VibeBooksKey, Vec<BookId>)> =
            [VibeKind::Recommendations, VibeKind::TopPicks]
                .iter()
                .filter_map(|kind| vibes.read(cx).system_vibe(*kind))
                .filter(|v| v.is_built())
                .map(|v| (vibe_key(v), v.book_ids.clone()))
                .collect();
        for (key, ids) in wanted {
            catalog.request_vibe_books(key, ids, demand, cx);
        }
    }

    /// "Because you loved"'s book, worked out again when the library
    /// changes.
    fn seed(&mut self, cx: &App) -> Option<Seed> {
        let model = self.model.read(cx);
        let revision = model.revision();
        match &self.seed {
            Some((r, seed)) if *r == revision => seed.clone(),
            _ => {
                let seed = discover::loved_seed(model.entries()).map(|e| Seed {
                    book: e.book.id,
                    title: e.book.title.clone(),
                });
                self.seed = Some((revision, seed.clone()));
                seed
            }
        }
    }

    fn seed_book(&mut self, cx: &App) -> Option<BookId> {
        self.seed(cx).map(|s| s.book)
    }

    // ----- the sections -----

    fn vibe_shelf(&self, kind: VibeKind, cx: &App) -> Shelf {
        let (Some(vibes), Some(catalog)) = (&self.vibes, &self.catalog) else {
            return Shelf::Hidden;
        };
        if !session::has_scope(cx, READ_VIBES) {
            return Shelf::Hidden;
        }
        let model = vibes.read(cx);
        let Some(vibe) = model.system_vibe(kind) else {
            return match model.system_status() {
                LoadStatus::Loading if !model.system_loaded() => Shelf::Loading,
                // One request reads both: its failure shows once.
                LoadStatus::Failed(message)
                    if !model.system_loaded() && kind == VibeKind::Recommendations =>
                {
                    Shelf::Failed(message)
                }
                _ => Shelf::Hidden,
            };
        };
        if !vibe.is_built() {
            return Shelf::NotBuilt(vibe.clone());
        }
        let shown: Vec<BookId> = VibesModel::shown_books(vibe, cx);
        let store = catalog.vibe_books.read(cx).resource();
        let key = vibe_key(vibe);
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
                    Shelf::Hidden
                } else {
                    Shelf::Books {
                        books,
                        see_all: Some(SeeAll {
                            page: Page::Vibe(vibe.id),
                            count: Some(shown.len()),
                        }),
                    }
                }
            }
            None => match store.error(&key) {
                Some(message) if !store.is_loading(&key) => Shelf::Failed(message.clone()),
                _ => Shelf::Loading,
            },
        }
    }

    /// The books Top picks holds, when it shows: "Because you loved" leaves
    /// them out.
    fn top_pick_ids(&self, top_picks: &Shelf, cx: &App) -> HashSet<BookId> {
        if top_picks.shows() != Shows::Covers {
            return HashSet::new();
        }
        self.vibes
            .as_ref()
            .and_then(|v| v.read(cx).system_vibe(VibeKind::TopPicks))
            .map(|vibe| VibesModel::shown_books(vibe, cx).into_iter().collect())
            .unwrap_or_default()
    }

    fn loved_shelf(&self, seed: Option<&Seed>, above: &HashSet<BookId>, cx: &App) -> Shelf {
        let (Some(seed), Some(catalog)) = (seed, &self.catalog) else {
            return Shelf::Hidden;
        };
        let details = catalog.details.read(cx).resource();
        let Some(book) = details.value(&seed.book) else {
            return match details.error(&seed.book) {
                Some(message) if !details.is_loading(&seed.book) => Shelf::Failed(message.clone()),
                _ => Shelf::Loading,
            };
        };
        if book.similar_book_ids.iter().all(|id| *id == seed.book) {
            return Shelf::Hidden;
        }
        let similar = catalog.similar.read(cx).resource();
        match similar.value(&seed.book) {
            Some(books) => {
                let in_library: HashSet<BookId> = self
                    .model
                    .read(cx)
                    .entries()
                    .iter()
                    .map(|e| e.book.id)
                    .collect();
                let books: Vec<BookSummary> = discover::loved_books(books, &in_library, above)
                    .into_iter()
                    .cloned()
                    .collect();
                if books.is_empty() {
                    Shelf::Hidden
                } else {
                    Shelf::Books {
                        books,
                        see_all: None,
                    }
                }
            }
            None => match similar.error(&seed.book) {
                Some(message) if !similar.is_loading(&seed.book) => Shelf::Failed(message.clone()),
                _ => Shelf::Loading,
            },
        }
    }

    fn trending_shelf(&self, cx: &App) -> Shelf {
        let Some(catalog) = &self.catalog else {
            return Shelf::Hidden;
        };
        let store = catalog.trending.read(cx).resource();
        match store.value(&TRENDING) {
            Some(books) => {
                let books: Vec<BookSummary> = discover::one_per_series(books)
                    .into_iter()
                    .cloned()
                    .collect();
                if books.is_empty() {
                    Shelf::Hidden
                } else {
                    Shelf::Books {
                        books,
                        see_all: Some(SeeAll {
                            page: Page::Trending(TRENDING),
                            count: None,
                        }),
                    }
                }
            }
            None => match store.error(&TRENDING) {
                Some(message) if !store.is_loading(&TRENDING) => Shelf::Failed(message.clone()),
                _ => Shelf::Loading,
            },
        }
    }

    /// "Try again" on a shelf: asks for what it shows now.
    fn retry(&mut self, section: Section, cx: &mut Context<Self>) {
        match section {
            Section::ForYou | Section::TopPicks => {
                if let Some(vibes) = &self.vibes {
                    vibes.update(cx, |m, cx| m.request_system(Demand::Now, cx));
                }
                self.request_vibe_books(Demand::Now, cx);
            }
            Section::Loved => {
                if let (Some(seed), Some(catalog)) = (self.seed_book(cx), self.catalog.clone()) {
                    catalog.request_details(seed, Demand::Now, cx);
                    catalog.request_similar(seed, Demand::Now, cx);
                }
            }
            Section::Trending => {
                if let Some(catalog) = &self.catalog {
                    catalog.request_trending(TRENDING, Demand::Now, cx);
                }
            }
            Section::Browse => {}
        }
        cx.notify();
    }

    // ----- drawing -----

    fn render_shelf(
        &self,
        section: Section,
        title: SharedString,
        shelf: Shelf,
        size: CoverSize,
        fit: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // The lead's covers take the shadow.
        let lead = size == CoverSize::Lg;
        let id = section_id(section);
        let mut out = ShelfSection::new(title);
        // For you's line sits between its title and its shelf.
        if section == Section::ForYou && matches!(shelf, Shelf::Books { .. } | Shelf::Loading) {
            out = out.intro(FOR_YOU_INTRO);
        }
        match shelf {
            Shelf::Hidden => return div().into_any_element(),
            Shelf::Books {
                books,
                see_all: more,
            } => {
                if let Some(more) = more
                    && more.count.is_none_or(|n| n > books.len().min(fit))
                {
                    let page = more.page.clone();
                    out = out.action(see_all(
                        SharedString::from(format!("{id}-see-all")),
                        more.count.map(|n| u32::try_from(n).unwrap_or(u32::MAX)),
                        move |_, _, cx| navigation::open(page.clone(), cx),
                    ));
                }
                out = out.books(
                    CoverShelf::new(size)
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
                                TextButton::book(
                                    SharedString::from(format!("{id}-title-{raw}")),
                                    book,
                                ),
                            )
                        })),
                );
            }
            Shelf::Loading => out = out.loading(size, fit),
            Shelf::Failed(message) => {
                out = out.failed(
                    LoadFailed::new(SharedString::from(format!("{id}-retry")), message)
                        .on_retry(cx.listener(move |this, _, _, cx| this.retry(section, cx))),
                );
            }
            Shelf::NotBuilt(vibe) => {
                out = out.note(
                    NOT_BUILT,
                    ExternalLink::new(
                        SharedString::from(format!("{id}-on-hardcover")),
                        "Open on Hardcover",
                        vibe.web_url(),
                    ),
                );
            }
        }
        out.into_any_element()
    }

    /// Browse: the moods, the genres and the featured vibes as page chips.
    fn render_browse(&self, cx: &App) -> AnyElement {
        let moods = TagRow::new(Mood::ALL.into_iter().map(|mood| {
            TagChip::new(mood.name()).opens(Page::Tag {
                tag: Tag::Mood(mood),
                within: None,
            })
        }))
        .keys("browse-moods", &self.mood_keys);
        let genres = TagRow::new(BROWSE_GENRES.into_iter().map(|genre| {
            TagChip::genre(genre.name).opens(Page::Tag {
                tag: Tag::Genre(genre),
                within: None,
            })
        }))
        .keys("browse-genres", &self.genre_keys);
        let mut groups = vec![
            TagGroup::row("By mood", moods),
            TagGroup::row("By genre", genres),
        ];
        let featured: Vec<Vibe> = self
            .vibes
            .as_ref()
            .filter(|_| session::has_scope(cx, READ_VIBES))
            .and_then(|v| v.read(cx).featured_vibes().map(<[Vibe]>::to_vec))
            .unwrap_or_default();
        if !featured.is_empty() {
            let row = TagRow::new(featured.into_iter().map(|vibe| {
                TagChip::new(vibe_name(&vibe))
                    .key(ElementId::NamedInteger(
                        "featured-vibe".into(),
                        vibe.id.get().cast_unsigned(),
                    ))
                    .opens(Page::Vibe(vibe.id))
            }))
            .keys("browse-featured", &self.featured_keys);
            groups.push(TagGroup::row("Featured on Hardcover", row));
        }
        v_flex()
            .w_full()
            .gap_3()
            .child(SectionHeader::new("Browse"))
            .child(tag_groups(groups))
            .into_any_element()
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let all_keys = self.focus_handle.is_focused(window);
        if scroll_page(&self.scroll, event, all_keys, window) {
            cx.notify();
            cx.stop_propagation();
        }
    }
}

/// Which page of the catalogue holds a vibe's books.
fn vibe_key(vibe: &Vibe) -> VibeBooksKey {
    VibeBooksKey {
        vibe: vibe.id,
        built: vibe.built_at,
    }
}

/// A section's id, for its controls'.
fn section_id(section: Section) -> &'static str {
    match section {
        Section::ForYou => "for-you",
        Section::Browse => "browse",
        Section::TopPicks => "top-picks",
        Section::Loved => "loved",
        Section::Trending => "trending-month",
    }
}

impl DiscoverView {
    /// What each shelf shows, the sections in order and the lead.
    fn sections(&mut self, cx: &App) -> (Vec<(Section, Shelf)>, Vec<Section>, Option<Section>) {
        let seed = self.seed(cx);
        let for_you = self.vibe_shelf(VibeKind::Recommendations, cx);
        let top_picks = self.vibe_shelf(VibeKind::TopPicks, cx);
        // "Because you loved" repeats nothing the vibes above it hold:
        // For you's loaded books, and Top picks' books while it shows.
        let mut above = self.top_pick_ids(&top_picks, cx);
        if let Shelf::Books { books, .. } = &for_you {
            above.extend(books.iter().map(|b| b.id));
        }
        let loved = self.loved_shelf(seed.as_ref(), &above, cx);
        let trending = self.trending_shelf(cx);
        let shelves = vec![
            (Section::ForYou, for_you),
            (Section::TopPicks, top_picks),
            (Section::Loved, loved),
            (Section::Trending, trending),
        ];
        let shows: Vec<(Section, Shows)> = shelves
            .iter()
            .map(|(s, shelf)| (*s, shelf.shows()))
            .collect();
        let (order, lead) = discover::layout(&shows);
        (shelves, order, lead)
    }
}

impl Render for DiscoverView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let seed = self.seed(cx);
        let (shelves, order, lead) = self.sections(cx);
        let mut shelves: Vec<(Section, Option<Shelf>)> = shelves
            .into_iter()
            .map(|(s, shelf)| (s, Some(shelf)))
            .collect();
        // The column inside the page's 32 insets.
        let width = region_width(window, size::PAGE_INSET);
        let sections: Vec<AnyElement> = order
            .into_iter()
            .map(|section| {
                if section == Section::Browse {
                    return self.render_browse(cx);
                }
                let shelf = shelves
                    .iter_mut()
                    .find(|(s, _)| *s == section)
                    .and_then(|(_, shelf)| shelf.take())
                    .unwrap_or(Shelf::Hidden);
                let title: SharedString = match section {
                    Section::ForYou => "For you".into(),
                    Section::TopPicks => "Top picks".into(),
                    Section::Loved => format!(
                        "Because you loved {}",
                        seed.as_ref().map_or("", |s| s.title.as_str())
                    )
                    .into(),
                    _ => "Trending this month".into(),
                };
                let size = if Some(section) == lead {
                    CoverSize::Lg
                } else {
                    CoverSize::Md
                };
                let fit = books_that_fit(width, size, window.rem_size()).max(1);
                self.render_shelf(section, title, shelf, size, fit, cx)
            })
            .collect();
        let page = v_flex()
            .w_full()
            .p_8()
            .child(PageHeader::new("Discover"))
            // The first section 24 under the title, the others 32 apart.
            .child(v_flex().w_full().pt_6().gap_8().children(sections));
        // The scrollbar overlays a wrapper: inside the scroller it would
        // scroll away with the page.
        div()
            .id("discover-region")
            .relative()
            .size_full()
            .child(
                div()
                    .id("discover")
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(Self::on_key_down))
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(page)
                    .reveals_focus(&self.scroll),
            )
            .vertical_scrollbar(&self.scroll)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;
    use std::time::Duration;

    use gpui_kit::{TestAppContext, VisualTestContext, px, size as gsize};
    use hardcover_core::VibeId;
    use hardcover_core::demo::Faults;
    use hardcover_core::prefs::Preferences;
    use hardcover_core::store::LibraryCache;
    use hardcover_core::tags::browse_genre;

    use super::*;
    use crate::catalog::navigation::{NavigationEvent, Navigator};
    use crate::session::SessionScope;
    use crate::ui::test_support::{press, redraw, tab};

    /// The demo reader's Recommendations ("For you").
    const FOR_YOU: VibeId = VibeId::new(501);

    fn settle(cx: &mut VisualTestContext) {
        redraw(cx);
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        redraw(cx);
    }

    type Harness<'a> = (
        Entity<DiscoverView>,
        Rc<RefCell<Vec<Page>>>,
        Entity<VibesModel>,
        &'a mut VisualTestContext,
    );

    /// Discover over the demo with `faults`, 900 wide (six lg covers fit
    /// For you's row), the pages it opened, and the vibes model it reads
    /// (the workspace owns it; the test holds it).
    fn discover<'a>(faults: &str, cx: &'a mut TestAppContext) -> Harness<'a> {
        let faults = Faults::parse(faults).unwrap();
        let cache = LibraryCache::open_in_memory().ok().map(Arc::new);
        let session = SessionScope::for_tests(hardcover_core::demo::client_with(&faults), cache);
        let library = cx.new(|_| LibraryModel::new(session.clone()));
        let opened: Rc<RefCell<Vec<Page>>> = Rc::default();
        let vibes = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::kit::bind_keys(cx);
            crate::ui::chip_keys::bind_keys(cx);
            session.install(cx);
            Catalog::init(&session, cx);
            crate::preferences::init(Preferences::default(), None, cx);
            let vibes = VibesModel::init(session.clone(), &library, cx);
            let navigator = Navigator::init(cx);
            let opened = opened.clone();
            cx.subscribe(&navigator, move |_, event: &NavigationEvent, _| {
                if let NavigationEvent::Open(page) = event {
                    opened.borrow_mut().push(page.clone());
                }
            })
            .detach();
            vibes
        });
        library.update(cx, LibraryModel::start);
        cx.run_until_parked();
        let (view, cx) = cx.add_window_view(|window, cx| DiscoverView::new(&library, window, cx));
        cx.simulate_resize(gsize(px(900.), px(700.)));
        // Focus events (a chip row's memory of its chip) need it active.
        cx.update(|window, _| window.activate_window());
        cx.update(|window, cx| {
            view.update(cx, |v, cx| {
                v.show(window, cx);
                v.focus(window, cx);
            });
        });
        settle(cx);
        (view, opened, vibes, cx)
    }

    fn sections(
        view: &Entity<DiscoverView>,
        cx: &mut VisualTestContext,
    ) -> (Vec<(Section, Shelf)>, Vec<Section>, Option<Section>) {
        view.update(cx, |v, cx| v.sections(cx))
    }

    fn titles(shelves: &[(Section, Shelf)], section: Section) -> Vec<String> {
        match shelves.iter().find(|(s, _)| *s == section) {
            Some((_, Shelf::Books { books, .. })) => {
                books.iter().map(|b| b.title.clone()).collect()
            }
            _ => Vec::new(),
        }
    }

    #[gpui_kit::test]
    fn for_you_leads_then_browse_top_picks_loved_and_trending(cx: &mut TestAppContext) {
        let (view, _, _vibes, cx) = discover("", cx);
        let (shelves, order, lead) = sections(&view, cx);
        use Section::*;
        assert_eq!(order, [ForYou, Browse, TopPicks, Loved, Trending]);
        assert_eq!(lead, Some(ForYou));
        // Dune is the demo's latest 5-star read; of its similar books only
        // Children of Dune is neither in the library nor in For you or Top
        // picks.
        assert_eq!(titles(&shelves, Loved), ["Children of Dune"]);
        // One book per series: Dune's four books bring one.
        let trending = titles(&shelves, Trending);
        let dune = trending
            .iter()
            .filter(|t| {
                [
                    "Dune",
                    "Dune Messiah",
                    "Children of Dune",
                    "God Emperor of Dune",
                ]
                .contains(&t.as_str())
            })
            .count();
        assert_eq!(dune, 1, "{trending:?}");
        let see_all = shelves.iter().find_map(|(s, shelf)| match (s, shelf) {
            (Trending, Shelf::Books { see_all, .. }) => see_all.as_ref().map(|m| m.page.clone()),
            _ => None,
        });
        assert_eq!(see_all, Some(Page::Trending(TrendingPeriod::Month)));
    }

    #[gpui_kit::test]
    fn a_new_five_star_read_brings_its_similar_books_while_discover_is_open(
        cx: &mut TestAppContext,
    ) {
        use crate::library::model::{LibraryWrite, WriteKind};
        use hardcover_core::Rating;

        let (view, _, _vibes, cx) = discover("", cx);
        let library = view.read_with(cx, |v, _| v.model.clone());
        // The Martian, finished lately and unrated, after Dune.
        let martian = library.read_with(cx, |m, _| {
            m.entries()
                .iter()
                .find(|e| e.book.title == "The Martian")
                .map(|e| (e.user_book_id, e.status))
        });
        let (id, keep) = martian.expect("the demo has The Martian");
        cx.update(|window, cx| {
            let write = LibraryWrite::entry(id, WriteKind::Rating, "Couldn't save the rating", {
                move |client| client.set_rating(id, Rating::new(5.0), keep)
            });
            drop(library.update(cx, |m, cx| m.write(write, window, cx)));
        });
        settle(cx);
        settle(cx);

        let seed = view.update(cx, |v, cx| v.seed(cx)).map(|s| s.title);
        assert_eq!(seed.as_deref(), Some("The Martian"));
        let (shelves, _, _) = sections(&view, cx);
        let loved = shelves.iter().find(|(s, _)| *s == Section::Loved);
        assert!(
            !matches!(loved, Some((_, Shelf::Loading))),
            "its similar books were asked for, not left loading"
        );
    }

    #[gpui_kit::test]
    fn without_the_vibes_scope_because_you_loved_leads(cx: &mut TestAppContext) {
        let (view, _, _vibes, cx) = discover("scope=read:vibes", cx);
        let (shelves, order, lead) = sections(&view, cx);
        use Section::*;
        assert_eq!(order, [Loved, Browse, Trending]);
        assert_eq!(lead, Some(Loved));
        assert_eq!(
            titles(&shelves, Loved),
            ["Children of Dune"],
            "Top picks doesn't show, so it takes nothing away"
        );
    }

    #[gpui_kit::test]
    fn a_vibe_not_built_keeps_its_place_and_gives_up_the_lead(cx: &mut TestAppContext) {
        let (view, _, _vibes, cx) = discover("vibes=unbuilt", cx);
        let (shelves, order, lead) = sections(&view, cx);
        use Section::*;
        assert_eq!(order, [ForYou, Browse, TopPicks, Loved, Trending]);
        assert!(matches!(shelves[0].1, Shelf::NotBuilt(_)));
        assert_eq!(lead, Some(Loved));
    }

    #[gpui_kit::test]
    fn a_failed_shelf_says_so_and_the_others_stay(cx: &mut TestAppContext) {
        let (view, _, _vibes, cx) = discover("500", cx);
        let (shelves, order, _) = sections(&view, cx);
        assert!(
            shelves
                .iter()
                .any(|(_, shelf)| matches!(shelf, Shelf::Failed(_))),
            "every request fails: a failure in a shelf's place"
        );
        assert!(order.contains(&Section::Browse), "Browse needs no request");
    }

    /// From the page: Tab reaches For you's "See all 11", which opens the
    /// vibe; then past its six titles, the moods (one Tab stop, the arrows
    /// moving along it) and the genres, each chip opening its tag's page,
    /// and the featured vibes, each opening the vibe's.
    #[gpui_kit::test]
    fn see_all_and_the_browse_chips_open_their_pages(cx: &mut TestAppContext) {
        let (_, opened, _vibes, cx) = discover("", cx);
        tab(cx);
        press(cx, "enter");
        settle(cx);
        assert_eq!(opened.borrow().last(), Some(&Page::Vibe(FOR_YOU)));

        for _ in 0..7 {
            tab(cx);
        }
        press(cx, "enter");
        press(cx, "right");
        redraw(cx);
        press(cx, "enter");
        press(cx, "end");
        redraw(cx);
        press(cx, "enter");
        // Tab leaves the row for the genres', on its first chip.
        press(cx, "tab");
        settle(cx);
        press(cx, "enter");
        // The featured vibes' row: its first opens the vibe's page.
        press(cx, "tab");
        settle(cx);
        press(cx, "enter");
        assert_eq!(opened.borrow().last(), Some(&Page::Vibe(VibeId::new(601))));
        // Shift+Tab comes back to the genre it left from.
        press(cx, "shift-tab");
        settle(cx);
        press(cx, "enter");
        let tags: Vec<Tag> = opened
            .borrow()
            .iter()
            .filter_map(|page| match page {
                Page::Tag { tag, within: None } => Some(*tag),
                _ => None,
            })
            .collect();
        assert_eq!(
            tags,
            [
                Tag::Mood(Mood::Adventurous),
                Tag::Mood(Mood::Emotional),
                Tag::Mood(Mood::Relaxing),
                Tag::Genre(browse_genre("Fantasy").unwrap()),
                Tag::Genre(browse_genre("Fantasy").unwrap()),
            ]
        );
    }

    #[gpui_kit::test]
    fn page_down_scrolls_the_page_from_a_chip_too(cx: &mut TestAppContext) {
        let (view, _, _vibes, cx) = discover("", cx);
        let offset = |cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.scroll.offset().y);
        assert_eq!(offset(cx), px(0.));
        press(cx, "pagedown");
        redraw(cx);
        assert!(offset(cx) < px(0.), "scrolled down");
        press(cx, "home");
        redraw(cx);
        assert_eq!(offset(cx), px(0.));
        // On a chip, Home is the row's; Page Up and Page Down still scroll.
        for _ in 0..8 {
            tab(cx);
        }
        let on_chip = offset(cx);
        press(cx, "pagedown");
        redraw(cx);
        assert!(offset(cx) < on_chip, "Page Down from a chip");
    }
}
