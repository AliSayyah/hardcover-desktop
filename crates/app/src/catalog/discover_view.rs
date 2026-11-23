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
//!    less the books in the library and Top picks' books while Top picks
//!    shows. No "See all".
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
            cx.observe(model, |_, _, cx| cx.notify()),
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

    fn loved_shelf(&self, seed: Option<&Seed>, top_picks: &HashSet<BookId>, cx: &App) -> Shelf {
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
                let books: Vec<BookSummary> = discover::loved_books(books, &in_library, top_picks)
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
        lead: bool,
        width: gpui_kit::Pixels,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let size = if lead { CoverSize::Lg } else { CoverSize::Md };
        let fit = books_that_fit(width, size, window.rem_size()).max(1);
        let id = section_id(section);
        let mut out = ShelfSection::new(title);
        if section == Section::ForYou {
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

impl Render for DiscoverView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let seed = self.seed(cx);
        let for_you = self.vibe_shelf(VibeKind::Recommendations, cx);
        let top_picks = self.vibe_shelf(VibeKind::TopPicks, cx);
        let top_pick_ids = self.top_pick_ids(&top_picks, cx);
        let loved = self.loved_shelf(seed.as_ref(), &top_pick_ids, cx);
        let trending = self.trending_shelf(cx);
        let (order, lead) = discover::layout(&[
            (Section::ForYou, for_you.shows()),
            (Section::TopPicks, top_picks.shows()),
            (Section::Loved, loved.shows()),
            (Section::Trending, trending.shows()),
        ]);
        let mut shelves = [
            (Section::ForYou, Some(for_you)),
            (Section::TopPicks, Some(top_picks)),
            (Section::Loved, Some(loved)),
            (Section::Trending, Some(trending)),
        ];
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
                self.render_shelf(
                    section,
                    title,
                    shelf,
                    Some(section) == lead,
                    width,
                    window,
                    cx,
                )
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
