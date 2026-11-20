//! Every shared component in its states, on one scrolling page: for checking
//! them by eye in both themes, and by keyboard (Tab walks every control;
//! Page Up, Page Down, Home and End scroll). Only in the demo build, opened instead of the app with
//! `HARDCOVER_GALLERY=1`:
//!
//! ```sh
//! HARDCOVER_GALLERY=1 HARDCOVER_THEME=dark cargo run -p hardcover-desktop --features demo
//! ```

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::list::ListItem;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div,
};
use gpui_kit::{
    FocusHandle, KeyDownEvent, ScrollHandle, StatefulInteractiveElement as _, point, px,
};

use super::kit::{RaisedField as _, open_alert_dialog, open_dialog};
use super::*;
use crate::chip::StatusChip;
use crate::cover::{Cover, CoverSize};
use crate::design::type_roles::Family;
use crate::design::{TypeRoles as _, colors, size};
use hardcover_core::ReadingStatus;

pub struct Gallery {
    input: Entity<InputState>,
    selected_nav: usize,
    focus: FocusHandle,
    scroll: ScrollHandle,
}

impl Gallery {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            input: cx.new(|cx| InputState::new(window, cx).placeholder("Filter books")),
            selected_nav: 1,
            focus,
            scroll: ScrollHandle::new(),
        }
    }

    /// Page Up and Down, Home and End: the page scrolls without a pointer
    /// (a headless session has none).
    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.scroll.offset();
        let page = self.scroll.bounds().size.height * 0.9;
        let max = self.scroll.max_offset().y;
        let y = match event.keystroke.key.as_str() {
            "pagedown" => offset.y - page,
            "pageup" => offset.y + page,
            "home" => px(0.),
            "end" => -max,
            _ => return,
        };
        self.scroll
            .set_offset(point(offset.x, y.clamp(-max, px(0.))));
        cx.notify();
    }
}

/// A titled specimen.
fn specimen(title: &'static str, body: impl IntoElement) -> AnyElement {
    v_flex()
        .gap_3()
        .child(div().heading().child(title))
        .child(body)
        .into_any_element()
}

fn tools() -> Vec<AnyElement> {
    vec![
        Button::new("g-lists")
            .ghost()
            .icon(IconName::GalleryVerticalEnd)
            .label("Lists")
            .into_any_element(),
        Button::new("g-more")
            .ghost()
            .icon(IconName::Ellipsis)
            .tooltip("More actions")
            .into_any_element(),
    ]
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx);
        let main = v_flex()
            .flex_1()
            .min_w_0()
            .gap_8()
            .child(headers())
            .child(meters(cx))
            .child(sequence(cx))
            .child(list_rows(cx))
            .child(blocks_and_properties(cx))
            .child(skeletons())
            .child(kit_fixes(&self.input, cx))
            .child(empty(cx));

        v_flex()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key_down))
            .type_family(Family::Sans)
            .size_full()
            .bg(c.bg)
            .text_color(c.fg)
            .child(ObjectBar::new().tools(tools()))
            .child(
                ObjectBar::new()
                    .hairline(true)
                    .back("Discover", |_, _, _| {})
                    .tools(tools()),
            )
            .child(
                div()
                    .id("gallery-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .vertical_scrollbar(&self.scroll)
                    .child(
                        h_flex()
                            .items_start()
                            .gap_6()
                            .p_8()
                            .child(main)
                            .child(rail(self.selected_nav, cx)),
                    ),
            )
    }
}

fn headers() -> AnyElement {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).expect("a valid date");
    specimen(
        "Page header, greeting, section header",
        v_flex()
            .gap_6()
            .child(PageHeader::new("Currently reading").count(1204))
            .child(
                PageHeader::new("Coming soon")
                    .line("5 books still to come in the series on your shelves"),
            )
            .child(Greeting::new(date_line(date), greeting(9)))
            .child(SectionHeader::new("Coming soon").action(see_all(
                "g-see-all",
                Some(5),
                |_, _, _| {},
            ))),
    )
}

fn meters(cx: &App) -> AnyElement {
    let c = colors(cx);
    let progress = panel(cx)
        .flex()
        .flex_col()
        .gap_3()
        .max_w(size::METER_MAX)
        .child(
            h_flex()
                .child(div().body_strong().tabular().child("Page 312 of 476"))
                .child(div().flex_1())
                .child(div().body().tabular().text_color(c.fg_muted).child("66%")),
        )
        .child(Meter::new("g-meter", 66.).accessibility_label("Reading progress"))
        .child(
            div()
                .caption(cx)
                .child("Started Nov 1 · 16 pages a day · done around Dec 1"),
        );
    specimen(
        "Panel and meters",
        v_flex()
            .gap_4()
            .child(progress)
            .child(
                Meter::new("g-goal", 75.)
                    .pace(Some(88.))
                    .max_w(size::METER_MAX)
                    .accessibility_label("Progress toward the goal"),
            )
            .child(Meter::new("g-thin", 40.).thin().w(size::METER_IN_ROW))
            .child(Meter::new("g-empty", 0.).thin().w(size::METER_IN_ROW))
            .child(
                Meter::new("g-full", 120.)
                    .thin()
                    .pace(Some(100.))
                    .w(size::METER_IN_ROW),
            ),
    )
}

fn upcoming_row(
    ix: usize,
    title: &'static str,
    series: &'static str,
    date: &'static str,
    cx: &App,
) -> AnyElement {
    let action = if ix == 1 {
        StatusChip(ReadingStatus::WantToRead).into_any_element()
    } else {
        Button::new(("g-want", ix))
            .ghost()
            .small()
            .icon(IconName::Plus)
            .label("Want to read")
            .into_any_element()
    };
    h_flex()
        .gap_3()
        .child(Cover::new(CoverSize::Sm, title))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(TextButton::new(("g-up", ix), title))
                .child(
                    div()
                        .body()
                        .text_color(colors(cx).fg_muted)
                        .truncate()
                        .child(series),
                ),
        )
        .child(
            Lane::fit()
                .stacked()
                .child(div().body_strong().tabular().child(date))
                .child(div().caption(cx).child("In 6 days")),
        )
        .child(Lane::action().child(action))
        .into_any_element()
}

fn sequence(cx: &App) -> AnyElement {
    let long = "The Extraordinarily Long Title of a Book That Never Seems to End, Volume One";
    specimen(
        "Sequence rows, text buttons, lanes",
        Sequence::new()
            .child(upcoming_row(
                0,
                "Wind and Truth",
                "The Stormlight Archive #5",
                "Nov 26",
                cx,
            ))
            .child(upcoming_row(
                1,
                "Untitled Empyrean #4",
                "The Empyrean #4",
                "Jan 5, 2027",
                cx,
            ))
            .child(upcoming_row(2, "ملت عشق", "رمان‌های ترجمه‌شده", "2028", cx))
            .child(upcoming_row(3, long, "A series", "Feb 3", cx)),
    )
}

fn list_row(ix: usize, title: &'static str, selected: bool, focused: bool, cx: &App) -> ListItem {
    ListItem::new(("g-row", ix))
        .list_row(focused, cx)
        .selected(selected)
        .child(
            h_flex()
                .gap_3()
                .child(Cover::new(CoverSize::Sm, title))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().body_strong().truncate().child(title))
                        .child(
                            div()
                                .body()
                                .text_color(colors(cx).fg_muted)
                                .child("Andy Weir"),
                        ),
                )
                .child(
                    Lane::trailing()
                        .stacked()
                        .child(div().caption(cx).tabular().child("66%"))
                        .child(Meter::new(("g-row-meter", ix), 66.).thin()),
                ),
        )
}

fn list_rows(cx: &App) -> AnyElement {
    specimen(
        "List rows (rest, selected, selected with keyboard focus)",
        v_flex()
            .w(size::LIST)
            .p_2()
            .child(list_row(0, "Project Hail Mary", false, false, cx))
            .child(list_row(1, "The Way of Kings", true, false, cx))
            .child(list_row(2, "Atomic Habits", true, true, cx))
            .child(SkeletonRow::new()),
    )
}

fn review(
    id: &'static str,
    name: &'static str,
    color: Option<[u8; 3]>,
    meta: &'static str,
    cx: &App,
) -> AnyElement {
    let url = format!("https://hardcover.app/@{id}");
    h_flex()
        .items_start()
        .gap_3()
        .child(Avatar::new(name).data_color(color))
        .child(
            v_flex()
                .gap_1()
                .child(BylineLink::new(id, name, url))
                .child(div().caption(cx).child(meta)),
        )
        .into_any_element()
}

fn blocks_and_properties(cx: &App) -> AnyElement {
    let muted = colors(cx).fg_muted;
    let edition = h_flex()
        .gap_2()
        .child(div().text_color(muted).child("Not chosen"))
        .child(Button::new("g-choose").small().label("Choose…"));
    let reviews = Sequence::new()
        .spacing(RowSpacing::Reviews)
        .flush_top()
        .child(review(
            "jane",
            "Jane Quill",
            Some([0x2f, 0x6f, 0x9f]),
            "Mar 14, 2026 · 48 likes",
            cx,
        ))
        .child(review(
            "shirin",
            "شیرین",
            Some([0xf5, 0xe6, 0x80]),
            "Mar 2, 2026 · 3 likes",
            cx,
        ))
        .child(review(
            "quill_reads",
            "@quill_reads",
            None,
            "Feb 9, 2026",
            cx,
        ));
    specimen(
        "Blocks and property list",
        blocks()
            .max_w(size::PROSE)
            .child(
                Block::new("Notes")
                    .action(Button::new("g-edit").ghost().small().label("Edit"))
                    .child(
                        div()
                            .body()
                            .child("Borrowed from the library, due back on the 30th."),
                    ),
            )
            .child(
                Block::new("Details").child(
                    PropertyList::new()
                        .row("Edition", edition)
                        .row("Added", div().tabular().child("Nov 19, 2026")),
                ),
            )
            .child(
                Block::new("Reviews")
                    .action(div().caption(cx).child("12 reviews"))
                    .child(reviews),
            ),
    )
}

fn skeletons() -> AnyElement {
    specimen(
        "Skeletons (pulsing, then still)",
        v_flex()
            .gap_4()
            .max_w(size::PROSE)
            .child(
                h_flex()
                    .gap_3()
                    .child(Skeleton::circle())
                    .child(SkeletonText::new(3)),
            )
            .child(SkeletonText::new(2).still())
            .child(
                h_flex()
                    .gap_4()
                    .child(Skeleton::cover(CoverSize::Md))
                    .child(Skeleton::cover(CoverSize::Sm)),
            ),
    )
}

fn kit_fixes(input: &Entity<InputState>, cx: &App) -> AnyElement {
    let dialog = Button::new("g-dialog")
        .label("Open dialog…")
        .on_click(|_, window, cx| {
            open_dialog(window, cx, |dialog, _, _| {
                dialog.title("Edit list").child("A dialog on raised.")
            })
        });
    let alert = Button::new("g-alert")
        .label("Confirm…")
        .on_click(|_, window, cx| {
            open_alert_dialog(window, cx, |alert, _, _| {
                alert
                    .title("Delete “Book club picks”?")
                    .description("Its books stay in your library.")
                    .confirm()
                    .ok_text("Delete")
            })
        });
    specimen(
        "Kit fixes: raised field and dialogs",
        h_flex()
            .gap_2()
            .child(Input::new(input).raised_field(false, cx).w(size::SELECT))
            .child(dialog)
            .child(alert),
    )
}

fn empty(cx: &App) -> AnyElement {
    specimen(
        "Empty state",
        div()
            .h(size::RAIL)
            .border_1()
            .border_color(colors(cx).border)
            .child(
                EmptyState::new(IconName::Search, "No matches")
                    .description("No books in Did not finish match “dune”.")
                    .action(Button::new("g-clear").label("Clear the filter")),
            ),
    )
}

fn rail(selected_nav: usize, cx: &mut Context<Gallery>) -> Rail {
    let c = colors(cx);
    let view = cx.entity().downgrade();
    let nav = [
        ("Home", IconName::LayoutDashboard, None),
        ("Currently reading", IconName::BookOpen, Some("3")),
        ("Want to read", IconName::Inbox, Some("1,204")),
        ("Did not finish", IconName::Ban, Some("2")),
    ];
    let nav_rows: Vec<NavRow> = nav
        .into_iter()
        .enumerate()
        .map(|(ix, (label, icon, count))| {
            let view = view.clone();
            let mut row = NavRow::new(("g-nav", ix), icon, label)
                .selected(ix == selected_nav)
                .on_click(move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.selected_nav = ix;
                        cx.notify();
                    });
                });
            if let Some(count) = count {
                row = row.count(count);
            }
            if ix == 3 {
                row = row.hint(Some(div().caption(cx).child("Ctrl 5")));
            }
            row
        })
        .collect();

    Rail::new()
        .section(
            RailSection::new("On Hardcover")
                .child(div().display_30().child("4.63"))
                .child(
                    div()
                        .body()
                        .text_color(c.fg_muted)
                        .child("4,280 ratings · 10,144 readers"),
                ),
        )
        .section(
            RailSection::new("Details").child(
                PropertyList::new()
                    .in_rail(true)
                    .row("Edition", div().text_color(c.fg_muted).child("Not chosen"))
                    .row("Added", div().tabular().child("Oct 10, 2026")),
            ),
        )
        .section(
            RailSection::new("Reads")
                .action(
                    Button::new("g-add-read")
                        .ghost()
                        .small()
                        .icon(IconName::Plus)
                        .label("Add a past read…"),
                )
                .child(div().body().child("Current read")),
        )
        .section(
            RailSection::new("Sidebar rows").child(div().bg(c.sidebar).p_2().children(nav_rows)),
        )
}
