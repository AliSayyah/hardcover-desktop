//! The book's header (`hardcover-design`, Shelf → Book pane, and Full book
//! page): the cover beside the info column, 24 apart, and the book's tools.
//!
//! The info column, top to bottom: the title (`display-24` in the pane,
//! `display-30` on the page), the subtitle, the authors (text buttons at
//! 16), other contributors with their roles, the series (a text button),
//! the facts (muted, tabular), Hardcover's average (unless the page's rail
//! shows it), the genres as tag chips, and at the cover's foot the
//! action row: the status menu button and the rating control, 16 apart. A
//! book not in the library gets "Add to library" there instead.
//!
//! A narrow pane wraps the info column under the cover.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, IconName, Side, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, ElementId, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window, div, prelude::FluentBuilder as _, relative,
};
use hardcover_core::auth::scopes::WRITE_LIBRARY;
use hardcover_core::{
    BookDetails, BookSummary, Contributor, Length, LibraryEntry, Rating, ReadingStatus,
};
use std::fmt::Write as _;

use super::BookDetailView;
use crate::app_state::release_cover_image;
use crate::bidi::{WrappingText, is_rtl};
use crate::book::star_rating::StarRating;
use crate::catalog::navigation::Page;
use crate::chip::{TagChip, TagRow, status_icon};
use crate::cover::{Cover, CoverArt, CoverSize, detail_art};
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{
    average_rating_text, count_text, duration_text, needs_permission, series_position_text,
    status_label,
};
use crate::library::model::WriteKind;
use crate::lists::book_lists;
use crate::lists::model::ListsModel;
use crate::session;
use crate::ui::TextButton;
use crate::ui::kit::{KeyboardRing as _, SmallIconButton as _, danger_menu_item};

/// Genres shown in the header. With more, every genre is in the Tags block.
pub(super) const HEADER_GENRES: usize = 6;

impl BookDetailView {
    /// Loads the cover for `url`, releasing the previous cover if it changed.
    pub(super) fn load_cover(
        &mut self,
        url: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CoverArt {
        if self.cover.as_ref().map(|(u, _)| u) != url.as_ref()
            && let Some((old, image)) = self.cover.take()
        {
            release_cover_image(&old, image, window, cx);
        }
        let Some(url) = url else {
            return CoverArt::None;
        };
        let art = detail_art(&url, window, cx);
        self.cover = Some((url, art.image()));
        art
    }

    /// The session may change the library entry.
    pub(super) fn can_write(cx: &App) -> bool {
        session::has_scope(cx, WRITE_LIBRARY)
    }

    /// The header. `with_rail`: the page's rail beside it holds Hardcover's
    /// average, so the header leaves it out.
    pub(super) fn render_header(
        &self,
        book: &BookSummary,
        entry: Option<&LibraryEntry>,
        details: Option<&BookDetails>,
        with_rail: bool,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let page = self.tools_in_bar;
        let muted = colors(cx).fg_muted;
        let art = self.art.clone();
        let size = if page { CoverSize::Xxl } else { CoverSize::Xl };

        let facts = book_facts(book, entry, details);
        let average = (!with_rail)
            .then(|| community_average(book, details))
            .flatten();
        let genres: Vec<String> =
            details.map_or_default(|d| d.genres.iter().take(HEADER_GENRES).cloned().collect());

        // The title's line height grows for Persian and Arabic, whose marks
        // need the room at display sizes.
        let rtl_title = is_rtl(&book.title);
        let title = div()
            .map(|this| {
                if page {
                    this.display_30()
                } else {
                    this.display_24()
                }
            })
            .when(rtl_title, |this| this.line_height(relative(1.5)))
            .child(WrappingText::new(book.title.clone()));

        let text = v_flex()
            .gap_1()
            .child(title)
            .when_some(book.subtitle.clone(), |this, subtitle| {
                this.child(
                    div()
                        .prose()
                        .text_color(muted)
                        .child(WrappingText::new(subtitle)),
                )
            })
            .child(Self::render_contributors(book, details, cx))
            .children(series_line(details))
            .when(!facts.is_empty(), |this| {
                this.child(
                    div()
                        .body()
                        .tabular()
                        .text_color(muted)
                        .child(facts.join(" · ")),
                )
            })
            .when_some(average, |this, average| {
                this.child(
                    h_flex()
                        .gap_1()
                        .body()
                        .tabular()
                        .text_color(muted)
                        .child(crate::star::icon().size_3().text_color(muted))
                        .child(average),
                )
            })
            .when(!genres.is_empty(), |this| {
                this.child(
                    div()
                        .pt_2()
                        .child(TagRow::new(genres.into_iter().map(TagChip::genre))),
                )
            });
        // The action row sits at the cover's foot, or 16 under the info
        // when that runs longer.
        let info = v_flex()
            .flex_1()
            .min_w(size::HEADER_INFO_MIN)
            .justify_between()
            .gap_4()
            .child(text)
            // In a row, so a lone button keeps its own width rather than
            // stretching across the column.
            .child(h_flex().child(match entry {
                Some(entry) => self.render_actions(entry, window, cx),
                None => self.render_add(book, window, cx),
            }));

        h_flex()
            .w_full()
            .flex_wrap()
            .items_stretch()
            .gap_6()
            .child(
                Cover::new(size, book.title.clone())
                    .author(book.authors.first().cloned())
                    .art(art)
                    .shadow(true),
            )
            .child(info)
            .into_any_element()
    }

    /// The authors as text buttons once the details name them, otherwise as
    /// text. Other contributors follow on their own line with their roles.
    fn render_contributors(
        book: &BookSummary,
        details: Option<&BookDetails>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let contributors = details.map_or_default(|d| d.contributors.as_slice());
        let authors: Vec<&Contributor> = contributors.iter().filter(|c| c.is_author()).collect();
        let others: Vec<&Contributor> = contributors.iter().filter(|c| !c.is_author()).collect();
        let byline = |this: gpui_kit::Div| this.byline();
        let authors_line = if authors.is_empty() && !others.is_empty() {
            // Only non-author credits: the roles line below names everyone, and
            // `book.authors` would repeat the same people without their roles.
            None
        } else if authors.is_empty() {
            Some(
                byline(div())
                    .child(WrappingText::new(crate::format::authors_text(
                        &book.authors,
                    )))
                    .into_any_element(),
            )
        } else {
            Some(
                h_flex()
                    .flex_wrap()
                    .map(byline)
                    .children(authors.iter().enumerate().map(|(i, c)| {
                        h_flex()
                            .min_w_0()
                            .child(contributor_name(c, "author", true))
                            .when(i + 1 < authors.len(), |this| {
                                this.child(div().pr_1().child(","))
                            })
                    }))
                    .into_any_element(),
            )
        };
        v_flex()
            .gap_0p5()
            .children(authors_line)
            .when(!others.is_empty(), |this| {
                this.child(
                    h_flex()
                        .flex_wrap()
                        .gap_x_1()
                        .body()
                        .text_color(colors(cx).fg_muted)
                        .children(others.iter().enumerate().map(|(i, c)| {
                            let role = c.role.clone().unwrap_or_default();
                            h_flex()
                                .gap_1()
                                .child(contributor_name(c, "contributor", false))
                                .child(if i + 1 < others.len() {
                                    format!("({role}),")
                                } else {
                                    format!("({role})")
                                })
                        })),
                )
            })
            .into_any_element()
    }

    /// The status menu button, then the rating control (or a book not in the
    /// library: "Add to library").
    fn render_actions(
        &self,
        entry: &LibraryEntry,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let pending = self.model.read(cx).pending_write(entry.user_book_id);
        let busy = pending.is_some();
        let can_write = Self::can_write(cx);
        let status = entry.status;
        // A rating held or saving shows at once.
        let rating = self
            .ratings
            .read(cx)
            .shown(entry.user_book_id)
            .unwrap_or(entry.rating);
        let view = cx.entity().downgrade();
        let trigger = self.status_focus.clone();
        let status_button = Button::new("book-status")
            .keyboard_ring(&self.status_focus, window)
            .icon(status_icon(status))
            .label(status_label(status))
            .dropdown_caret(true)
            .loading(pending == Some(WriteKind::Status))
            .disabled(busy || !can_write)
            .when(!can_write, |this| {
                this.tooltip(needs_permission(WRITE_LIBRARY))
            })
            .dropdown_menu(move |menu, _, _| {
                // The tick trails, so each status keeps its icon.
                let mut menu = menu.check_side(Side::Right);
                for option in ReadingStatus::ALL {
                    let view = view.clone();
                    let trigger = trigger.clone();
                    menu = menu.item(
                        PopupMenuItem::new(status_label(option))
                            .icon(status_icon(option))
                            .checked(option == status)
                            .on_click(move |_, window, cx| {
                                // From the trigger, so a confirmation gives
                                // focus back to it, not to the closed menu.
                                window.focus(&trigger, cx);
                                let _ = view
                                    .update(cx, |this, cx| this.change_status(option, window, cx));
                            }),
                    );
                }
                menu
            });
        // The stars and Clear rating stay enabled while a write runs, so
        // they keep the keyboard focus; a change made meanwhile is held and
        // sent when the write ends (`Ratings`).
        let rate = h_flex()
            .gap_2()
            .child(
                StarRating::new("your-rating")
                    .value(rating)
                    .disabled(!can_write)
                    .when(!can_write, |this| {
                        this.tooltip(needs_permission(WRITE_LIBRARY))
                    })
                    .on_change(cx.listener(|this, rating: &Option<Rating>, window, cx| {
                        this.change_rating(*rating, window, cx);
                    })),
            )
            .map(|this| match rating {
                None if pending != Some(WriteKind::Rating) => {
                    this.child(div().body().text_color(colors(cx).fg_muted).child("Rate"))
                }
                None => this,
                Some(_) => this.child(
                    Button::new("clear-rating")
                        .ghost()
                        .small_icon_button()
                        .icon(IconName::Close)
                        .tooltip(if can_write {
                            "Clear rating".to_owned()
                        } else {
                            needs_permission(WRITE_LIBRARY)
                        })
                        .disabled(!can_write)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.change_rating(None, window, cx)),
                        ),
                ),
            })
            .when(pending == Some(WriteKind::Rating), |this| {
                this.child(saving(cx))
            });
        h_flex()
            .flex_wrap()
            .items_center()
            .gap_4()
            .child(status_button)
            .child(rate)
            .into_any_element()
    }

    /// "Add to library", a menu of the statuses to add the book with.
    fn render_add(&self, book: &BookSummary, window: &Window, cx: &Context<Self>) -> AnyElement {
        let adding = self.model.read(cx).is_adding(book.id);
        let can_write = Self::can_write(cx);
        let view = cx.entity().downgrade();
        Button::new("add-to-library")
            .keyboard_ring(&self.add_focus, window)
            .icon(IconName::Plus)
            .label("Add to library")
            .dropdown_caret(true)
            .loading(adding)
            .disabled(adding || !can_write)
            .when(!can_write, |this| {
                this.tooltip(needs_permission(WRITE_LIBRARY))
            })
            .dropdown_menu(move |mut menu, _, _| {
                for option in ReadingStatus::ALL {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(status_label(option))
                            .icon(status_icon(option))
                            .on_click(move |_, window, cx| {
                                let _ = view
                                    .update(cx, |this, cx| this.add_to_library(option, window, cx));
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    /// The book's tools, for the object bar: the "Lists" menu button and
    /// "more": "Open on Hardcover" for any book with a page there, then, for
    /// a book in the library, its library commands.
    pub(super) fn render_tools(
        &self,
        book: &BookSummary,
        entry: Option<&LibraryEntry>,
        window: &Window,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let mut tools: Vec<AnyElement> = book_lists::lists_button(book, cx).into_iter().collect();
        let url = book.web_url();
        if url.is_none() && entry.is_none() {
            return tools;
        }
        // The library commands, while nothing else on the entry is saving.
        let library = entry.map(|entry| {
            let busy = self
                .model
                .read(cx)
                .pending_write(entry.user_book_id)
                .is_some();
            // With `read:lists` the Owned switch under Details replaces the
            // one-way "Add to Owned list…".
            let one_way_owned = book_lists::offers_one_way_owned(cx);
            let owning = ListsModel::global(cx).is_some_and(|l| l.read(cx).is_busy(entry.book.id));
            let has_edition = entry.edition.is_some();
            (entry.user_book_id, has_edition, busy, one_way_owned, owning)
        });
        let can_write = Self::can_write(cx);
        let view = cx.entity().downgrade();
        let model = self.model.clone();
        let trigger = self.more_focus.clone();
        tools.push(
            Button::new("book-more")
                .ghost()
                .icon(IconName::Ellipsis)
                .tooltip("More actions")
                .keyboard_ring(&self.more_focus, window)
                .dropdown_menu(move |menu, _, cx| {
                    let menu = match url.clone() {
                        Some(url) => menu.item(
                            PopupMenuItem::new("Open on Hardcover")
                                .icon(IconName::ExternalLink)
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        ),
                        None => menu,
                    };
                    let Some((id, has_edition, busy, one_way_owned, owning)) = library else {
                        return menu;
                    };
                    let remove_view = view.clone();
                    // Each command focuses the trigger first, so the
                    // confirmation it opens gives focus back to it.
                    let (owned_trigger, remove_trigger) = (trigger.clone(), trigger.clone());
                    let menu = if url.is_some() {
                        menu.separator()
                    } else {
                        menu
                    };
                    let menu = if one_way_owned {
                        let model = model.clone();
                        menu.item(
                            PopupMenuItem::new("Add to Owned list…")
                                .disabled(!has_edition || owning || !can_write || busy)
                                .on_click(move |_, window, cx| {
                                    window.focus(&owned_trigger, cx);
                                    let entry = model.read(cx).entry(id).cloned();
                                    if let Some(entry) = entry {
                                        book_lists::confirm_owned(&entry, window, cx);
                                    }
                                }),
                        )
                        // Ownership is per edition on Hardcover. Without
                        // write:library the last row says why, for both.
                        .when(can_write && !has_edition, |menu| {
                            menu.item(reason_row("Choose an edition first"))
                        })
                        .separator()
                    } else {
                        menu
                    };
                    // The destructive command, in danger and set apart.
                    menu.item(
                        danger_menu_item("Remove from library…", can_write && !busy, cx).on_click(
                            move |_, window, cx| {
                                window.focus(&remove_trigger, cx);
                                let _ = remove_view
                                    .update(cx, |this, cx| this.confirm_remove(window, cx));
                            },
                        ),
                    )
                    .when(!can_write, |menu| {
                        menu.item(reason_row(needs_permission(WRITE_LIBRARY)))
                    })
                })
                .into_any_element(),
        );
        tools
    }
}

fn book_facts(
    book: &BookSummary,
    entry: Option<&LibraryEntry>,
    details: Option<&BookDetails>,
) -> Vec<String> {
    let mut facts: Vec<String> = Vec::new();
    if let Some(year) = book
        .release_year
        .or_else(|| details.and_then(|d| d.release_date.map(|d| chrono::Datelike::year(&d))))
    {
        facts.push(year.to_string());
    }
    // The entry's edition decides; an audiobook whose running time is
    // unknown shows no length rather than the book's pages.
    let length = match entry.and_then(LibraryEntry::measure) {
        Some(measure) => measure.length(),
        None => book.pages.map(Length::Pages),
    };
    match length {
        Some(Length::Pages(pages)) => facts.push(format!("{} pages", count_text(pages))),
        Some(Length::Seconds(seconds)) => facts.push(duration_text(seconds)),
        None => {}
    }
    facts
}

fn community_average(book: &BookSummary, details: Option<&BookDetails>) -> Option<String> {
    book.average_rating
        .or_else(|| details.and_then(|d| d.summary.average_rating))
        .filter(|a| *a > 0.0)
        .map(|average| {
            let mut text = format!("{} average", average_rating_text(average));
            if let Some(n) = details.and_then(|d| d.ratings_count).filter(|n| *n > 0) {
                let unit = if n == 1 { "rating" } else { "ratings" };
                write!(text, " · {} {unit}", count_text(n)).expect("writing to a String");
            }
            text
        })
}

/// The series and the book's place in it, opening the series page when
/// Hardcover gave its id.
fn series_line(details: Option<&BookDetails>) -> Option<AnyElement> {
    let series = details?.series.first()?;
    let label = match series.position {
        Some(p) => format!("{} {}", series.name, series_position_text(p)),
        None => series.name.clone(),
    };
    Some(match series.id {
        Some(id) => TextButton::new("open-series", label)
            .opens(Page::Series(id))
            .into_any_element(),
        None => div()
            .body()
            .min_w_0()
            .child(WrappingText::new(label))
            .into_any_element(),
    })
}

/// A contributor's name: a text button to their author page when the id is
/// known. In the byline (Inter 500 at 16) or in the muted roles line (body).
/// The button's id comes from the author's id and role, `kind` telling the
/// byline's authors from the other contributors, so it stays put as the
/// list changes and one person in two roles gets two.
fn contributor_name(c: &Contributor, kind: &'static str, byline: bool) -> AnyElement {
    fn style<S: gpui_kit::Styled>(this: S, byline: bool) -> S {
        if byline { this.byline() } else { this.body() }
    }
    match c.author_id {
        Some(author) => style(
            TextButton::new(
                ElementId::Name(
                    format!(
                        "{kind}-{}-{}",
                        author.get(),
                        c.role.as_deref().unwrap_or_default()
                    )
                    .into(),
                ),
                c.name.clone(),
            )
            .opens(Page::Author(author)),
            byline,
        )
        .into_any_element(),
        None => style(div().min_w_0(), byline)
            .child(WrappingText::new(c.name.clone()))
            .into_any_element(),
    }
}

/// A menu row saying why the items above it are disabled: muted, and
/// wrapped at [`size::MENU_REASON`], since the kit's menu labels don't wrap.
fn reason_row(reason: impl Into<SharedString>) -> PopupMenuItem {
    let reason = reason.into();
    PopupMenuItem::element(move |_, _| div().w(size::MENU_REASON).child(reason.clone()))
        .disabled(true)
}

/// "Saving…" beside a control whose write is in flight.
pub(super) fn saving(cx: &App) -> impl IntoElement {
    div().caption(cx).child(SharedString::new_static("Saving…"))
}
