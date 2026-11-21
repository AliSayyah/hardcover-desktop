//! An author page: photo, bio and their most-read books.
//!
//! The header puts the author's photo where a book has its cover, a large
//! `ui::Avatar` (initials on `accent-soft` without one), beside their name
//! in `display-24`, the facts, how many of their books the reader has read
//! and "Open on Hardcover". The bio follows in `prose`, then "Books" with
//! the order, and the books as catalogue rows.
//!
//! The photo, name and bio scroll on their own in a short window
//! (`book_list::page_header_scrolling`); "Books" and the list stay, with
//! room for at least three rows.

use std::collections::HashMap;
use std::sync::Arc;

use gpui_kit::ScrollHandle;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::list::ListState;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::{IndexPath, h_flex, v_flex};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Task, Window, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::{AuthorBook, AuthorDetails, AuthorId};

use super::book_list::{
    CatalogBooks, CatalogRow, ListPane, focus_list_if_fallback_is_focused, follow_list,
    load_failed, new_list, page_body, page_header, page_header_scrolling, page_list, read_summary,
    replace_rows, reset_scroll, row_details,
};
use crate::bidi::WrappingText;
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use crate::freshness::Fetched;
use crate::library::model::LibraryModel;
use crate::session::{self, SessionScope};
use crate::ui::kit::OnTextSpine as _;
use crate::ui::kit::RaisedField as _;
use crate::ui::kit::SmallControl as _;
use crate::ui::{Avatar, AvatarSize, ExternalLink, SectionHeader, Skeleton};

/// How an author's books are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BookOrder {
    #[default]
    MostRead,
    Newest,
    Oldest,
}

impl BookOrder {
    const ALL: [BookOrder; 3] = [BookOrder::MostRead, BookOrder::Newest, BookOrder::Oldest];

    fn label(self) -> &'static str {
        match self {
            BookOrder::MostRead => "Most read",
            BookOrder::Newest => "Newest",
            BookOrder::Oldest => "Oldest",
        }
    }
}

#[derive(Clone)]
pub struct OrderOption(BookOrder, SharedString);

impl SelectItem for OrderOption {
    type Value = BookOrder;

    fn title(&self) -> SharedString {
        self.1.clone()
    }

    fn value(&self) -> &BookOrder {
        &self.0
    }
}

/// `books` (most read first, as fetched) in `order`. Books without a year go
/// last when ordering by year; ties keep the most-read first.
fn ordered(books: &[AuthorBook], order: BookOrder) -> Vec<&AuthorBook> {
    let mut out: Vec<&AuthorBook> = books.iter().collect();
    match order {
        BookOrder::MostRead => {}
        BookOrder::Newest => {
            out.sort_by_key(|b| std::cmp::Reverse(b.book.release_year.unwrap_or(i32::MIN)));
        }
        BookOrder::Oldest => out.sort_by_key(|b| b.book.release_year.unwrap_or(i32::MAX)),
    }
    out
}

/// "1920–1986", "Born 1947", or nothing.
fn life_years(author: &AuthorDetails) -> Option<String> {
    match (author.born_year, author.death_year) {
        (Some(b), Some(d)) => Some(format!("{b}–{d}")),
        (Some(b), None) => Some(format!("Born {b}")),
        (None, Some(d)) => Some(format!("Died {d}")),
        (None, None) => None,
    }
}

/// Bios longer than this start collapsed.
const LONG_BIO: usize = 400;

enum Loaded {
    /// Kept for the run, fetched again when stale ([`crate::freshness`]).
    Ready(Fetched<Arc<AuthorDetails>>),
    Failed(SharedString),
}

pub struct AuthorView {
    model: Entity<LibraryModel>,
    current: Option<AuthorId>,
    /// Pages fetched this session, so Back costs no request.
    pages: HashMap<AuthorId, Loaded>,
    loading: Option<AuthorId>,
    task: Option<Task<()>>,
    /// Holds focus while the page loads; see [`Self::focus`].
    focus_when_ready: Option<FocusHandle>,
    order: BookOrder,
    order_select: Entity<SelectState<Vec<OrderOption>>>,
    bio_expanded: bool,
    list: Entity<ListState<CatalogBooks>>,
    /// The selected book, beside the list when the window is wide.
    pane: ListPane,
    /// The author and order the list holds.
    listed: Option<(AuthorId, BookOrder)>,
    /// The header's scroll, when it gives way in a short window.
    header_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl AuthorView {
    pub fn new(model: Entity<LibraryModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let pane = ListPane::new(model.clone(), window, cx);
        let list = new_list(
            model.clone(),
            "Hardcover lists no books by this author.",
            &pane,
            window,
            cx,
        );
        let options: Vec<OrderOption> = BookOrder::ALL
            .into_iter()
            .map(|o| OrderOption(o, o.label().into()))
            .collect();
        let order_select =
            cx.new(|cx| SelectState::new(options, Some(IndexPath::new(0)), window, cx));
        let mut subscriptions = follow_list(&list, &model, &pane, window, cx);
        subscriptions.push(cx.subscribe_in(
            &order_select,
            window,
            |this, _, event: &SelectEvent<Vec<OrderOption>>, window, cx| {
                if let SelectEvent::Confirm(Some(order)) = event {
                    this.order = *order;
                    this.fill_list(window, cx);
                }
            },
        ));
        Self {
            model,
            current: None,
            pages: HashMap::new(),
            loading: None,
            task: None,
            focus_when_ready: None,
            order: BookOrder::default(),
            order_select,
            bio_expanded: false,
            list,
            pane,
            listed: None,
            header_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn show(&mut self, id: AuthorId, window: &mut Window, cx: &mut Context<Self>) {
        if self.current != Some(id) {
            self.bio_expanded = false;
        }
        if self.current != Some(id) {
            reset_scroll(&self.header_scroll);
        }
        self.current = Some(id);
        self.load(window, cx);
        self.fill_list(window, cx);
        cx.notify();
    }

    /// Focuses the book list, which renders only once the page has loaded.
    /// Until then `fallback` holds focus, so Escape and Back keep working, and
    /// the list takes over when the page arrives if `fallback` still has it.
    pub fn focus(&mut self, fallback: &FocusHandle, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_ready() {
            self.focus_when_ready = None;
            crate::ui::enter_list(&self.list, window, cx);
        } else {
            self.focus_when_ready = Some(fallback.clone());
            window.focus(fallback, cx);
        }
    }

    fn is_ready(&self) -> bool {
        self.current
            .is_some_and(|id| matches!(self.pages.get(&id), Some(Loaded::Ready(_))))
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.current else { return };
        let revalidating = match self.pages.get(&id) {
            Some(Loaded::Ready(fetched)) if !fetched.is_stale(cx) => return,
            Some(Loaded::Ready(_)) => true,
            Some(Loaded::Failed(_)) | None => false,
        };
        if self.loading == Some(id) {
            return;
        }
        // A stale page stays on screen while it is fetched again.
        if !revalidating {
            self.pages.remove(&id);
        }
        self.loading = Some(id);
        let Some(session) = SessionScope::current(cx) else {
            return;
        };
        let page = session.interactive(move |client| client.author(id));
        // Replacing the task drops the fetch for a page no longer shown.
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let Some(result) = page.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.loading == Some(id) {
                    this.loading = None;
                }
                let result = result.map(Arc::new);
                let changed = match (this.pages.get_mut(&id), result) {
                    // A refetch: a failure keeps the page shown, and only
                    // records a refused scope.
                    (Some(Loaded::Ready(fetched)), result) => {
                        if let Err(e) = &result {
                            session::error_message(e, Some(READ_CATALOG), cx);
                        }
                        fetched.refetched(result, cx)
                    }
                    (_, Ok(page)) => {
                        this.pages.insert(id, Loaded::Ready(Fetched::new(page, cx)));
                        false
                    }
                    (_, Err(e)) => {
                        let message = session::error_message(&e, Some(READ_CATALOG), cx);
                        this.pages.insert(id, Loaded::Failed(message.into()));
                        false
                    }
                };
                if changed
                    && this.listed.is_some_and(|(listed, _)| listed == id)
                    && let Some(rows) = this.rows(id)
                {
                    replace_rows(&this.list, rows, window, cx);
                }
                this.fill_list(window, cx);
                if this.is_ready() {
                    focus_list_if_fallback_is_focused(
                        &this.list,
                        this.focus_when_ready.take(),
                        window,
                        cx,
                    );
                }
                cx.notify();
            });
        }));
    }

    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.loading = None;
        self.load(window, cx);
        cx.notify();
    }

    fn fill_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.current else { return };
        if self.listed == Some((id, self.order)) {
            return;
        }
        let Some(rows) = self.rows(id) else {
            return;
        };
        self.listed = Some((id, self.order));
        self.list.update(cx, |list, cx| {
            list.delegate_mut().set_rows(rows);
            list.set_selected_index(None, window, cx);
            cx.notify();
        });
    }

    /// The list's rows for a loaded page, in the order chosen.
    fn rows(&self, id: AuthorId) -> Option<Vec<CatalogRow>> {
        let Some(Loaded::Ready(author)) = self.pages.get(&id) else {
            return None;
        };
        let rows = ordered(&author.value().books, self.order)
            .into_iter()
            .map(|b| CatalogRow {
                details: row_details(&[b.role.clone().unwrap_or_default()], &b.book),
                book: b.book.clone(),
            })
            .collect();
        Some(rows)
    }

    /// Fetches the page again if it is stale ([`crate::freshness`]), and
    /// the book in the pane beside the list.
    /// A failed page is tried again only with `shown_again`.
    pub fn revalidate(&mut self, shown_again: bool, window: &mut Window, cx: &mut Context<Self>) {
        let failed = self
            .current
            .is_some_and(|id| matches!(self.pages.get(&id), Some(Loaded::Failed(_))));
        if shown_again || !failed {
            self.load(window, cx);
        }
        self.pane.revalidate(shown_again, cx);
    }

    fn render_header(&self, author: &AuthorDetails, cx: &mut Context<Self>) -> AnyElement {
        let muted = colors(cx).fg_muted;
        let model = self.model.read(cx);
        let mut facts: Vec<String> = Vec::new();
        if let Some(years) = life_years(author) {
            facts.push(years);
        }
        match author.books_count.filter(|n| *n > 0) {
            Some(1) => facts.push("1 book on Hardcover".into()),
            Some(n) => facts.push(format!(
                "{} books on Hardcover",
                crate::format::count_text(n)
            )),
            None => {}
        }
        let summary = read_summary(
            author
                .books
                .iter()
                .filter(|b| b.role.is_none())
                .map(|b| model.entry_for_book(b.book.id).map(|e| e.status)),
        );
        let long_bio = author
            .bio
            .as_ref()
            .is_some_and(|b| b.chars().count() > LONG_BIO);
        v_flex()
            .gap_6()
            .child(
                h_flex()
                    .gap_6()
                    .items_start()
                    .child(
                        // Scaled on the cover-decoder thread like a row
                        // thumbnail, not decoded full size on the shared pool.
                        Avatar::new(author.name.clone())
                            .size(AvatarSize::Xl)
                            .image(author.photo.as_ref().map(|p| p.url.clone())),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .display_24()
                                    .child(WrappingText::new(author.name.clone())),
                            )
                            .when(!facts.is_empty(), |this| {
                                this.child(
                                    div()
                                        .body()
                                        .tabular()
                                        .text_color(muted)
                                        .child(facts.join(" · ")),
                                )
                            })
                            .when_some(summary, |this, summary| {
                                this.child(div().body().tabular().child(summary))
                            })
                            .when_some(author.web_url(), |this, url| {
                                this.child(div().pt_3().child(ExternalLink::new(
                                    "author-on-hardcover",
                                    "Open on Hardcover",
                                    url,
                                )))
                            }),
                    ),
            )
            .when_some(author.bio.clone(), |this, bio| {
                let collapsed = long_bio && !self.bio_expanded;
                this.child(
                    v_flex()
                        .gap_2()
                        .items_start()
                        .child(div().max_w(size::PROSE).prose().child({
                            let bio = WrappingText::new(bio);
                            if collapsed { bio.line_clamp(4) } else { bio }
                        }))
                        .when(long_bio, |this| {
                            this.child(
                                Button::new("toggle-bio")
                                    .ghost()
                                    .small_control()
                                    // The label on the bio's spine, not the
                                    // button's edge.
                                    .on_text_spine(true)
                                    .label(if self.bio_expanded {
                                        "Show less"
                                    } else {
                                        "Read more"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.bio_expanded = !this.bio_expanded;
                                        cx.notify();
                                    })),
                            )
                        }),
                )
            })
            .into_any_element()
    }

    /// "Books" and the order, over the list: 24 under the header (whose own
    /// 16 of foot this completes).
    fn render_books_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        page_header().pt_2().child(
            SectionHeader::new("Books").action(
                div().w(size::SELECT_SMALL).child(
                    Select::new(&self.order_select)
                        .small_control()
                        .raised_field(false, cx),
                ),
            ),
        )
    }
}

impl AuthorView {
    /// The name of a page already loaded this session, for Back's label.
    pub fn name(&self, id: AuthorId) -> Option<&str> {
        match self.pages.get(&id)? {
            Loaded::Ready(details) => Some(&details.value().name),
            Loaded::Failed(_) => None,
        }
    }
}

impl Render for AuthorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.current.and_then(|id| match self.pages.get(&id) {
            Some(Loaded::Ready(author)) => Some(Ok(author.value().clone())),
            Some(Loaded::Failed(message)) => Some(Err(message.clone())),
            None => None,
        });
        let ready = matches!(current, Some(Ok(_)));
        let header = match current {
            Some(Ok(author)) => self.render_header(&author, cx),
            Some(Err(message)) => load_failed(
                message,
                Button::new("retry-author")
                    .small_control()
                    .label("Try again")
                    .on_click(cx.listener(|this, _, window, cx| this.retry(window, cx))),
                cx,
            )
            .into_any_element(),
            None => h_flex()
                .gap_6()
                .items_start()
                .child(Skeleton::circle().size(size::AVATAR_LARGE))
                .child(
                    v_flex()
                        .gap_2()
                        .child(Skeleton::line_in(TypeRole::Display24).w_64())
                        .child(Skeleton::line().w_48())
                        .child(Skeleton::line().w_32()),
                )
                .into_any_element(),
        };
        let column = v_flex().size_full().min_w_0().map(|this| {
            if ready {
                // The header scrolls on its own when the window is short;
                // "Books" and the list keep their room.
                this.child(page_header_scrolling(
                    "author-header",
                    &self.header_scroll,
                    header,
                ))
                .child(self.render_books_bar(cx))
                .child(page_body().child(page_list(&self.list)))
            } else {
                this.child(page_header().child(header))
            }
        });
        self.pane
            .layout(&self.list, CatalogBooks::book_at, column, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::{BookId, BookSummary};

    fn book(id: i64, year: Option<i32>) -> AuthorBook {
        let mut b = BookSummary::new(BookId::new(id), format!("Book {id}"));
        b.release_year = year;
        AuthorBook::new(b)
    }

    #[test]
    fn orders_by_year_with_unknown_years_last() {
        let books = vec![
            book(1, Some(1990)),
            book(2, None),
            book(3, Some(2001)),
            book(4, Some(1990)),
        ];
        let ids = |order| -> Vec<i64> {
            ordered(&books, order)
                .iter()
                .map(|b| b.book.id.get())
                .collect()
        };
        assert_eq!(ids(BookOrder::MostRead), vec![1, 2, 3, 4]);
        assert_eq!(ids(BookOrder::Newest), vec![3, 1, 4, 2]);
        assert_eq!(ids(BookOrder::Oldest), vec![1, 4, 3, 2]);
    }

    #[test]
    fn life_years_text() {
        let mut a = AuthorDetails::new(AuthorId::new(1), "A. Writer");
        assert_eq!(life_years(&a), None);
        a.born_year = Some(1920);
        assert_eq!(life_years(&a).as_deref(), Some("Born 1920"));
        a.death_year = Some(1986);
        assert_eq!(life_years(&a).as_deref(), Some("1920–1986"));
    }
}
