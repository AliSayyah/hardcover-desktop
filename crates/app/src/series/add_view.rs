//! Add series (`hardcover-design`, Screens → Add series), a page over
//! Series: the series of the books on the reader's shelves that they don't
//! follow yet, each with Follow. The workspace draws the object bar with
//! Back above it.
//!
//! - **Content:** inset 32, at most `size::PAGE_BESIDE_PANE` wide: the page
//!   title and its line, with the filter on the header's trailing edge;
//!   then "Started" (a book read, being read, paused or given up on, the
//!   most recently active first) and "Only on Want to read", each a label
//!   with its count over a sequence of rows.
//! - **Row:** the sm cover of the series' first book on the shelves (opens
//!   the book), the name (opens the series page), "author · how far", and
//!   Follow, or a pressed "Following" once followed here. A followed row
//!   stays where it is until the page is opened again, so nothing moves
//!   under the pointer and a slip is one press to undo.
//! - **Without `write:social`:** every Follow is disabled, and one line
//!   under the header says why.
//! - **Loading:** skeleton rows until the library and the follows are known;
//!   until then nothing is offered (a series followed already would show).
//! - **Empty:** an empty state, also when the filter matches nothing.

use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable as _, Icon, IconName, Selectable as _, h_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::SeriesId;
use hardcover_core::auth::scopes::WRITE_SOCIAL;

use super::follows::{self, FollowsModel};
use super::progress::{Unfollowed, unfollowed, unfollowed_matches};
use crate::catalog::navigation::Page;
use crate::cover::{Cover, CoverSize};
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{count_text, needs_permission};
use crate::library::model::LibraryModel;
use crate::resource::LoadStatus;
use crate::session;
use crate::text::search_words;
use crate::ui::kit::{RaisedField as _, SmallControl as _};
use crate::ui::reveal::RevealScroll as _;
use crate::ui::{EmptyState, Lane, LoadFailed, PageHeader, Sequence, Skeleton, TextButton};

/// Skeleton rows while the page loads.
const SKELETON_ROWS: usize = 5;

/// What the rows were worked out from: the library's and the follows'
/// revisions, and how many series were followed here.
type RowsKey = (u64, u64, usize);

pub struct AddSeriesView {
    focus_handle: FocusHandle,
    library: Entity<LibraryModel>,
    follows: Entity<FollowsModel>,
    filter_input: Entity<InputState>,
    /// The series followed since the page was opened: their rows stay, as
    /// Following.
    followed_here: HashSet<SeriesId>,
    /// The rows, and what they were worked out from (the library's and the
    /// follows' revisions, and `followed_here`'s size: it only grows).
    rows: Option<(RowsKey, Rc<Vec<Unfollowed>>)>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for AddSeriesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl AddSeriesView {
    pub fn new(
        library: Entity<LibraryModel>,
        follows: Entity<FollowsModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter series"));
        let subscriptions = vec![
            cx.observe(&library, |_, _, cx| cx.notify()),
            cx.observe(&follows, |_, _, cx| cx.notify()),
            session::observe_grant(cx, |_, cx| cx.notify()),
            cx.subscribe(&filter_input, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        Self {
            focus_handle: cx.focus_handle(),
            library,
            follows,
            filter_input,
            followed_here: HashSet::new(),
            rows: None,
            _subscriptions: subscriptions,
        }
    }

    /// The page is opened (not come back to): the series followed last
    /// time leave, and the filter starts empty.
    pub fn opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.followed_here.clear();
        self.rows = None;
        self.filter_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.follows.update(cx, |follows, cx| {
            follows.request(crate::resource::Demand::Shown, cx)
        });
        cx.notify();
    }

    /// Focuses the filter.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.filter_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// The rows, worked out again only when the library, the follows or
    /// the series followed here change. `None` until both are known.
    fn rows(&mut self, cx: &App) -> Option<Rc<Vec<Unfollowed>>> {
        let library = self.library.read(cx);
        let follows = self.follows.read(cx);
        if !library.is_loaded() {
            return None;
        }
        follows.followed()?;
        let key = (
            library.revision(),
            follows.revision(),
            self.followed_here.len(),
        );
        if self.rows.as_ref().is_none_or(|(k, _)| *k != key) {
            let mut followed = follows.id_set();
            followed.retain(|id| !self.followed_here.contains(id));
            self.rows = Some((key, Rc::new(unfollowed(library.entries(), &followed))));
        }
        self.rows.as_ref().map(|(_, rows)| rows.clone())
    }

    fn follow(&mut self, id: SeriesId, window: &mut Window, cx: &mut Context<Self>) {
        self.followed_here.insert(id);
        follows::follow(id, window, cx);
        cx.notify();
    }

    /// "Following" pressed again: unfollows quietly (the row says so).
    fn unfollow(&mut self, id: SeriesId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(model) = FollowsModel::global(cx) else {
            return;
        };
        let task = model.update(cx, |m, cx| m.unfollow(id, cx));
        crate::notify::notify_failure(task, "Couldn't unfollow the series", window, cx);
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = colors(cx).fg_muted;
        PageHeader::new("Add series")
            .line("The series of the books on your shelves that you don't follow yet.")
            .actions(
                div().w(size::SELECT).child(
                    Input::new(&self.filter_input)
                        .raised_field(false, cx)
                        .cleanable(true)
                        .prefix(Icon::new(IconName::Search).text_color(muted)),
                ),
            )
    }

    fn row(&self, series: &Unfollowed, can_follow: bool, cx: &mut Context<Self>) -> AnyElement {
        let id = series.id;
        let muted = colors(cx).fg_muted;
        let follows = self.follows.read(cx);
        let following = follows.is_followed(id);
        let writing = follows.is_writing(id);
        let today = crate::dates::today();
        let line = match &series.author {
            Some(author) => format!("{author} · {}", series.line(today)),
            None => series.line(today),
        };
        let button = if following {
            // One id for Follow and Following, so focus stays as it turns.
            Button::new(SharedString::from(format!("add-series-follow-{id}")))
                .small_control()
                .selected(true)
                .icon(crate::icons::following())
                .label("Following")
                .loading(writing)
                .disabled(writing || !can_follow)
                .on_click(cx.listener(move |this, _, window, cx| this.unfollow(id, window, cx)))
        } else {
            Button::new(SharedString::from(format!("add-series-follow-{id}")))
                .small_control()
                .icon(crate::icons::follow())
                .label("Follow")
                .loading(writing)
                .disabled(writing || !can_follow)
                .when(!can_follow, |this| {
                    this.tooltip(needs_permission(WRITE_SOCIAL))
                })
                .on_click(cx.listener(move |this, _, window, cx| this.follow(id, window, cx)))
        };
        h_flex()
            .w_full()
            .gap_3()
            .child(
                Cover::new(CoverSize::Sm, series.book.title.clone())
                    .thumbnail(series.book.cover.as_ref().map(|c| c.url.clone()))
                    .opens(
                        SharedString::from(format!("add-series-cover-{id}")),
                        Page::Book(series.book.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        TextButton::new(
                            SharedString::from(format!("add-series-name-{id}")),
                            series.name.clone(),
                        )
                        .opens(Page::Series(id)),
                    )
                    .child(div().body().text_color(muted).truncate().child(line)),
            )
            .child(Lane::fit().child(button))
            .into_any_element()
    }

    fn group(
        &self,
        label: &'static str,
        rows: &[&Unfollowed],
        can_follow: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if rows.is_empty() {
            return None;
        }
        let muted = colors(cx).fg_muted;
        Some(
            div()
                .flex()
                .flex_col()
                .child(
                    h_flex()
                        .pt_6()
                        .pb_1()
                        .gap_2()
                        .justify_between()
                        .child(div().label(cx).child(label))
                        .child(
                            div()
                                .caption(cx)
                                .tabular()
                                .text_color(muted)
                                .child(count_text(rows.len())),
                        ),
                )
                .child(Sequence::new().children(rows.iter().map(|s| self.row(s, can_follow, cx))))
                .into_any_element(),
        )
    }
}

/// The page's column: inset 32, at most the list's width beside a pane.
fn column() -> gpui_kit::Div {
    div()
        .flex()
        .flex_col()
        .w_full()
        .max_w(size::PAGE_BESIDE_PANE)
        .p_8()
}

/// A row while the page loads: its cover, two lines and the button.
fn skeleton_row() -> AnyElement {
    h_flex()
        .w_full()
        .gap_3()
        .child(Skeleton::cover(CoverSize::Sm))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap_2()
                .child(Skeleton::line().w_3_5())
                .child(Skeleton::line().w_2_5()),
        )
        .into_any_element()
}

/// The empty state's description: nothing left to follow, or nothing
/// matching the filter.
fn empty_text(query: &str) -> (&'static str, String) {
    let query = query.trim();
    if query.is_empty() {
        (
            "Nothing to add",
            "You follow every series of the books on your shelves.".to_owned(),
        )
    } else {
        ("No series found", format!("No series match “{query}”."))
    }
}

impl Render for AddSeriesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_follow = session::has_scope(cx, WRITE_SOCIAL);
        let query = self.filter_input.read(cx).value().to_string();
        let words = search_words(&query);
        let root = div()
            .id("add-series-page")
            .track_focus(&self.focus_handle)
            .size_full();
        let header = self.header(cx).into_any_element();
        let why = (!can_follow).then(|| {
            div()
                .pt_4()
                .body()
                .text_color(colors(cx).fg_muted)
                .child(needs_permission(WRITE_SOCIAL))
        });

        let Some(rows) = self.rows(cx) else {
            let failure = match self.follows.read(cx).status() {
                LoadStatus::Failed(message) => Some(message),
                _ => self.library.read(cx).load_failure().cloned(),
            };
            let body = match failure {
                Some(message) => LoadFailed::new("add-series-retry", message)
                    .on_retry(cx.listener(|this, _, _, cx| {
                        this.follows.update(cx, FollowsModel::load);
                        if !this.library.read(cx).is_loaded() {
                            this.library.update(cx, LibraryModel::sync);
                        }
                    }))
                    .into_any_element(),
                None => div()
                    .pt_6()
                    .child(Sequence::new().children((0..SKELETON_ROWS).map(|_| skeleton_row())))
                    .into_any_element(),
            };
            return root
                .child(column().child(header).children(why).child(body))
                .into_any_element();
        };

        let shown: Vec<&Unfollowed> = rows
            .iter()
            .filter(|s| unfollowed_matches(s, &words))
            .collect();
        if shown.is_empty() {
            let (title, description) = empty_text(&query);
            return root
                .child(
                    column()
                        .child(header)
                        .children(why)
                        .child(
                            div().pt_8().child(
                                EmptyState::new(crate::icons::library(), title)
                                    .description(description),
                            ),
                        ),
                )
                .into_any_element();
        }
        let (started, wanted): (Vec<&Unfollowed>, Vec<&Unfollowed>) =
            shown.into_iter().partition(|s| s.started);
        let started = self.group("Started", &started, can_follow, cx);
        let wanted = self.group("Only on Want to read", &wanted, can_follow, cx);
        root.scroll_y_revealing_focus()
            .child(
                column()
                    .child(header)
                    .children(why)
                    .children(started)
                    .children(wanted),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_state_says_why() {
        assert_eq!(empty_text(" ").0, "Nothing to add");
        assert_eq!(
            empty_text("dune"),
            ("No series found", "No series match “dune”.".to_owned())
        );
    }
}
