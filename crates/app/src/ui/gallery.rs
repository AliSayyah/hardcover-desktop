//! Every shared component in its states, on one scrolling page: for checking
//! them by eye in both themes, and by keyboard (Tab walks every control).
//! Only in the demo build, opened instead of the app with
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
    AnyElement, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div,
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
}

impl Gallery {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            input: cx.new(|cx| InputState::new(window, cx).placeholder("Filter books")),
            selected_nav: 1,
        }
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
        let view = cx.entity().downgrade();
        let nav = [
            ("Home", IconName::LayoutDashboard, None),
            ("Currently reading", IconName::BookOpen, Some("3")),
            ("Want to read", IconName::Inbox, Some("1,204")),
            ("Did not finish", IconName::Ban, Some("2")),
        ];
        let nav_rows = nav
            .into_iter()
            .enumerate()
            .map(|(ix, (label, icon, count))| {
                let view = view.clone();
                let mut row = NavRow::new(("g-nav", ix), icon, label)
                    .selected(ix == self.selected_nav)
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
            });

        let upcoming =
            |ix: usize, title: &'static str, series: &'static str, date: &'static str| {
                h_flex()
                    .gap_3()
                    .child(Cover::new(CoverSize::Sm, title))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(TextButton::new(("g-up", ix), title))
                            .child(div().body().text_color(c.fg_muted).truncate().child(series)),
                    )
                    .child(
                        Lane::fit()
                            .stacked()
                            .child(div().body_strong().tabular().child(date))
                            .child(div().caption(cx).child("In 6 days")),
                    )
                    .child(Lane::action().child(if ix == 1 {
                        StatusChip(ReadingStatus::WantToRead).into_any_element()
                    } else {
                        Button::new(("g-want", ix))
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .label("Want to read")
                            .into_any_element()
                    }))
            };

        let list_row = |ix: usize, title: &'static str, selected: bool, focused: bool| {
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
                                .child(div().body().text_color(c.fg_muted).child("Andy Weir")),
                        )
                        .child(
                            Lane::trailing()
                                .stacked()
                                .child(div().caption(cx).tabular().child("66%"))
                                .child(Meter::new(("g-row-meter", ix), 66.).thin()),
                        ),
                )
        };

        let main = v_flex()
            .flex_1()
            .min_w_0()
            .gap_8()
            .child(specimen(
                "Page header, greeting, section header",
                v_flex()
                    .gap_6()
                    .child(PageHeader::new("Currently reading").count(1204))
                    .child(
                        PageHeader::new("Coming soon")
                            .line("5 books still to come in the series on your shelves"),
                    )
                    .child(Greeting::new(
                        date_line(chrono::NaiveDate::from_ymd_opt(2026, 11, 20).unwrap()),
                        greeting(9),
                    ))
                    .child(
                        SectionHeader::new("Coming soon")
                            .action(see_all("g-see-all", Some(5), |_, _, _| {})),
                    ),
            ))
            .child(specimen(
                "Panel and meters",
                v_flex()
                    .gap_4()
                    .child(
                        panel(cx)
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
                            ),
                    )
                    .child(
                        Meter::new("g-goal", 75.)
                            .pace(Some(88.))
                            .max_w(size::METER_MAX)
                            .accessibility_label("Progress toward the goal"),
                    )
                    .child(Meter::new("g-thin", 40.).thin().w(size::METER_IN_ROW))
                    .child(Meter::new("g-empty", 0.).thin().w(size::METER_IN_ROW))
                    .child(Meter::new("g-full", 120.).thin().pace(Some(100.)).w(size::METER_IN_ROW)),
            ))
            .child(specimen(
                "Sequence rows, text buttons, lanes",
                Sequence::new()
                    .child(upcoming(0, "Wind and Truth", "The Stormlight Archive #5", "Nov 26"))
                    .child(upcoming(1, "Untitled Empyrean #4", "The Empyrean #4", "Jan 5, 2027"))
                    .child(upcoming(2, "ملت عشق", "رمان‌های ترجمه‌شده", "2028"))
                    .child(upcoming(
                        3,
                        "The Extraordinarily Long Title of a Book That Never Seems to End, Volume One",
                        "A series",
                        "Feb 3",
                    )),
            ))
            .child(specimen(
                "List rows (rest, selected, selected with keyboard focus)",
                v_flex()
                    .w(size::LIST)
                    .p_2()
                    .child(list_row(0, "Project Hail Mary", false, false))
                    .child(list_row(1, "The Way of Kings", true, false))
                    .child(list_row(2, "Atomic Habits", true, true))
                    .child(SkeletonRow::new()),
            ))
            .child(specimen(
                "Blocks and property list",
                blocks()
                    .max_w(size::PROSE)
                    .child(
                        Block::new("Notes")
                            .action(
                                Button::new("g-edit")
                                    .ghost()
                                    .small()
                                    .label("Edit"),
                            )
                            .child(div().body().child("Borrowed from the library, due back on the 30th.")),
                    )
                    .child(
                        Block::new("Details").child(
                            PropertyList::new()
                                .row(
                                    "Edition",
                                    h_flex()
                                        .gap_2()
                                        .child(div().text_color(c.fg_muted).child("Not chosen"))
                                        .child(Button::new("g-choose").small().label("Choose…")),
                                )
                                .row("Added", div().tabular().child("Nov 19, 2026")),
                        ),
                    )
                    .child(Block::new("Reviews").action(div().caption(cx).child("12 reviews")).child(
                        Sequence::new()
                            .spacing(RowSpacing::Reviews)
                            .flush_top()
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_3()
                                    .child(Avatar::new("Jane Quill").data_color(Some([0x2f, 0x6f, 0x9f])))
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(BylineLink::new("g-by", "Jane Quill", "https://hardcover.app/@jane"))
                                            .child(div().caption(cx).child("Mar 14, 2026 · 48 likes")),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_3()
                                    .child(Avatar::new("شیرین").data_color(Some([0xf5, 0xe6, 0x80])))
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(BylineLink::new("g-by-rtl", "شیرین", "https://hardcover.app/@shirin"))
                                            .child(div().caption(cx).child("Mar 2, 2026 · 3 likes")),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_3()
                                    .child(Avatar::new("@quill_reads"))
                                    .child(div().body_strong().child("@quill_reads")),
                            ),
                    )),
            ))
            .child(specimen(
                "Skeletons (pulsing, then still)",
                v_flex()
                    .gap_4()
                    .max_w(size::PROSE)
                    .child(h_flex().gap_3().child(Skeleton::circle()).child(SkeletonText::new(3)))
                    .child(SkeletonText::new(2).still())
                    .child(h_flex().gap_4().child(Skeleton::cover(CoverSize::Md)).child(Skeleton::cover(CoverSize::Sm))),
            ))
            .child(specimen(
                "Kit fixes: raised field and dialogs",
                h_flex()
                    .gap_2()
                    .child(Input::new(&self.input).raised_field(false, cx).w(size::SELECT))
                    .child(Button::new("g-dialog").label("Open dialog…").on_click(|_, window, cx| {
                        open_dialog(window, cx, |dialog, _, _| {
                            dialog.title("Edit list").child("A dialog on raised.")
                        })
                    }))
                    .child(Button::new("g-alert").label("Confirm…").on_click(|_, window, cx| {
                        open_alert_dialog(window, cx, |alert, _, _| {
                            alert
                                .title("Delete “Book club picks”?")
                                .description("Its books stay in your library.")
                                .confirm()
                                .ok_text("Delete")
                        })
                    })),
            ))
            .child(specimen(
                "Empty state",
                div().h(size::RAIL).border_1().border_color(c.border).child(
                    EmptyState::new(IconName::Search, "No matches")
                        .description("No books in Did not finish match “dune”.")
                        .action(Button::new("g-clear").label("Clear the filter")),
                ),
            ));

        let rail = Rail::new()
            .section(
                RailSection::new("On Hardcover")
                    .child(
                        h_flex()
                            .items_baseline()
                            .gap_2()
                            .child(div().display_30().child("4.63")),
                    )
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
                RailSection::new("Sidebar rows")
                    .child(div().bg(c.sidebar).p_2().children(nav_rows)),
            );

        v_flex()
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
                    .overflow_y_scrollbar()
                    .child(h_flex().items_start().gap_6().p_8().child(main).child(rail)),
            )
    }
}
