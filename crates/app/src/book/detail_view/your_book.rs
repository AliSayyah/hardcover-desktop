//! Your book: the parts of the pane and page about the reader's own copy
//! (`hardcover-design`, Shelf → Book pane, and Full book page).
//!
//! - The one panel under the header: progress (Currently reading, Paused)
//!   or Not started (Want to read).
//! - Details: a property list of Edition, Owned, Privacy and Added.
//! - Reads: every read of the book, each with its menu, and "Add a past
//!   read…".
//!
//! Each is a block in the pane and a rail section on the page. Notes, beside
//! them, are `notes.rs`.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::select::Select;
use gpui_kit::component::{Disableable as _, Icon, IconName, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, ElementId, IntoElement, ParentElement as _, SharedString, Styled as _,
    Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::WRITE_LIBRARY;
use hardcover_core::{Date, LibraryEntry, ReadSession, ReadingStatus};

use super::BookDetailView;
use super::header::saving;
use crate::book::progress_dialog::open_progress;
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{
    date_text, edition_text, needs_permission, progress_summary, read_dates_text, short_position,
};
use crate::library::model::WriteKind;
use crate::lists::book_lists;
use crate::ui::kit::RaisedField as _;
use crate::ui::kit::{
    KeyboardRing as _, SmallControl as _, SmallIconButton as _, danger_menu_item,
};
use crate::ui::{Block, Meter, PropertyList, RailSection, panel};

impl BookDetailView {
    /// The panel under the header, if the entry's status has one.
    pub(super) fn render_panel(
        &self,
        entry: &LibraryEntry,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        match entry.status {
            ReadingStatus::CurrentlyReading | ReadingStatus::Paused => {
                Some(self.render_progress(entry, window, cx))
            }
            ReadingStatus::WantToRead => Some(self.render_not_started(entry, window, cx)),
            _ => None,
        }
    }

    /// Where the reader is: the position and percentage over a meter, then
    /// when the read started and its pace, and "Update progress…".
    fn render_progress(
        &self,
        entry: &LibraryEntry,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let pending = self.model.read(cx).pending_write(entry.user_book_id);
        let progress = progress_summary(entry);
        let muted = colors(cx).fg_muted;
        let can_write = Self::can_write(cx);
        // A progress update with no open read starts one, and moves the book
        // to Currently reading: not something a paused book's panel should
        // do behind the reader's back.
        let open = entry.open_read().is_some() || entry.status == ReadingStatus::CurrentlyReading;
        let today = crate::dates::today();
        let meta = pace_line(entry, today);
        let model = self.model.clone();
        let id = entry.user_book_id;
        panel(cx)
            .flex()
            .flex_col()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .body_strong()
                            .tabular()
                            .child(progress.text()),
                    )
                    .when_some(progress.percent, |this, percent| {
                        this.child(
                            div()
                                .body()
                                .tabular()
                                .text_color(muted)
                                .child(format!("{}%", percent.round().max(0.0))),
                        )
                    }),
            )
            .when_some(progress.percent, |this, percent| {
                this.child(Meter::new(
                    ("reading-progress", entry.user_book_id.get().cast_unsigned()),
                    percent,
                    "Reading progress",
                ))
            })
            .child(
                h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .body()
                            .tabular()
                            .text_color(muted)
                            .children(meta),
                    )
                    .when(open, |this| {
                        this.child(
                            Button::new("update-progress")
                                .small_control()
                                .label("Update progress…")
                                .keyboard_ring(&self.panel_focus, window)
                                .loading(pending == Some(WriteKind::Progress))
                                .disabled(pending.is_some() || !can_write)
                                .when(!can_write, |this| {
                                    this.tooltip(needs_permission(WRITE_LIBRARY))
                                })
                                .on_click(move |_, window, cx| {
                                    let entry = model.read(cx).entry(id).cloned();
                                    if let Some(entry) = entry {
                                        open_progress(model.clone(), &entry, window, cx);
                                    }
                                }),
                        )
                    }),
            )
            .into_any_element()
    }

    /// A book on Want to read: what starting it does, and "Start reading".
    /// Starting moves focus, if on "Start reading", to "Update progress…",
    /// which takes its place.
    fn render_not_started(
        &self,
        entry: &LibraryEntry,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let pending = self.model.read(cx).pending_write(entry.user_book_id);
        let can_write = Self::can_write(cx);
        panel(cx)
            .flex()
            .items_center()
            .gap_3()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().body_strong().child("Not started"))
                    .child(
                        div()
                            .body()
                            .text_color(colors(cx).fg_muted)
                            .child("Starting moves it to Currently reading."),
                    ),
            )
            .child(
                Button::new("start-reading")
                    .label("Start reading")
                    .keyboard_ring(&self.panel_focus, window)
                    .loading(pending == Some(WriteKind::Status))
                    .disabled(pending.is_some() || !can_write)
                    .when(!can_write, |this| {
                        this.tooltip(needs_permission(WRITE_LIBRARY))
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.change_status(ReadingStatus::CurrentlyReading, window, cx);
                    })),
            )
            .into_any_element()
    }

    /// Details: the edition, Owned, privacy and when it was added.
    pub(super) fn render_details(
        &self,
        entry: &LibraryEntry,
        in_rail: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let pending = self.model.read(cx).pending_write(entry.user_book_id);
        let busy = pending.is_some();
        let can_write = Self::can_write(cx);
        let muted = colors(cx).fg_muted;
        let (edition, chosen) = match &entry.edition {
            Some(edition) => (
                edition_text(edition)
                    .or_else(|| edition.title.clone())
                    .unwrap_or_else(|| "Edition chosen".to_owned()),
                true,
            ),
            None => ("Not chosen".to_owned(), false),
        };
        let edition_value = h_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .when(!chosen, |this| this.text_color(muted))
                    .child(edition),
            )
            .child(
                Button::new("change-edition")
                    .small_control()
                    .label(if chosen { "Change…" } else { "Choose…" })
                    .loading(pending == Some(WriteKind::Edition))
                    .disabled(busy || !can_write)
                    .when(!can_write, |this| {
                        this.tooltip(needs_permission(WRITE_LIBRARY))
                    })
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_edition_picker(window, cx)),
                    ),
            );
        let privacy_value = h_flex()
            .flex_wrap()
            .gap_x_2()
            .child(
                div().w(size::SELECT_SMALL).child(
                    Select::new(&self.privacy)
                        .small_control()
                        .placeholder("Unknown")
                        .disabled(busy || !can_write)
                        .raised_field(busy || !can_write, cx),
                ),
            )
            .when(pending == Some(WriteKind::Privacy), |this| {
                this.child(saving(cx))
            });
        let list = PropertyList::new()
            .in_rail(in_rail)
            .row("Edition", edition_value)
            .when_some(book_lists::owned_control(entry, cx), |this, owned| {
                this.row("Owned", owned)
            })
            .row("Privacy", privacy_value)
            .when_some(entry.date_added, |this, added| {
                this.row("Added", div().tabular().child(date_text(added)))
            });
        if in_rail {
            RailSection::new("Details").child(list).into_any_element()
        } else {
            Block::new("Details").child(list).into_any_element()
        }
    }

    /// Every read of the book: the current one first, then the others, each
    /// with its menu; "Add a past read…" on the heading.
    pub(super) fn render_reads(
        &self,
        entry: &LibraryEntry,
        in_rail: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let pending = self.model.read(cx).pending_write(entry.user_book_id);
        let can_write = Self::can_write(cx);
        let current = entry.current_read();
        let progress = progress_summary(entry);
        let mut reads: Vec<&ReadSession> = current.into_iter().collect();
        reads.extend(
            entry
                .reads
                .iter()
                .filter(|read| Some(read.id) != current.map(|c| c.id)),
        );
        let rows: Vec<AnyElement> = reads
            .into_iter()
            .map(|read| {
                let is_current = Some(read.id) == current.map(|c| c.id) && !read.is_finished();
                let (title, mut facts) = if is_current {
                    let mut facts = vec![format!("{} – now", start_text(read.started_at))];
                    facts.extend(short_position(&progress));
                    ("Current read".to_owned(), facts)
                } else {
                    let mut facts = Vec::new();
                    if !read.is_finished() {
                        facts.push("Not finished".to_owned());
                    }
                    (read_dates_text(read.started_at, read.finished_at), facts)
                };
                facts.extend(read.edition.as_ref().and_then(edition_text));
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div().pt_0p5().child(
                            Icon::new(if read.is_finished() {
                                IconName::CircleCheck
                            } else {
                                IconName::BookOpen
                            })
                            .size_4()
                            .text_color(colors(cx).fg_muted),
                        ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().body().tabular().child(title))
                            .when(!facts.is_empty(), |this| {
                                this.child(div().caption(cx).tabular().child(facts.join(" · ")))
                            }),
                    )
                    .child(self.read_menu(read, pending.is_some() || !can_write, window, cx))
                    .into_any_element()
            })
            .collect();
        let add = Button::new("add-read")
            .ghost()
            .small_control()
            .icon(IconName::Plus)
            .label("Add a past read…")
            .keyboard_ring(&self.add_read_focus, window)
            .loading(pending == Some(WriteKind::Reads))
            .disabled(pending.is_some() || !can_write)
            .when(!can_write, |this| {
                this.tooltip(needs_permission(WRITE_LIBRARY))
            })
            .on_click(cx.listener(|this, _, window, cx| this.open_read_dialog(None, window, cx)));
        let body = v_flex()
            .gap_3()
            .when(rows.is_empty(), |this| {
                this.child(
                    div()
                        .body()
                        .text_color(colors(cx).fg_muted)
                        .child("No reads recorded"),
                )
            })
            .children(rows);
        if in_rail {
            RailSection::new("Reads")
                .action(add)
                .child(body)
                .into_any_element()
        } else {
            Block::new("Reads")
                .action(add)
                .child(body)
                .into_any_element()
        }
    }

    /// A read's actions: changing its dates and deleting it.
    /// Its commands focus the trigger first, so the dialog they open gives
    /// focus back to it, not to the closed menu.
    fn read_menu(
        &self,
        read: &ReadSession,
        disabled: bool,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().downgrade();
        let session = read.clone();
        let read_id = read.id;
        let trigger = self
            .read_focus
            .borrow_mut()
            .entry(read_id)
            .or_insert_with(|| cx.focus_handle())
            .clone();
        Button::new(ElementId::Name(format!("read-{}", read.id).into()))
            .ghost()
            .small_icon_button()
            .icon(IconName::Ellipsis)
            .tooltip("Read actions")
            .keyboard_ring(&trigger, window)
            .disabled(disabled)
            .dropdown_menu(move |menu, _, cx| {
                let edit_view = view.clone();
                let delete_view = view.clone();
                let session = session.clone();
                let (edit_trigger, delete_trigger) = (trigger.clone(), trigger.clone());
                menu.item(
                    PopupMenuItem::new("Change dates…")
                        .icon(IconName::Calendar)
                        .on_click(move |_, window, cx| {
                            window.focus(&edit_trigger, cx);
                            let _ = edit_view.update(cx, |this, cx| {
                                this.open_read_dialog(Some(&session), window, cx);
                            });
                        }),
                )
                .item(
                    danger_menu_item("Delete read…", true, cx).on_click(move |_, window, cx| {
                        window.focus(&delete_trigger, cx);
                        let _ = delete_view.update(cx, |this, cx| {
                            this.confirm_delete_read(read_id, window, cx);
                        });
                    }),
                )
            })
    }
}

/// The muted line under the meter: "Started Nov 1 · 16 pages a day · done
/// around Dec 1", as much of it as is known. A paused book has no pace and
/// no finish date: only when it started.
fn pace_line(entry: &LibraryEntry, today: Date) -> Option<SharedString> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(started) = entry.open_read().and_then(|r| r.started_at) {
        parts.push(format!("Started {}", short_date(started, today)));
    }
    let pace = (entry.status != ReadingStatus::Paused)
        .then(|| crate::home::digest::pace(entry, today))
        .flatten();
    if let Some(pace) = pace {
        parts.push(crate::format::pages_per_day_text(
            pace.pages_per_day,
            "under a page a day",
        ));
        if let Some(finish) = pace.finish {
            parts.push(format!("done around {}", short_date(finish, today)));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" · ").into())
}

/// "Nov 30", with the year when it isn't this year's.
fn short_date(date: Date, today: Date) -> String {
    use chrono::Datelike as _;
    if date.year() == today.year() {
        date.format("%b %-d").to_string()
    } else {
        date_text(date)
    }
}

/// "Oct 20, 2026", or "No start date".
fn start_text(started: Option<Date>) -> String {
    started.map_or_else(|| "No start date".to_owned(), date_text)
}

/// The paragraphs of a reader's text, without blank lines.
pub(super) fn paragraphs(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split('\n')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_owned)
}

/// "Saved", with a check, after a save the result of which is otherwise
/// visible only as the text it already shows.
pub(super) fn saved_note(cx: &gpui_kit::App) -> impl IntoElement {
    h_flex()
        .gap_1()
        .caption(cx)
        .child(Icon::new(IconName::Check).size_3())
        .child("Saved")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::ProgressSummary;

    fn progress(page: Option<u32>, total: Option<u32>, seconds: Option<u32>) -> ProgressSummary {
        ProgressSummary {
            page,
            total_pages: total,
            seconds,
            total_seconds: seconds.map(|_| 36_000),
            percent: None,
            audio: seconds.is_some(),
        }
    }

    #[test]
    fn the_position_separates_thousands() {
        assert_eq!(
            progress(Some(418), Some(1007), None).text(),
            "Page 418 of 1,007"
        );
        assert_eq!(progress(Some(12), None, None).text(), "Page 12");
        assert_eq!(
            progress(None, None, None).text(),
            "Not started",
            "nothing read yet"
        );
        assert_eq!(
            short_position(&progress(Some(1200), None, None)).unwrap(),
            "page 1,200"
        );
        assert_eq!(short_position(&progress(None, None, None)), None);
    }

    #[test]
    fn dates_carry_the_year_only_when_it_isnt_this_one() {
        let today = Date::from_ymd_opt(2026, 11, 20).unwrap();
        assert_eq!(
            short_date(Date::from_ymd_opt(2026, 12, 1).unwrap(), today),
            "Dec 1"
        );
        assert_eq!(
            short_date(Date::from_ymd_opt(2027, 1, 5).unwrap(), today),
            "Jan 5, 2027"
        );
    }

    #[test]
    fn a_paused_book_has_no_pace() {
        use hardcover_core::{BookId, BookSummary, ReadId, UserBookId, UserId};
        let today = Date::from_ymd_opt(2026, 11, 20).unwrap();
        let mut book = BookSummary::new(BookId::new(1), "Book");
        book.pages = Some(476);
        let mut entry = LibraryEntry::new(
            UserBookId::new(1),
            UserId::new(1),
            book,
            ReadingStatus::CurrentlyReading,
        );
        let mut read = ReadSession::new(ReadId::new(1));
        read.started_at = Date::from_ymd_opt(2026, 11, 1);
        read.progress_pages = Some(312);
        entry.reads.push(read);
        assert_eq!(
            pace_line(&entry, today).unwrap(),
            "Started Nov 1 · 16 pages a day · done around Dec 1"
        );
        entry.status = ReadingStatus::Paused;
        assert_eq!(pace_line(&entry, today).unwrap(), "Started Nov 1");
    }

    #[test]
    fn paragraphs_drop_blank_lines() {
        let text = "First.\n\n  Second.  \n";
        assert_eq!(paragraphs(text).collect::<Vec<_>>(), ["First.", "Second."]);
    }
}
