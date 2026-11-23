//! What Hardcover's readers say a book is like (`hardcover-design`, The
//! mood line and The Votes popover): one line in the book's header, and
//! every figure behind it on demand.
//!
//! - [`MoodLine`]: up to three moods (`tags::mood_line`, from the book's
//!   exact votes), each a text button that opens the mood's page, " · " in
//!   `fg-muted` between them; for a slow or fast book, the pace in plain
//!   ink; then, 4 after, "Votes", a small ghost menu button. The moods wrap;
//!   the pace and "Votes" stay together. Under 50 mood votes there is no
//!   line and no "Votes" ([`MoodLine::new`] gives `None`). To a screen
//!   reader the line is a group named "Moods from Hardcover readers".
//! - The Votes popover (the kit's `Popover`, 360 wide, inset 16, `raised`,
//!   `r-box` and `shadow-popover`): [`VotesPanel`], "Votes from Hardcover
//!   readers", then the groups `tags::breakdown` gives, 16 apart: Moods (a
//!   muted meter per mood against the first mood's votes, its votes in a 48
//!   lane), Pace and Driven by (three cells, shares adding up to 100:
//!   [`shares`]), Characters (one muted line), and Rarely under a hairline.
//!   The meters are Hardcover's figures: muted, and they never move.
//! - **Keys:** Enter or Space on "Votes" opens it with focus on the first
//!   mood; Tab goes through the moods; Escape, or a press outside, closes it
//!   and gives focus back to "Votes" (the button's handle is the app's, so
//!   also after a pointer opened it). Choosing a mood closes it and opens
//!   the mood's page.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    Anchor, App, ElementId, FocusHandle, FontWeight, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::BookId;
use hardcover_core::tags::{
    self, BookVotes, CharacterLean, CharacterTrait, Drive, Mood, Pace, Split, VoteBreakdown,
};

use crate::catalog::navigation::{self, Page};
use crate::catalog::tag::Tag;
use crate::design::{TypeRoles as _, colors, size};
use crate::format::count_text;
use crate::ui::focus::box_radius;
use crate::ui::kit::{KeyboardRing as _, SmallControl as _};
use crate::ui::{Meter, TextButton};

/// The pace as the line names it.
pub fn pace_line_word(pace: Pace) -> &'static str {
    match pace {
        Pace::Slow => "Slow pace",
        Pace::Medium => "Medium pace",
        Pace::Fast => "Fast pace",
    }
}

/// A pace cell's answer.
fn pace_word(pace: Pace) -> &'static str {
    match pace {
        Pace::Slow => "Slow",
        Pace::Medium => "Medium",
        Pace::Fast => "Fast",
    }
}

/// A "Driven by" cell's answer.
fn drive_word(drive: Drive) -> &'static str {
    match drive {
        Drive::Plot => "Plot",
        Drive::Mix => "A mix",
        Drive::Character => "Character",
    }
}

/// A character trait as the Characters line names it.
fn trait_word(side: CharacterTrait) -> &'static str {
    match side {
        CharacterTrait::Loveable => "Loveable",
        CharacterTrait::Unloveable => "Unloveable",
        CharacterTrait::StrongDevelopment => "Strong development",
        CharacterTrait::WeakDevelopment => "Weak development",
        CharacterTrait::Diverse => "Diverse",
        CharacterTrait::NotDiverse => "Not diverse",
    }
}

/// Each count's share of their total in whole percents, adding up to
/// exactly 100 (largest remainder: the floors first, then a point each to
/// the largest remainders, earlier first on a tie). All zeros with no
/// votes.
pub fn shares(counts: &[u32]) -> Vec<u32> {
    let total: u64 = counts.iter().map(|&c| u64::from(c)).sum();
    if total == 0 {
        return vec![0; counts.len()];
    }
    let mut out: Vec<u32> = Vec::with_capacity(counts.len());
    let mut remainders: Vec<(u64, usize)> = Vec::with_capacity(counts.len());
    for (ix, &count) in counts.iter().enumerate() {
        let scaled = u64::from(count) * 100;
        out.push(u32::try_from(scaled / total).unwrap_or(100));
        remainders.push((scaled % total, ix));
    }
    let left = 100 - out.iter().sum::<u32>();
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    for &(_, ix) in remainders.iter().take(left as usize) {
        out[ix] += 1;
    }
    out
}

/// A mood meter's length: its votes against the first (most voted) mood's.
pub fn mood_meter_percent(votes: u32, most: u32) -> f32 {
    if most == 0 {
        return 0.;
    }
    #[expect(clippy::cast_precision_loss, reason = "vote counts")]
    let percent = votes as f32 / most as f32 * 100.;
    percent
}

/// The Rarely line: "Rarely dark.", "Rarely dark, mysterious or sad.".
pub fn rarely_text(moods: &[Mood]) -> Option<String> {
    let words: Vec<String> = moods.iter().map(|m| m.name().to_lowercase()).collect();
    match words.as_slice() {
        [] => None,
        [one] => Some(format!("Rarely {one}.")),
        [rest @ .., last] => Some(format!("Rarely {} or {last}.", rest.join(", "))),
    }
}

/// A character trait readers agree on, with its share of the pair's votes:
/// ("Loveable", 92).
pub fn character_share(lean: &CharacterLean) -> (&'static str, u32) {
    let share = shares(&[lean.votes, lean.pair_votes - lean.votes])[0];
    (trait_word(lean.side), share)
}

/// The book header's mood line. See the module doc.
#[derive(IntoElement)]
pub struct MoodLine {
    book: BookId,
    line: tags::MoodLine,
    breakdown: Rc<VoteBreakdown>,
}

impl MoodLine {
    /// The line for `book`'s `votes`, or `None` under 50 mood votes: no
    /// line and no "Votes".
    pub fn new(book: BookId, votes: &BookVotes) -> Option<Self> {
        let line = tags::mood_line(votes)?;
        Some(Self {
            book,
            line,
            breakdown: Rc::new(tags::breakdown(votes)),
        })
    }
}

fn separator(cx: &App) -> impl IntoElement {
    div()
        .flex_none()
        .text_color(colors(cx).fg_muted)
        .child(" · ")
}

/// A mood that opens its page, as a text button in `body`.
fn mood_button(id: impl Into<ElementId>, mood: Mood) -> TextButton {
    TextButton::new(id, mood.name())
        .opens(Page::Tag {
            tag: Tag::Mood(mood),
            within: None,
        })
        .body()
}

impl RenderOnce for MoodLine {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let book = self.book.get().cast_unsigned();
        // Kept by the app, so focus can go back to "Votes" and on to the
        // first mood whatever opened the popover.
        let trigger = window
            .use_keyed_state(("votes-trigger-focus", book), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let first_mood = window
            .use_keyed_state(("votes-first-mood", book), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let has_moods = !self.breakdown.moods.is_empty();
        let palette = colors(cx);
        let max_h =
            window.viewport_size().height - size::POPOVER_MARGIN.to_pixels(window.rem_size());
        let breakdown = self.breakdown.clone();
        let close_to = trigger.clone();
        let panel_focus = first_mood.clone();
        let popover = Popover::new(("votes-popover", book))
            .anchor(Anchor::TopLeft)
            .trigger(
                Button::new(("votes", book))
                    .ghost()
                    .small_control()
                    .label("Votes")
                    .dropdown_caret(true)
                    .keyboard_ring(&trigger, window),
            )
            .when(has_moods, |this| this.track_focus(&first_mood))
            .on_open_change(move |open, window, cx| {
                if !*open {
                    window.focus(&close_to, cx);
                }
            })
            .p_4()
            .rounded(box_radius(cx))
            .bg(palette.raised)
            .shadow(palette.shadow_popover())
            .content(move |_, _, cx| {
                let popover = cx.entity().downgrade();
                div()
                    .id("votes-scroll")
                    .max_h(max_h)
                    .overflow_y_scroll()
                    .child(
                        VotesPanel::new(breakdown.clone())
                            .first_mood_focus(&panel_focus)
                            .on_choose(move |window, cx| {
                                let _ = popover.update(cx, |p, cx| p.dismiss(window, cx));
                            }),
                    )
            });

        let moods = self.line.moods.iter().enumerate().map(|(ix, mood)| {
            h_flex()
                .flex_none()
                .gap_1()
                .when(ix > 0, |this| this.child(separator(cx)))
                .child(mood_button(
                    ("mood-line", mood.id().get().cast_unsigned()),
                    *mood,
                ))
        });
        let end = h_flex()
            .flex_none()
            .items_center()
            .gap_1()
            .when_some(self.line.pace, |this, pace| {
                this.child(separator(cx))
                    .child(div().text_color(palette.fg).child(pace_line_word(pace)))
            })
            .child(popover);
        #[cfg(test)]
        let end = end.debug_selector(|| "votes-end".into());
        h_flex()
            .id(("mood-line", book))
            .role(Role::Group)
            .aria_label("Moods from Hardcover readers")
            .flex_wrap()
            .items_center()
            .gap_x_1()
            .body()
            .children(moods)
            .child(end)
    }
}

type Choose = Rc<dyn Fn(&mut Window, &mut App)>;

/// Every figure behind a book's mood line, as the Votes popover shows it
/// (the gallery shows it alone). See the module doc.
#[derive(IntoElement)]
pub struct VotesPanel {
    breakdown: Rc<VoteBreakdown>,
    first_mood: Option<FocusHandle>,
    on_choose: Option<Choose>,
}

impl VotesPanel {
    pub fn new(breakdown: Rc<VoteBreakdown>) -> Self {
        Self {
            breakdown,
            first_mood: None,
            on_choose: None,
        }
    }

    /// The first mood takes its focus from `handle`, which the popover
    /// focuses when it opens.
    pub fn first_mood_focus(mut self, handle: &FocusHandle) -> Self {
        self.first_mood = Some(handle.clone());
        self
    }

    /// Runs before a chosen mood's page opens: the popover closes.
    pub fn on_choose(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_choose = Some(Rc::new(handler));
        self
    }
}

/// A group of the panel: its `label`, 6 above its content.
fn group(label: impl IntoElement, content: impl IntoElement) -> impl IntoElement {
    v_flex().w_full().gap_1p5().child(label).child(content)
}

/// A three-way answer (pace, drive): three equal cells 12 apart, in scale
/// order, each its answer, its share and a muted meter; the leading
/// answer's name in ink at 500.
fn three_way<T: Copy + PartialEq + 'static>(
    id: &'static str,
    split: &Split<T>,
    word: fn(T) -> &'static str,
    meter_label: fn(T, u32) -> String,
    cx: &App,
) -> impl IntoElement {
    let palette = colors(cx);
    let counts: Vec<u32> = split.parts.iter().map(|(_, n)| *n).collect();
    let percents = shares(&counts);
    h_flex()
        .w_full()
        .gap_3()
        .children(
            split
                .parts
                .iter()
                .zip(percents)
                .enumerate()
                .map(|(ix, ((part, _), percent))| {
                    let lead = *part == split.leader;
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(if lead {
                                            palette.fg
                                        } else {
                                            palette.fg_muted
                                        })
                                        .when(lead, |this| this.font_weight(FontWeight::MEDIUM))
                                        .child(word(*part)),
                                )
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .flex_none()
                                        .caption(cx)
                                        .tabular()
                                        .child(format!("{percent}%")),
                                ),
                        )
                        .child(
                            #[expect(clippy::cast_precision_loss, reason = "a percentage")]
                            Meter::new((id, ix), percent as f32, meter_label(*part, percent))
                                .muted(),
                        )
                }),
        )
}

impl RenderOnce for VotesPanel {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let palette = colors(cx);
        let b = &self.breakdown;
        let most = b.moods.first().map_or(0, |(_, n)| *n);
        let choose = self.on_choose.clone();
        let moods = (!b.moods.is_empty()).then(|| {
            group(
                h_flex()
                    .w_full()
                    .justify_between()
                    .child(div().label(cx).child("Moods"))
                    .child(div().caption(cx).child("Votes")),
                v_flex()
                    .w_full()
                    .gap_1()
                    .children(b.moods.iter().enumerate().map(|(ix, (mood, votes))| {
                        let mood = *mood;
                        let choose = choose.clone();
                        let button = TextButton::new(
                            ("votes-mood", mood.id().get().cast_unsigned()),
                            mood.name(),
                        )
                        .body()
                        .on_click(move |_, window, cx| {
                            if let Some(choose) = &choose {
                                choose(window, cx);
                            }
                            navigation::open(
                                Page::Tag {
                                    tag: Tag::Mood(mood),
                                    within: None,
                                },
                                cx,
                            );
                        });
                        let button = match (&self.first_mood, ix) {
                            (Some(focus), 0) => button.track_focus(focus),
                            _ => button,
                        };
                        h_flex()
                            .w_full()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .w(size::VOTES_NAME)
                                    .flex_none()
                                    .min_w_0()
                                    .child(button),
                            )
                            .child(
                                Meter::new(
                                    ("votes-mood-meter", mood.id().get().cast_unsigned()),
                                    mood_meter_percent(*votes, most),
                                    format!("{}: {} votes", mood.name(), count_text(*votes)),
                                )
                                .muted()
                                .flex_1(),
                            )
                            .child(
                                div()
                                    .w(size::VOTES_COUNT)
                                    .flex_none()
                                    .caption(cx)
                                    .tabular()
                                    .text_right()
                                    .child(count_text(*votes)),
                            )
                    })),
            )
        });
        let pace = b.pace.as_ref().map(|split| {
            group(
                div().label(cx).child("Pace"),
                three_way(
                    "votes-pace",
                    split,
                    pace_word,
                    |p, percent| format!("{}: {percent}%", pace_line_word(p)),
                    cx,
                ),
            )
        });
        let drive = b.drive.as_ref().map(|split| {
            group(
                div().label(cx).child("Driven by"),
                three_way(
                    "votes-drive",
                    split,
                    drive_word,
                    |d, percent| format!("Driven by {}: {percent}%", drive_word(d).to_lowercase()),
                    cx,
                ),
            )
        });
        let characters = (!b.characters.is_empty()).then(|| {
            let parts = b.characters.iter().enumerate().map(|(ix, lean)| {
                let (word, share) = character_share(lean);
                h_flex()
                    .flex_none()
                    .gap_1()
                    .when(ix > 0, |this| this.child(separator(cx)))
                    .child(div().text_color(palette.fg).child(word))
                    .child(format!("{share}%"))
            });
            group(
                div().label(cx).child("Characters"),
                h_flex()
                    .flex_wrap()
                    .gap_x_1()
                    .tabular()
                    .text_color(palette.fg_muted)
                    .children(parts),
            )
        });
        let rarely = rarely_text(&b.rarely);
        v_flex()
            .w(size::VOTES_POPOVER)
            .body()
            .child(
                div()
                    .caption(cx)
                    .pb_3()
                    .child(SharedString::new_static("Votes from Hardcover readers")),
            )
            .child(
                v_flex()
                    .gap_4()
                    .children(moods)
                    .children(pace)
                    .children(drive)
                    .children(characters),
            )
            .when_some(rarely, |this, rarely| {
                this.child(div().my_3().h(size::HAIRLINE).bg(palette.border))
                    .child(div().caption(cx).child(rarely))
            })
    }
}

/// The line's parts, for tests: the moods named and the pace word.
#[cfg(test)]
pub fn line_words(votes: &BookVotes) -> Option<(Vec<&'static str>, Option<&'static str>)> {
    let line = tags::mood_line(votes)?;
    Some((
        line.moods.iter().map(|m| m.name()).collect(),
        line.pace.map(pace_line_word),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_add_up_to_a_hundred() {
        // Naive rounding gives 33 + 33 + 33.
        assert_eq!(shares(&[1, 1, 1]), vec![34, 33, 33]);
        // Naive rounding gives 13 + 58 + 29 = 100 here, but 2/3 split
        // rounding up twice would give 101.
        assert_eq!(shares(&[1, 2, 0]), vec![33, 67, 0]);
        assert_eq!(shares(&[58, 259, 133]).iter().sum::<u32>(), 100);
        assert_eq!(shares(&[130, 232, 43]).iter().sum::<u32>(), 100);
        assert_eq!(shares(&[0, 0, 0]), vec![0, 0, 0]);
        for a in 0..30 {
            for b in 0..30 {
                let c = 30 - a.min(30);
                let s = shares(&[a, b, c]);
                if a + b + c > 0 {
                    assert_eq!(s.iter().sum::<u32>(), 100, "{a} {b} {c}");
                }
            }
        }
    }

    #[test]
    fn a_mood_meter_is_against_the_first_mood() {
        assert!((mood_meter_percent(422, 422) - 100.).abs() < f32::EPSILON);
        assert!((mood_meter_percent(211, 422) - 50.).abs() < f32::EPSILON);
        assert!(mood_meter_percent(5, 0).abs() < f32::EPSILON);
    }

    #[test]
    fn rarely_reads_as_a_sentence() {
        assert_eq!(rarely_text(&[]), None);
        assert_eq!(rarely_text(&[Mood::Dark]).as_deref(), Some("Rarely dark."));
        assert_eq!(
            rarely_text(&[Mood::Dark, Mood::Mysterious, Mood::Sad]).as_deref(),
            Some("Rarely dark, mysterious or sad.")
        );
        assert_eq!(
            rarely_text(&[Mood::Dark, Mood::Sad]).as_deref(),
            Some("Rarely dark or sad.")
        );
    }

    #[test]
    fn the_copy_is_the_design_systems() {
        assert_eq!(pace_line_word(Pace::Slow), "Slow pace");
        assert_eq!(drive_word(Drive::Mix), "A mix");
        assert_eq!(trait_word(CharacterTrait::NotDiverse), "Not diverse");
        let votes = BookVotes::from_counts([
            (CharacterTrait::Loveable.id(), 379),
            (CharacterTrait::Unloveable.id(), 31),
        ]);
        let leans = tags::breakdown(&votes).characters;
        assert_eq!(character_share(&leans[0]), ("Loveable", 92));
    }
}

#[cfg(test)]
mod ui_tests {
    use std::cell::RefCell;

    use gpui_kit::{
        Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext, VisualTestContext,
    };
    use hardcover_core::BookDetails;

    use super::*;
    use crate::catalog::navigation::{NavigationEvent, Navigator};
    use crate::ui::test_support::{host, press, redraw, tab};

    /// Project Hail Mary in the demo: "Hopeful · Funny · Adventurous", no
    /// pace, and Rarely dark.
    const HAIL_MARY: BookId = BookId::new(427_578);

    fn details() -> BookDetails {
        hardcover_core::demo::client()
            .book_details(HAIL_MARY)
            .expect("the demo's book")
    }

    struct Fixture<'a> {
        cx: &'a mut VisualTestContext,
        opened: Rc<RefCell<Vec<Page>>>,
        backs: Rc<std::cell::Cell<usize>>,
    }

    /// The line, inside a page that goes Back on Escape, as the shell's
    /// does.
    fn fixture(cx: &mut TestAppContext) -> Fixture<'_> {
        let votes = details().votes;
        let opened: Rc<RefCell<Vec<Page>>> = Rc::default();
        let backs = Rc::new(std::cell::Cell::new(0));
        cx.update(|cx| {
            cx.bind_keys([gpui_kit::KeyBinding::new(
                "escape",
                crate::GoBack,
                Some(crate::shell::PAGE_CONTEXT),
            )]);
            let navigator = Navigator::init(cx);
            let opened = opened.clone();
            cx.subscribe(&navigator, move |_, event: &NavigationEvent, _| {
                if let NavigationEvent::Open(page) = event {
                    opened.borrow_mut().push(page.clone());
                }
            })
            .detach();
        });
        let counted = backs.clone();
        let cx = host(cx, move |_, _| {
            let counted = counted.clone();
            div()
                .id("page")
                .key_context(crate::shell::PAGE_CONTEXT)
                .on_action(move |_: &crate::GoBack, _, _| counted.set(counted.get() + 1))
                .size_full()
                .child(MoodLine::new(HAIL_MARY, &votes).expect("a line"))
                .into_any_element()
        });
        cx.update(|window, _| window.activate_window());
        Fixture { cx, opened, backs }
    }

    fn settle(cx: &mut VisualTestContext) {
        redraw(cx);
        cx.run_until_parked();
        redraw(cx);
    }

    fn key(cx: &mut VisualTestContext, key: &str) {
        press(cx, key);
        settle(cx);
    }

    fn first_mood() -> Mood {
        tags::breakdown(&details().votes).moods[0].0
    }

    fn mood_page(mood: Mood) -> Page {
        Page::Tag {
            tag: Tag::Mood(mood),
            within: None,
        }
    }

    #[test]
    fn the_demo_line_is_the_design_systems() {
        let votes = details().votes;
        assert_eq!(
            line_words(&votes),
            Some((vec!["Hopeful", "Funny", "Adventurous"], None))
        );
        assert_eq!(
            rarely_text(&tags::breakdown(&votes).rarely).as_deref(),
            Some("Rarely dark.")
        );
    }

    #[gpui_kit::test]
    fn the_line_s_moods_open_their_pages(cx: &mut TestAppContext) {
        let f = fixture(cx);
        tab(f.cx);
        key(f.cx, "enter");
        assert_eq!(*f.opened.borrow(), vec![mood_page(Mood::Hopeful)]);
    }

    #[gpui_kit::test]
    fn enter_opens_the_votes_on_the_first_mood_and_escape_comes_back(cx: &mut TestAppContext) {
        let f = fixture(cx);
        // Hopeful, Funny, Adventurous, then Votes.
        for _ in 0..4 {
            tab(f.cx);
        }
        key(f.cx, "enter");
        // Focus is on the popover's first mood: Enter presses it.
        key(f.cx, "enter");
        assert_eq!(*f.opened.borrow(), vec![mood_page(first_mood())]);

        // It closed onto "Votes": Enter opens it again, Escape closes it
        // without going Back, and Enter on "Votes" opens it once more.
        key(f.cx, "enter");
        key(f.cx, "escape");
        assert_eq!(f.backs.get(), 0, "Escape closes the popover only");
        key(f.cx, "enter");
        key(f.cx, "enter");
        assert_eq!(f.opened.borrow().len(), 2, "back on Votes, then the mood");
    }

    #[gpui_kit::test]
    fn a_click_opens_it_and_escape_gives_focus_to_votes(cx: &mut TestAppContext) {
        let f = fixture(cx);
        let bounds = f.cx.debug_bounds("votes-end").expect("Votes is drawn");
        let position = bounds.center();
        f.cx.simulate_event(MouseDownEvent {
            position,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 1,
            first_mouse: false,
        });
        f.cx.simulate_event(MouseUpEvent {
            position,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 1,
        });
        settle(f.cx);
        key(f.cx, "escape");
        assert_eq!(f.backs.get(), 0);
        // Focus is on "Votes", though a press never focuses a button.
        key(f.cx, "enter");
        key(f.cx, "enter");
        assert_eq!(*f.opened.borrow(), vec![mood_page(first_mood())]);
    }
}
