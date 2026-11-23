//! Every shared component in its states, on one scrolling page: for checking
//! them by eye in both themes, and by keyboard (Tab walks every control;
//! Page Up, Page Down, Home and End scroll). Only in the demo build, opened instead of the app with
//! `HARDCOVER_GALLERY=1`:
//!
//! ```sh
//! HARDCOVER_GALLERY=1 HARDCOVER_THEME=dark cargo run -p hardcover-desktop --features demo
//! ```

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::date_picker::{DatePicker, DatePickerState};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{SearchableVec, Select, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::{Disableable as _, IconName, IndexPath, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Window, div,
    prelude::FluentBuilder as _, rems,
};
use gpui_kit::{
    FocusHandle, KeyDownEvent, Keystroke, ScrollHandle, StatefulInteractiveElement as _, point, px,
};

use super::kit::{
    RaisedField as _, SmallControl as _, SmallIconButton as _, open_alert_dialog, open_dialog,
    small_tab,
};
use super::reveal::RevealScroll as _;
use super::{
    AccountRow, Avatar, AvatarSize, Block, BylineLink, Columns, CoverShelf, CoverSlot, EmptyState,
    ExternalLink, GoalRow, Greeting, Lane, ListRow, LoadFailed, Meter, MonthBars, NavRow,
    ObjectBar, PageHeader, PropertyList, Rail, RailSection, RowSpacing, SectionHeader, Sequence,
    SeriesStrip, Skeleton, SkeletonRow, SkeletonText, Stars, StripItem, TextButton, Tier, blocks,
    books_that_fit, brand_row, date_line, greeting, panel, see_all, sidebar_label,
};
use crate::app_state::CoverService;
use crate::bidi::WrappingText;
use crate::book::star_rating::StarRating;
use crate::chip::genre::GENRES;
use crate::chip::{StatusChip, StatusIconChip, TagChip, TagGroup, TagRow, tag_groups};
use crate::cover::{Cover, CoverArt, CoverSize};

use crate::design::type_roles::Family;
use crate::design::{TypeRole, TypeRoles as _, colors, size};
use hardcover_core::{Rating, ReadingStatus};

pub struct Gallery {
    input: Entity<InputState>,
    select: Entity<SelectState<SearchableVec<&'static str>>>,
    date: Entity<DatePickerState>,
    rating: Option<Rating>,
    tab: usize,
    selected_nav: usize,
    focus: FocusHandle,
    scroll: ScrollHandle,
    /// The page chips' row keys.
    chip_keys: Entity<super::chip_keys::ChipKeys>,
}

impl Gallery {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // The covers and avatars below load through the app's cover
        // pipeline, which the shell would otherwise open.
        if cx.try_global::<CoverService>().is_none() {
            let opened = crate::io::run(crate::io::Priority::Interactive, CoverService::open);
            cx.spawn(async move |this, cx| {
                let Some(covers) = opened.await else {
                    return;
                };
                let _ = this.update(cx, |_, cx| {
                    cx.set_global(covers);
                    cx.notify();
                });
            })
            .detach();
        }
        Self {
            input: cx.new(|cx| InputState::new(window, cx).placeholder("Filter")),
            select: cx.new(|cx| {
                SelectState::new(
                    SearchableVec::new(vec!["Not finished", "Finished", "All series"]),
                    Some(IndexPath::new(0)),
                    window,
                    cx,
                )
            }),
            date: cx.new(|cx| DatePickerState::new(window, cx).date_format("%b %-d, %Y")),
            rating: Rating::new(3.5),
            tab: 1,
            selected_nav: 1,
            focus,
            scroll: ScrollHandle::new(),
            chip_keys: cx.new(|cx| super::chip_keys::ChipKeys::new(window, cx)),
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx);
        let main = v_flex()
            .flex_1()
            .min_w_0()
            .gap_8()
            .child(headers())
            .child(meters(cx))
            .child(sequence(cx))
            .child(series_strips(cx))
            .child(list_rows(cx))
            .child(blocks_and_properties(cx))
            .child(chips_and_tags(&self.chip_keys))
            .child(moods_and_votes(cx))
            .child(skeletons())
            .child(kit_fixes(&self.input, cx))
            .child(release_notes(cx))
            .child(self.controls(cx))
            // A cover asked for before the pipeline opens is missing for
            // the run, so these wait for it.
            .when(cx.has_global::<CoverService>(), |this| {
                this.child(covers_and_avatars(cx))
            })
            .child(empty(cx))
            .child(stars_and_links())
            .child(shelf_and_columns(window, cx))
            .child(wrapping_text(cx));

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
                // The scrollbar overlays a wrapper: inside the scroller it
                // would scroll away with the page.
                div()
                    .id("gallery-region")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("gallery-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .child(
                                h_flex()
                                    .items_start()
                                    .gap_6()
                                    .p_8()
                                    .child(main)
                                    .child(rail(self.selected_nav, cx)),
                            )
                            .reveals_focus(&self.scroll),
                    )
                    .vertical_scrollbar(&self.scroll),
            )
    }
}

fn headers() -> AnyElement {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 11, 20).expect("a valid date");
    specimen(
        "Page headers (a destination's; a catalogue page's, wrapping, with tools and facts), greeting, section header",
        v_flex()
            .gap_6()
            .child(PageHeader::new("Currently reading").count(1204_u32))
            .child(
                PageHeader::new("Coming soon")
                    .line("5 books still to come in the series on your shelves"),
            )
            .child(
                div().max_w(rems(28.)).child(
                    PageHeader::new("کلیدر (جلد اول): The Long and Winding Northern Chronicle")
                        .wraps()
                        .actions(
                            Button::new("g-header-tools")
                                .ghost()
                                .icon(IconName::Ellipsis)
                                .tooltip("List actions")
                                .small_icon_button(),
                        )
                        .facts("12 books · 3 more in the series · Ongoing")
                        .line("The editions you own. Mark a book owned, or not, from its page."),
                ),
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
        .child(Meter::new("g-meter", 66., "Reading progress"))
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
                Meter::new("g-goal", 75., "Progress toward the goal")
                    .pace(Some(88.))
                    .max_w(size::METER_MAX),
            )
            .child(
                Meter::new("g-thin", 40., "Reading progress")
                    .thin()
                    .w(size::METER_IN_ROW),
            )
            .child(
                Meter::new("g-empty", 0., "Reading progress")
                    .thin()
                    .w(size::METER_IN_ROW),
            )
            .child(
                Meter::new("g-full", 120., "Progress toward the goal")
                    .thin()
                    .pace(Some(100.))
                    .w(size::METER_IN_ROW),
            )
            // Hardcover's figures: a ratings breakdown row.
            .child(
                Meter::new("g-muted", 62., "4 stars")
                    .muted()
                    .max_w(size::METER_MAX),
            )
            .child(
                div().w(size::RAIL).child(
                    MonthBars::new("g-months", [1, 0, 2, 1, 0, 3, 1, 0, 1, 0, 0, 0])
                        .future_from(Some(10)),
                ),
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
            .small_control()
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

/// The series strip in every state, and a series row's main column with a
/// right-to-left name and author, shut to one line and opened.
fn series_strips(cx: &App) -> AnyElement {
    use crate::series::progress::{Activity, Group, SeriesBook, SeriesSummary};
    use hardcover_core::{BookId, BookSummary, SeriesId};

    let cover = |title: &'static str| Cover::new(CoverSize::Md, title);
    let legend = SeriesStrip::new()
        .item(
            StripItem::cover("g-s1", cover("Dune"))
                .shelf(ReadingStatus::Read)
                .position("#1"),
        )
        .item(
            StripItem::cover("g-s2", cover("Dune Messiah"))
                .shelf(ReadingStatus::CurrentlyReading)
                .position("#2")
                .progress(Some(42.))
                .next(true),
        )
        .item(
            StripItem::cover("g-s3", cover("Children of Dune"))
                .shelf(ReadingStatus::WantToRead)
                .position("#3"),
        )
        .item(
            StripItem::cover("g-s4", cover("God Emperor of Dune"))
                .shelf(ReadingStatus::Paused)
                .position("#4"),
        )
        .item(
            StripItem::cover("g-s5", cover("Heretics of Dune"))
                .shelf(ReadingStatus::DidNotFinish)
                .position("#5"),
        )
        .item(StripItem::missing("g-s6", cover("Chapterhouse: Dune")).position("#6"))
        .item(StripItem::unreleased("g-s7", "Untitled Dune #7").position("#6.5"));

    // A Persian series of ten: two read, the third paused, the rest not in
    // the library, on a line with room for six: shut, then opened.
    let today: hardcover_core::Date = "2026-11-20".parse().unwrap_or_default();
    let books = (1_u8..=10)
        .map(|n| {
            let digits: String = n
                .to_string()
                .chars()
                .map(|d| char::from_u32(0x06F0 + d.to_digit(10).unwrap_or(0)).unwrap_or(d))
                .collect();
            let status = match n {
                1 | 2 => Some(ReadingStatus::Read),
                3 => Some(ReadingStatus::Paused),
                _ => None,
            };
            SeriesBook {
                book: BookSummary::new(
                    BookId::new(90_000 + i64::from(n)),
                    format!("کلیدر {digits}"),
                ),
                position: f32::from(n),
                entry: None,
                status,
                out: true,
                release_date: None,
                progress: None,
            }
        })
        .collect();
    let persian = SeriesSummary {
        id: SeriesId::new(9_000),
        name: "کلیدر".to_owned(),
        author: Some("محمود دولت‌آبادی".to_owned()),
        books,
        next: Some(2),
        group: Group::Untouched,
        activity: Activity::PausedSince("2024-06-02".parse().unwrap_or(today)),
        last_activity: None,
    };
    specimen(
        "Series strips and cover slots",
        v_flex()
            .gap_6()
            .child(legend)
            .child(
                h_flex()
                    .gap_4()
                    .child(CoverSlot::new(CoverSize::Sm))
                    .child(CoverSlot::new(CoverSize::Sm).icon(crate::icons::clock()))
                    .child(CoverSlot::new(CoverSize::Md))
                    .child(CoverSlot::new(CoverSize::Md).icon(crate::icons::clock())),
            )
            .children([false, true].map(|open| {
                let strip = crate::series::row::RowStrip {
                    capacity: 6,
                    open,
                    on_toggle: std::rc::Rc::new(|_, _, _| {}),
                };
                // An id each, so the two copies' elements don't share ids.
                div()
                    .id(("g-series", usize::from(open)))
                    .max_w(rems(30.))
                    .child(crate::series::row::series_body(
                        &persian, today, &strip, None, cx,
                    ))
            })),
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
                        .child(Meter::new(("g-row-meter", ix), 66., "Reading progress").thin()),
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

fn stars_and_links() -> AnyElement {
    specimen(
        "Stars, external link",
        v_flex()
            .items_start()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().display_30().child("4.63"))
                    .child(
                        div()
                            .body()
                            .baseline_on(TypeRole::Body, TypeRole::Display30)
                            .child(
                                Stars::new(4.63).accessibility_label("g-average", "4.63 average"),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_4()
                    .child(Stars::new(5.).small())
                    .child(Stars::new(3.5).small())
                    .child(Stars::new(0.).small()),
            )
            .child(ExternalLink::new(
                "g-external",
                "Open on Hardcover",
                "https://hardcover.app",
            )),
    )
}

fn blocks_and_properties(cx: &App) -> AnyElement {
    let muted = colors(cx).fg_muted;
    let edition = h_flex()
        .gap_2()
        .child(div().text_color(muted).child("Not chosen"))
        .child(Button::new("g-choose").small_control().label("Choose…"));
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
                    .action(Button::new("g-edit").ghost().small_control().label("Edit"))
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

fn chips_and_tags(chip_keys: &Entity<super::chip_keys::ChipKeys>) -> AnyElement {
    let statuses = [
        ReadingStatus::WantToRead,
        ReadingStatus::CurrentlyReading,
        ReadingStatus::Read,
        ReadingStatus::Paused,
        ReadingStatus::DidNotFinish,
        ReadingStatus::Ignored,
    ];
    // Every genre icon, then genres that match none (one in Persian).
    let genres = GENRES
        .iter()
        .map(|genre| TagChip::genre(genre.example))
        .chain(["General", "ادبیات فارسی"].map(TagChip::genre));
    // The pane is as narrow as 240 inside its inset; one tag is longer
    // than that and ends in an ellipsis.
    let tags = |id: &'static str, long: bool| {
        Block::new("Tags")
            .action(
                Button::new(id)
                    .ghost()
                    .small_control()
                    .label("Show 16 more"),
            )
            .child(tag_groups([
                TagGroup::new(
                    "Genres",
                    [
                        "Fantasy",
                        "Mystery",
                        "Magical Realism",
                        "Literary Fiction",
                        "Literature & Fiction",
                        "رمان",
                    ]
                    .map(TagChip::genre),
                ),
                TagGroup::new(
                    "Content warnings",
                    [
                        "kidnapping",
                        "Mental illness",
                        "confinement",
                        "Death",
                        "خشونت",
                    ]
                    .map(|t| TagChip::new(t).warning(true)),
                ),
                TagGroup::new(
                    "Other tags",
                    [
                        "Loveable Characters",
                        "Strong Character Development",
                        "Character driven",
                        "شخصیت‌پردازی قوی",
                    ]
                    .map(TagChip::new)
                    .into_iter()
                    .chain(long.then(|| {
                        TagChip::new("An extraordinarily long tag that never fits on one line")
                    })),
                ),
            ]))
    };
    specimen(
        "Chips and tags",
        v_flex()
            .gap_6()
            .child(h_flex().gap_1p5().children(statuses.map(StatusChip)))
            .child(div().max_w(size::PROSE).child(TagRow::new(genres)))
            // Page chips: a genre with a tag page opens it (hover for the
            // fill, Tab for the ring, the arrows along the row); General
            // has no page and stays plain.
            .child(
                div().max_w(size::PROSE).child(
                    TagRow::new(
                        [
                            "Fantasy",
                            "Science Fiction",
                            "General",
                            "Mystery",
                            "Young Adult",
                        ]
                        .map(TagChip::genre_page),
                    )
                    .keys("g-page-chips", chip_keys),
                ),
            )
            // A Tags block left with one group takes the group's name.
            .child(
                div()
                    .max_w(size::PROSE)
                    .child(Block::new("Content warnings").child(TagRow::new(
                        ["Death", "Grief", "Pandemic"].map(|t| TagChip::new(t).warning(true)),
                    ))),
            )
            .child(div().max_w(size::PROSE).child(tags("g-tags", false)))
            .child(div().w(rems(15.)).child(tags("g-tags-narrow", true))),
    )
}

/// Synthetic votes for the mood line's and the popover's states: moods,
/// then pace (slow, medium, fast), drive (plot, a mix, character) and the
/// character pairs (yes, no).
fn votes(
    moods: &[(hardcover_core::tags::Mood, u32)],
    pace: [u32; 3],
    drive: [u32; 3],
    traits: &[(hardcover_core::tags::CharacterTrait, u32)],
) -> hardcover_core::tags::BookVotes {
    use hardcover_core::tags::{BookVotes, Drive, Pace};
    BookVotes::from_counts(
        moods
            .iter()
            .map(|(m, n)| (m.id(), *n))
            .chain(Pace::ALL.into_iter().zip(pace).map(|(p, n)| (p.id(), n)))
            .chain(Drive::ALL.into_iter().zip(drive).map(|(d, n)| (d.id(), n)))
            .chain(traits.iter().map(|(t, n)| (t.id(), *n))),
    )
}

fn moods_and_votes(cx: &App) -> AnyElement {
    use std::rc::Rc;

    use hardcover_core::tags::{self, CharacterTrait as T, Mood as M};

    use crate::book::moods::{MoodLine, VotesPanel};

    // Hopeful, Funny and Adventurous stand out; Medium pace; Rarely dark.
    let everything = votes(
        &[
            (M::Adventurous, 420),
            (M::Funny, 330),
            (M::Hopeful, 250),
            (M::Emotional, 180),
            (M::Lighthearted, 160),
            (M::Inspiring, 150),
            (M::Mysterious, 140),
            (M::Reflective, 120),
            (M::Tense, 110),
            (M::Informative, 90),
            (M::Challenging, 40),
            (M::Sad, 30),
            (M::Dark, 24),
            (M::Relaxing, 23),
        ],
        [58, 259, 133],
        [130, 232, 43],
        &[
            (T::Loveable, 379),
            (T::Unloveable, 31),
            (T::StrongDevelopment, 267),
            (T::WeakDevelopment, 124),
            (T::Diverse, 203),
            (T::NotDiverse, 175),
        ],
    );
    // Fast, with a pace and a drive but no character pair readers agree on.
    let fast = votes(
        &[
            (M::Adventurous, 300),
            (M::Tense, 220),
            (M::Funny, 90),
            (M::Dark, 60),
        ],
        [10, 60, 240],
        [180, 40, 20],
        &[],
    );
    // As used across Hardcover: nothing stands out, so the most voted.
    let fallback = votes(
        &M::ALL.map(|m| {
            let share = u64::from(m.site_votes()) * 1000 / tags::SITE_MOOD_VOTES;
            (m, u32::try_from(share).unwrap_or(0))
        }),
        [0; 3],
        [0; 3],
        &[],
    );
    // 80 mood votes: a line, but no Rarely and no other group.
    let moods_only = votes(
        &[
            (M::Reflective, 30),
            (M::Sad, 22),
            (M::Emotional, 18),
            (M::Challenging, 6),
            (M::Hopeful, 4),
        ],
        [0; 3],
        [0; 3],
        &[],
    );
    // 140 mood votes and none dark, mysterious or sad.
    let with_rarely = votes(
        &[
            (M::Relaxing, 50),
            (M::Lighthearted, 40),
            (M::Funny, 30),
            (M::Hopeful, 20),
        ],
        [0; 3],
        [0; 3],
        &[],
    );
    let too_few = votes(&[(M::Dark, 20), (M::Tense, 20)], [0; 3], [0; 3], &[]);
    let line = |label: &'static str, id: i64, votes: &tags::BookVotes| {
        v_flex()
            .gap_1()
            .child(div().caption(cx).child(label))
            .child(
                MoodLine::new(hardcover_core::BookId::new(id), votes).map_or_else(
                    || {
                        div()
                            .body()
                            .text_color(colors(cx).fg_muted)
                            .child("(no line, no Votes)")
                            .into_any_element()
                    },
                    IntoElement::into_any_element,
                ),
            )
    };
    let c = colors(cx);
    let panel = |votes: &tags::BookVotes| {
        div()
            .flex_none()
            .p_4()
            .rounded(super::focus::box_radius(cx))
            .bg(c.raised)
            .shadow(c.shadow_popover())
            .child(VotesPanel::new(Rc::new(tags::breakdown(votes))))
    };
    specimen(
        "Mood line and votes",
        v_flex()
            .gap_6()
            .child(
                v_flex()
                    .gap_4()
                    .max_w(size::PROSE)
                    .child(line("Standouts, no pace", 9_000_001, &everything))
                    .child(line("Standouts and a pace", 9_000_002, &fast))
                    .child(line(
                        "Nothing stands out: the most voted",
                        9_000_003,
                        &fallback,
                    ))
                    .child(line("Under 50 mood votes", 9_000_004, &too_few)),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .items_start()
                    .gap_6()
                    .child(panel(&everything))
                    .child(panel(&moods_only))
                    .child(panel(&with_rarely)),
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
            });
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
            });
        });
    let small = h_flex()
        .gap_2()
        .child(
            Button::new("g-small")
                .label("Update progress…")
                .small_control(),
        )
        .child(
            Button::new("g-small-ghost")
                .ghost()
                .icon(IconName::Plus)
                .label("Want to read")
                .small_control(),
        )
        .child(
            Button::new("g-small-icon")
                .ghost()
                .icon(IconName::Ellipsis)
                .tooltip("Read actions")
                .small_icon_button(),
        );
    let switches = h_flex()
        .gap_4()
        .children(
            [(false, false), (true, false), (false, true), (true, true)].map(
                |(checked, disabled)| {
                    Switch::new(SharedString::from(format!("g-switch-{checked}-{disabled}")))
                        .small()
                        .checked(checked)
                        .disabled(disabled)
                        .accessibility_label("Owned")
                },
            ),
        )
        .child(
            RadioGroup::horizontal("g-radios")
                .children(["System", "Light"].map(|l| Radio::new(l).label(l)))
                .selected_index(Some(1)),
        )
        .child(
            RadioGroup::horizontal("g-radios-disabled")
                .children(["System", "Light"].map(|l| Radio::new(l).label(l)))
                .selected_index(Some(1))
                .disabled(true),
        );
    specimen(
        "Kit fixes: raised field, small controls, switches, dialogs",
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(input).raised_field(false, cx).w(size::SELECT))
                    .child(dialog)
                    .child(alert),
            )
            .child(small)
            .child(switches),
    )
}

fn empty(cx: &App) -> AnyElement {
    let region = |child: AnyElement| {
        div()
            .h(size::RAIL)
            .border_1()
            .border_color(colors(cx).border)
            .child(child)
    };
    specimen(
        "Empty state and a failed load",
        v_flex()
            .gap_4()
            .child(region(
                EmptyState::new(IconName::Search, "No matches")
                    .description("No books in Did not finish match “dune”.")
                    .action(Button::new("g-clear").label("Clear the filter"))
                    .into_any_element(),
            ))
            .child(
                LoadFailed::new(
                    "g-retry",
                    "No connection. Check your network and try again.",
                )
                .on_retry(|_, _, _| {}),
            )
            .child(region(
                LoadFailed::new(
                    "g-retry-region",
                    "No connection. Check your network and try again.",
                )
                .region("Couldn't load your library")
                .on_retry(|_, _, _| {})
                .into_any_element(),
            )),
    )
}

fn gallery_nav_rows(selected_nav: usize, cx: &mut Context<Gallery>) -> Vec<NavRow> {
    let view = cx.entity().downgrade();
    let nav = [
        ("Home", IconName::LayoutDashboard, None),
        ("Currently reading", IconName::BookOpen, Some("3")),
        ("Want to read", IconName::Inbox, Some("1,204")),
        ("Did not finish", IconName::Ban, Some("2")),
    ];
    nav.into_iter()
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
            // While Ctrl is held, a destination's shortcut replaces its count.
            if ix == 3 {
                let ctrl_5 = Keystroke::parse("ctrl-5").expect("a valid keystroke");
                row = row.hint(Some(Kbd::new(ctrl_5)));
            }
            row
        })
        .collect()
}

fn rail(selected_nav: usize, cx: &mut Context<Gallery>) -> Rail {
    let c = colors(cx);
    let nav_rows = gallery_nav_rows(selected_nav, cx);
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
                        .small_control()
                        .icon(IconName::Plus)
                        .label("Add a past read…"),
                )
                .child(div().body().child("Current read")),
        )
        .section(
            RailSection::new("Sidebar rows").child(
                div()
                    .bg(c.sidebar)
                    .p_2()
                    .child(brand_row(cx))
                    .children(nav_rows)
                    .child(sidebar_label("Reading goal", cx))
                    .child(
                        GoalRow::new("g-goal", "2026", "9 of 12 books", 75.)
                            .pace(Some(88.))
                            .on_click(|_, _, _| {}),
                    )
                    .child(
                        GoalRow::new("g-goal-dates", "By Jun 30", "1,200 of 5,000 pages", 24.)
                            .pace(Some(61.))
                            .on_click(|_, _, _| {}),
                    )
                    .child(sidebar_label("Lists", cx))
                    .child(
                        NavRow::new("g-new-list", IconName::Plus, "New list…")
                            .muted(true)
                            .on_click(|_, _, _| {}),
                    )
                    .child(
                        NavRow::new("g-new-goal", IconName::Plus, "Set a reading goal…")
                            .muted(true)
                            .disabled(true)
                            .tooltip(Some("Disabled: no permission")),
                    )
                    .child(
                        AccountRow::new(
                            "g-account",
                            Avatar::new("demo_reader"),
                            "demo_reader",
                            "Updated just now",
                        )
                        .dropdown_menu(|menu, _, _| menu.label("The account menu")),
                    ),
            ),
        )
}

fn shelf_and_columns(window: &Window, cx: &App) -> AnyElement {
    const TITLES: [&str; 9] = [
        "Fourth Wing",
        "Tomorrow, and Tomorrow, and Tomorrow",
        "ملت عشق (جلد اول)",
        "The Midnight Library",
        "Piranesi",
        "The Left Hand of Darkness",
        "Babel",
        "Station Eleven",
        "The Fifth Season",
    ];
    let books = |key: &'static str, size: CoverSize, count: usize| {
        TITLES
            .into_iter()
            .take(count)
            .enumerate()
            .map(move |(ix, title)| (Cover::new(size, title), TextButton::new((key, ix), title)))
    };
    let shelf = || CoverShelf::new(CoverSize::Md).books(books("g-shelf", CoverSize::Md, 3));
    let column = |text: &'static str| div().body().text_color(colors(cx).fg_muted).child(text);
    let main = || column("The main column");
    let section = || RailSection::new("Newest on Want to read").child(shelf());
    // Home's second column: one row, as many as fit (here a 448 column).
    let rem = window.rem_size();
    let fit = books_that_fit(size::WIDE_COLUMN_MIN.to_pixels(rem), CoverSize::Md, rem);
    let one_row = div()
        .flex()
        .flex_col()
        .gap_3()
        .w(size::WIDE_COLUMN_MIN)
        .child(SectionHeader::new("Newest on Want to read"))
        .child(CoverShelf::new(CoverSize::Md).one_row().books(books(
            "g-shelf-row",
            CoverSize::Md,
            fit,
        )));
    // Similar books beside the book's text: a grid of lg covers.
    let grid = div().w(rems(30.)).child(
        Block::new("Similar books").child(CoverShelf::new(CoverSize::Lg).grid().books(books(
            "g-shelf-grid",
            CoverSize::Lg,
            9,
        ))),
    );
    specimen(
        "Cover shelf (wrapping, one row, grid); columns: three, beside the rail, folded",
        v_flex()
            .gap_8()
            .child(one_row)
            .child(grid)
            .child(
                Columns::new(Tier::Three, main())
                    .second(column("The second column, sharing the width"))
                    .wide_gap()
                    .rail([section()]),
            )
            .child(
                Columns::new(Tier::Rail, main())
                    .wide_gap()
                    .rail([section()]),
            )
            .child(
                Columns::new(Tier::Folded, main())
                    .wide_gap()
                    .rail([section(), RailSection::new("Up next")]),
            ),
    )
}

impl Gallery {
    /// The app's own controls and the kit's that the app restyles: the
    /// rating (the only gold), the segmented tabs, a select and a date
    /// field.
    fn controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity().downgrade();
        let rating_view = view.clone();
        let stars =
            StarRating::new("g-stars")
                .value(self.rating)
                .on_change(move |rating, _, cx| {
                    let _ = rating_view.update(cx, |this, cx| {
                        this.rating = *rating;
                        cx.notify();
                    });
                });
        let small_stars = StarRating::new("g-stars-small").small().value(None);
        let disabled_stars = StarRating::new("g-stars-disabled")
            .value(Rating::new(4.))
            .disabled(true)
            .tooltip("Needs permission to change your library");
        let tabs = TabBar::new("g-tabs")
            .segmented()
            .small_control()
            .selected_index(self.tab)
            .children(["Week", "Month", "Year", "All time"].map(small_tab))
            .on_click(move |ix, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.tab = *ix;
                    cx.notify();
                });
            });
        let disabled_tabs = TabBar::new("g-tabs-disabled")
            .segmented()
            .small_control()
            .selected_index(0)
            .child(small_tab("Page"))
            .child(small_tab("Time").disabled(true))
            .child(small_tab("%"));
        specimen(
            "Rating, segmented tabs, select, date field",
            v_flex()
                .gap_3()
                .child(
                    h_flex()
                        .gap_6()
                        .child(stars)
                        .child(small_stars)
                        .child(disabled_stars),
                )
                .child(h_flex().gap_4().child(tabs).child(disabled_tabs))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .w(size::SELECT)
                                .child(Select::new(&self.select).raised_field(false, cx)),
                        )
                        .child(
                            div().w(size::SELECT_SMALL).child(
                                Select::new(&self.select)
                                    .small_control()
                                    .raised_field(false, cx),
                            ),
                        )
                        .child(
                            div()
                                .w_full()
                                .max_w(size::FIELD_DATE_MAX)
                                .child(DatePicker::new(&self.date).cleanable(false)),
                        ),
                ),
        )
    }
}

/// Covers with an image and loading, status icon chips, avatars with a
/// picture and at xl, and a right-to-left text button.
fn covers_and_avatars(cx: &App) -> AnyElement {
    // A demo catalogue cover: the demo's cover cache fetches only those.
    const COVER: &str =
        "https://assets.hardcover.app/editions/3274049/8741341047797682-91mYu67RfUL._SL1500_.jpg";
    let covers = h_flex()
        .items_end()
        .gap_4()
        .child(Cover::new(CoverSize::Sm, "Project Hail Mary").thumbnail(Some(COVER)))
        .child(
            Cover::new(CoverSize::Md, "Project Hail Mary")
                .thumbnail(Some(COVER))
                .shadow(true),
        )
        .child(Cover::new(CoverSize::Md, "Loading").art(CoverArt::Loading))
        .child(Cover::new(CoverSize::Sm, "Loading").art(CoverArt::Loading));
    let chips = h_flex().gap_2().children(
        [
            ReadingStatus::WantToRead,
            ReadingStatus::CurrentlyReading,
            ReadingStatus::Read,
            ReadingStatus::Paused,
            ReadingStatus::DidNotFinish,
        ]
        .map(StatusIconChip),
    );
    let avatars = h_flex()
        .items_end()
        .gap_4()
        .child(Avatar::new("Jane Quill").image(Some(COVER)))
        .child(Avatar::new("Jane Quill").data_color(Some([0x2F, 0x6B, 0x4F])))
        .child(Avatar::new("شیرین"))
        .child(
            Avatar::new("Jane Quill")
                .image(Some(COVER))
                .size(AvatarSize::Xl),
        )
        .child(Avatar::new("Jane Quill").size(AvatarSize::Xl));
    specimen(
        "Covers (image, loading), status icon chips, avatars, a right-to-left text button",
        v_flex()
            .gap_4()
            .child(covers)
            .child(chips)
            .child(avatars)
            .child(
                h_flex()
                    .gap_4()
                    .body()
                    .child(TextButton::new("g-rtl-button", "ملت عشق (جلد اول)"))
                    .child(div().caption(cx).child("opens the book, like any title")),
            ),
    )
}

/// Wrapped right-to-left text and cloth covers with long titles: the
/// punctuation, digits, brackets and Latin runs `WrappingText` lays out
/// itself, and titles broken between words.
/// The update dialog's release notes, as GitHub generates them: a short
/// release that fits, and a long one that scrolls.
fn release_notes(cx: &App) -> AnyElement {
    let change = |title: &str, n: usize| {
        format!(
            "* {title} by @AliSayyah in https://github.com/AliSayyah/hardcover-desktop/pull/{n}\n"
        )
    };
    let body = |titles: &[&str]| {
        let mut body = String::from("## What's Changed\n");
        for (n, title) in titles.iter().enumerate() {
            body.push_str(&change(title, 90 + n));
        }
        body.push_str(&change("Release 0.4.1", 99));
        body.push_str("\n\n**Full Changelog**: https://github.com/AliSayyah/hardcover-desktop/compare/v0.4.0...v0.4.1\n");
        body
    };
    let short = body(&[
        "Upgrade the dependencies, keyring to keyring-core",
        "Series: a readable shelf for each series",
        "Show HTML descriptions as plain paragraphs",
    ]);
    let long = body(&[
        "Polish the UI: bold text, search keys, read dates, shortcut hints, Persian text",
        "Redraw the rating stars",
        "Add a Home page: reading pace, goal outlook and what to read next",
        "Home: rate what you finished; Search: your own books first",
        "Redesign chips: neutral pills with status icons, tags on the label line",
        "Show other readers' reviews on the book page",
        "Coming soon: upcoming books in the reader's series",
        "Design system: Vazirmatn for Persian and Arabic everywhere",
        "Design foundation: palette, tokens, fonts, cover component, chips",
        "Series: where you are in each series, and what's next",
        "QA fixes: keyboard, focus and chrome",
        "Add a self-updating AppImage and a signed installer for Linux",
    ]);
    let dialog = |id: &'static str, body: &str| {
        let notes: SharedString = crate::update::release_notes(body).into();
        v_flex()
            .id(id)
            .w(size::DIALOG_CONFIRM)
            .p_6()
            .gap_4()
            .bg(colors(cx).raised)
            .border_1()
            .border_color(colors(cx).border)
            .rounded(gpui_kit::component::ActiveTheme::theme(cx).radius_lg)
            .children(crate::shell::release_notes_view(&notes))
            .child(ExternalLink::new(
                "release-page",
                "Open the release page",
                "https://github.com/AliSayyah/hardcover-desktop/releases",
            ))
    };
    specimen(
        "Release notes in the update dialog (short; long, scrolling)",
        h_flex()
            .items_start()
            .gap_6()
            .child(dialog("notes-short", &short))
            .child(dialog("notes-long", &long)),
    )
}

fn wrapping_text(cx: &App) -> AnyElement {
    let paragraph = "کتابی پرکشش و پر از امید. دوستی دو شخصیت اصلی (Rocky و Grace) را خیلی دوست داشتم، \
        و تا صفحهٔ ۴۷۶ نتوانستم کتاب را زمین بگذارم. چرا زودتر نخوانده بودم؟ «پروژهٔ هیل مری» در 2021 منتشر شد.";
    specimen(
        "Right-to-left text; cloth covers with long titles",
        v_flex()
            .gap_4()
            .child(
                div()
                    .max_w(rems(20.))
                    .reader()
                    .child(WrappingText::new(paragraph)),
            )
            .child(
                h_flex()
                    .items_start()
                    .gap_4()
                    .child(Cover::new(CoverSize::Xs, "The Way of Kings"))
                    .child(Cover::new(
                        CoverSize::Md,
                        "Tomorrow, and Tomorrow, and Tomorrow",
                    ))
                    .child(
                        Cover::new(CoverSize::Lg, "Tomorrow, and Tomorrow, and Tomorrow")
                            .author(Some("Gabrielle Zevin")),
                    )
                    .child(
                        Cover::new(
                            CoverSize::Xl,
                            "The Hitchhiker's Guide to the Galaxy: The Trilogy of Five",
                        )
                        .author(Some("Douglas Adams")),
                    )
                    .child(Cover::new(
                        CoverSize::Md,
                        "کتابی پرکشش (جلد اول) «امید»، دوستی دو شخصیت",
                    )),
            )
            .child(
                div()
                    .caption(cx)
                    .child("Cloth titles break between words, at most four lines."),
            ),
    )
}
