//! A status shelf's filter bar (`hardcover-design`, The filter bar):
//! narrowing the reader's own shelf by genre and mood, to choose the next
//! book.
//!
//! - **Two rows,** 6 apart, each 24 high: the genres, then the moods the
//!   shelf's books have, as [`FilterChip`]s 6 apart, as many as fit on one
//!   line (`tag_filter::fit_row`), then a small ghost "More" menu with the
//!   rest ("More genres", "More moods" to a screen reader). A row with
//!   nothing to offer hides.
//! - **Picks:** one genre and one mood at most, kept per shelf for the
//!   session; the shelf keeps the books with both ([`PickChanged`] tells the
//!   shelf). A pick is ignored while the bar is hidden (no `read:catalog`),
//!   so the shelf is never filtered by chips the reader can't see.
//! - **States:** skeleton chips until the shelf's tags first land; one
//!   muted line with "Try again" when they failed (beside the chips when
//!   some books' tags are known).
//! - **Keys:** each row is one Tab stop (`ui::chip_keys`); Left and Right
//!   move along it, onto its "More" too. A press keeps focus on the chip;
//!   a chip that leaves the row as it is pressed (a choice from "More",
//!   cleared) hands focus to the row's first.
//!
//! The bar measures its own width as it is laid out: the shelf's list is
//! resizable.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, EventEmitter, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString,
    Styled as _, Subscription, WeakEntity, Window, canvas, div, prelude::FluentBuilder as _, rems,
};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::tags::Mood;
use hardcover_core::{BookId, ReadingStatus};

use super::model::LibraryModel;
use super::shelf_tags::{ShelfTags, ShelfTagsModel};
use super::tag_filter::{Chip, Facets, TagPick, facets, fit_row};
use crate::chip::genre::genre_icon;
use crate::chip::{FilterChip, filter_chip_width, small_menu_button_width};
use crate::design::{TypeRoles as _, colors, size};
use crate::session;
use crate::ui::chip_keys::ChipKeys;
use crate::ui::kit::SmallControl as _;
use crate::ui::{LoadFailed, Skeleton, refresh_after_draw};

/// The shelf's picks changed: its rows must be worked out again.
pub struct PickChanged;

/// Which row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Row {
    Genres,
    Moods,
}

impl Row {
    fn more_name(self) -> &'static str {
        match self {
            Row::Genres => "More genres",
            Row::Moods => "More moods",
        }
    }
}

/// What the facets were worked out from.
type FacetsKey = (u64, u64, ReadingStatus, TagPick);

pub struct FilterBar {
    library: Entity<LibraryModel>,
    tags: Entity<ShelfTagsModel>,
    status: ReadingStatus,
    picks: HashMap<ReadingStatus, TagPick>,
    genre_keys: Entity<ChipKeys>,
    mood_keys: Entity<ChipKeys>,
    /// The bar's width as last laid out.
    width: Option<Pixels>,
    /// The chip last pressed, with its row, so focus stays in the row when
    /// the chip leaves it.
    pressed: Option<(Row, ElementId)>,
    /// Each chip's focus, kept here so the bar can tell which chip has it
    /// and move it.
    focus: HashMap<ElementId, FocusHandle>,
    /// The last facets worked out, and from what. Filled on read, so
    /// `render` and the shelf can ask without updating the bar.
    facets: RefCell<Option<(FacetsKey, Rc<Facets>)>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PickChanged> for FilterBar {}

impl FilterBar {
    pub fn new(
        library: Entity<LibraryModel>,
        tags: Entity<ShelfTagsModel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&library, |_, _, cx| cx.notify()),
            cx.observe(&tags, |_, _, cx| cx.notify()),
            // A grant gained or lost shows or hides the bar, and with it
            // whether the picks apply.
            session::observe_grant(cx, |_, cx| {
                cx.emit(PickChanged);
                cx.notify();
            }),
        ];
        Self {
            library,
            tags,
            status: ReadingStatus::CurrentlyReading,
            picks: HashMap::new(),
            genre_keys: cx.new(|cx| ChipKeys::new(window, cx)),
            mood_keys: cx.new(|cx| ChipKeys::new(window, cx)),
            width: None,
            pressed: None,
            focus: HashMap::new(),
            facets: RefCell::new(None),
            _subscriptions: subscriptions,
        }
    }

    /// The shelf shown.
    pub fn set_status(&mut self, status: ReadingStatus, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            self.pressed = None;
            cx.notify();
        }
    }

    fn available(cx: &App) -> bool {
        session::has_scope(cx, READ_CATALOG)
    }

    /// The chips picked on the shelf shown; none while the bar is hidden.
    pub fn pick(&self, cx: &App) -> TagPick {
        if !Self::available(cx) {
            return TagPick::default();
        }
        self.picks.get(&self.status).cloned().unwrap_or_default()
    }

    /// Clears the shelf's picks ("Clear filters").
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        if self.picks.remove(&self.status).is_some() {
            cx.emit(PickChanged);
            cx.notify();
        }
    }

    fn update_pick(
        &mut self,
        row: Row,
        id: ElementId,
        edit: impl FnOnce(&mut TagPick),
        cx: &mut Context<Self>,
    ) {
        let pick = self.picks.entry(self.status).or_default();
        edit(pick);
        if pick.is_empty() {
            self.picks.remove(&self.status);
        }
        self.pressed = Some((row, id));
        cx.emit(PickChanged);
        cx.notify();
    }

    fn focus_of(&mut self, id: &ElementId, cx: &mut App) -> FocusHandle {
        self.focus
            .entry(id.clone())
            .or_insert_with(|| cx.focus_handle())
            .clone()
    }

    fn shelf_books(&self, cx: &App) -> Vec<BookId> {
        self.library
            .read(cx)
            .entries()
            .iter()
            .filter(|e| e.status == self.status)
            .map(|e| e.book.id)
            .collect()
    }

    /// Where the shelf's tags stand.
    fn state(&self, cx: &App) -> ShelfTags {
        self.tags.read(cx).shelf(&self.shelf_books(cx))
    }

    /// The rows' chips for the shelf shown, under its pick.
    fn facets(&self, cx: &App) -> Rc<Facets> {
        let pick = self.pick(cx);
        let tags = self.tags.read(cx);
        let key = (
            self.library.read(cx).revision(),
            tags.revision(),
            self.status,
            pick.clone(),
        );
        if let Some((k, facets)) = &*self.facets.borrow()
            && *k == key
        {
            return facets.clone();
        }
        let books = self.shelf_books(cx);
        let computed = Rc::new(facets(books.iter().map(|b| tags.facts(*b)), &pick));
        *self.facets.borrow_mut() = Some((key, computed.clone()));
        computed
    }

    /// Whether the bar has anything to show: hidden without `read:catalog`,
    /// and when neither row has a chip, nothing is loading and nothing
    /// failed.
    pub fn is_shown(&self, cx: &App) -> bool {
        match self.state(cx) {
            ShelfTags::Unavailable => false,
            ShelfTags::Loading | ShelfTags::Failed(_) => true,
            ShelfTags::Ready { failure } => {
                let facets = self.facets(cx);
                failure.is_some() || !facets.genres.is_empty() || !facets.moods.is_empty()
            }
        }
    }

    /// The foot's line: while a chip is picked, how many of the shelf's
    /// books no chip finds.
    pub fn foot(&self, cx: &App) -> Option<String> {
        if self.pick(cx).is_empty() {
            return None;
        }
        let unfindable = self.facets(cx).unfindable;
        (unfindable > 0).then(|| super::tag_filter::unfindable_line(unfindable))
    }

    // ----- drawing -----

    fn skeleton() -> impl IntoElement {
        let chip = |w: f32| Skeleton::line().w(rems(w)).h(size::CHIP).rounded_full();
        let row = |widths: &[f32]| {
            h_flex()
                .h(size::CONTROL_SMALL)
                .gap_1p5()
                .children(widths.iter().map(|w| chip(*w)))
        };
        v_flex()
            .gap_1p5()
            .child(row(&[4.5, 3.75, 5.]))
            .child(row(&[5.5, 4., 4.5]))
    }

    fn render_row<T: Clone + 'static>(
        &mut self,
        row: Row,
        chips: &[Chip<T>],
        chip_id: fn(&T) -> ElementId,
        press: fn(&mut TagPick, &Chip<T>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if chips.is_empty() {
            return None;
        }
        let gap = window.rem_size() * (6. / 16.);
        let available = self
            .width
            .unwrap_or_else(|| (size::LIST - rems(2.)).to_pixels(window.rem_size()));
        // Each chip as it is drawn unpressed. A press puts a check in a
        // chip without an icon, which widens it: that check's room is kept
        // free from the start, so pressing a chip never pushes another into
        // "More".
        let has_icon = |c: &Chip<T>| row == Row::Genres && genre_icon(&c.name).is_some();
        let widths: Vec<f32> = chips
            .iter()
            .map(|c| f32::from(filter_chip_width(&c.name, c.count, has_icon(c), window)))
            .collect();
        let check_room = if chips.iter().any(|c| !has_icon(c)) {
            // The check and its gap, 12 + 4: one rem.
            window.rem_size()
        } else {
            Pixels::ZERO
        };
        let more_width = f32::from(small_menu_button_width("More", window));
        let selected = chips.iter().position(|c| c.selected);
        let fit = fit_row(
            &widths,
            selected,
            f32::from(available - check_room),
            f32::from(gap),
            more_width,
        );

        // A chip pressed from the keyboard that left the row: focus goes to
        // the row's first.
        if let Some((pressed_row, id)) = self.pressed.clone()
            && pressed_row == row
        {
            let shown = fit.shown.iter().any(|ix| chip_id(&chips[*ix].value) == id);
            if !shown {
                let pressed = self.focus_of(&id, cx);
                if pressed.is_focused(window)
                    && let Some(first) = fit.shown.first()
                {
                    let first = self.focus_of(&chip_id(&chips[*first].value), cx);
                    window.defer(cx, move |window, cx| window.focus(&first, cx));
                }
            }
            self.pressed = None;
        }

        let this = cx.entity().downgrade();
        let shown: Vec<_> = fit
            .shown
            .iter()
            .map(|ix| {
                let chip = chips[*ix].clone();
                let id = chip_id(&chip.value);
                let focus = self.focus_of(&id, cx);
                let element = match row {
                    Row::Genres => FilterChip::genre(id.clone(), chip.name.clone(), chip.count),
                    Row::Moods => FilterChip::mood(id.clone(), chip.name.clone(), chip.count),
                };
                element
                    .selected(chip.selected)
                    .focus(focus)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.update_pick(row, id.clone(), |pick| press(pick, &chip), cx);
                    }))
            })
            .collect();
        let more = (!fit.more.is_empty()).then(|| {
            let rest: Vec<Chip<T>> = fit.more.iter().map(|ix| chips[*ix].clone()).collect();
            more_menu(row, rest, chip_id, press, this)
        });
        let keys = match row {
            Row::Genres => &self.genre_keys,
            Row::Moods => &self.mood_keys,
        };
        let id = match row {
            Row::Genres => "filter-genres",
            Row::Moods => "filter-moods",
        };
        Some(
            keys.read(cx)
                .wrap(div().id(id))
                .flex()
                .items_center()
                .h(size::CONTROL_SMALL)
                .min_w_0()
                .gap_1p5()
                .children(shown)
                .children(more)
                .into_any_element(),
        )
    }
}

/// "More": the row's other chips, each with its count on the trailing edge,
/// the picked one checked and one that would leave nothing disabled.
fn more_menu<T: Clone + 'static>(
    row: Row,
    rest: Vec<Chip<T>>,
    chip_id: fn(&T) -> ElementId,
    press: fn(&mut TagPick, &Chip<T>),
    bar: WeakEntity<FilterBar>,
) -> AnyElement {
    let id = match row {
        Row::Genres => "filter-more-genres",
        Row::Moods => "filter-more-moods",
    };
    Button::new(id)
        .ghost()
        .small_control()
        .label("More")
        .dropdown_caret(true)
        .accessibility_label(row.more_name())
        .dropdown_menu(move |menu, _, _| {
            let mut menu = menu.scrollable(true);
            for chip in &rest {
                let (name, count) = (chip.name.clone(), chip.count);
                let chip = chip.clone();
                let bar = bar.clone();
                menu = menu.item(
                    PopupMenuItem::element(move |_, cx| {
                        h_flex()
                            .w_full()
                            .gap_4()
                            .justify_between()
                            .child(div().min_w_0().truncate().child(name.clone()))
                            .child(
                                div()
                                    .flex_none()
                                    .tabular()
                                    .text_color(colors(cx).fg_muted)
                                    .child(crate::format::count_text(count)),
                            )
                    })
                    .checked(chip.selected)
                    .disabled(chip.disabled())
                    .on_click(move |_, _, cx| {
                        let id = chip_id(&chip.value);
                        let chip = chip.clone();
                        let _ = bar.update(cx, |bar, cx| {
                            bar.update_pick(row, id, |pick| press(pick, &chip), cx);
                        });
                    }),
                );
            }
            menu
        })
        .into_any_element()
}

fn genre_chip_id(key: &String) -> ElementId {
    ElementId::Name(SharedString::from(format!("filter-genre-{key}")))
}

fn mood_chip_id(mood: &Mood) -> ElementId {
    ElementId::Name(SharedString::from(format!("filter-mood-{}", mood.slug())))
}

fn press_genre(pick: &mut TagPick, chip: &Chip<String>) {
    pick.press_genre(&chip.value, &chip.name);
}

fn press_mood(pick: &mut TagPick, chip: &Chip<Mood>) {
    pick.press_mood(chip.value);
}

impl Render for FilterBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bar = cx.entity().downgrade();
        // The bar's width, measured as it is laid out; a change shows on
        // the next frame.
        let measure = canvas(
            move |bounds, window, cx| {
                let width = bounds.size.width;
                let _ = bar.update(cx, |bar, _| {
                    if bar.width != Some(width) {
                        bar.width = Some(width);
                        refresh_after_draw(window);
                    }
                });
            },
            |_, (), _, _| {},
        )
        .absolute()
        .inset_0();
        let root = v_flex()
            .relative()
            .w_full()
            .min_w_0()
            .gap_1p5()
            .child(measure);
        match self.state(cx) {
            ShelfTags::Unavailable => root,
            ShelfTags::Loading => root.child(Self::skeleton()),
            ShelfTags::Failed(message) => root.child(self.failure(message)),
            ShelfTags::Ready { failure } => {
                let facets = self.facets(cx);
                let genres = self.render_row(
                    Row::Genres,
                    &facets.genres,
                    genre_chip_id,
                    press_genre,
                    window,
                    cx,
                );
                let moods = self.render_row(
                    Row::Moods,
                    &facets.moods,
                    mood_chip_id,
                    press_mood,
                    window,
                    cx,
                );
                root.children(genres)
                    .children(moods)
                    .when_some(failure, |this, message| this.child(self.failure(message)))
            }
        }
    }
}

impl FilterBar {
    fn failure(&self, message: SharedString) -> LoadFailed {
        let (tags, status) = (self.tags.clone(), self.status);
        LoadFailed::new("filter-retry", message)
            .on_retry(move |_, _, cx| tags.update(cx, |t, cx| t.retry(status, cx)))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui_kit::component::button::Button;
    use gpui_kit::{TestAppContext, VisualTestContext};

    use super::*;
    use crate::session::SessionScope;
    use crate::ui::test_support::{host, press, redraw, tab};

    /// A button, the Want to read shelf's bar, and a button after it, with
    /// the demo's tags loaded.
    fn bar(cx: &mut TestAppContext) -> (&mut VisualTestContext, Entity<FilterBar>) {
        bar_at(rems(40.), cx)
    }

    /// [`bar`] at `width`.
    fn bar_at(
        width: gpui_kit::Rems,
        cx: &mut TestAppContext,
    ) -> (&mut VisualTestContext, Entity<FilterBar>) {
        let session = SessionScope::for_tests(hardcover_core::demo::client(), None);
        cx.update(|cx| session.install(cx));
        let library = cx.new(|_| LibraryModel::new(session.clone()));
        let tags = cx.new(|cx| ShelfTagsModel::new(session, library.clone(), cx));
        tags.update(cx, |t, cx| t.show_shelf(ReadingStatus::WantToRead, cx));
        library.update(cx, LibraryModel::start);
        cx.run_until_parked();

        let slot: Rc<RefCell<Option<Entity<FilterBar>>>> = Rc::default();
        let made = slot.clone();
        let cx = host(cx, move |window, cx| {
            let bar = made
                .borrow_mut()
                .get_or_insert_with(|| {
                    cx.new(|cx| {
                        let mut bar = FilterBar::new(library.clone(), tags.clone(), window, cx);
                        bar.set_status(ReadingStatus::WantToRead, cx);
                        bar
                    })
                })
                .clone();
            div()
                .w(width)
                .child(Button::new("before").label("Before"))
                .child(bar)
                .child(Button::new("after").label("After"))
                .into_any_element()
        });
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        redraw(cx);
        let bar = slot.borrow().clone().expect("drawn");
        (cx, bar)
    }

    fn settle(cx: &mut VisualTestContext) {
        redraw(cx);
        cx.run_until_parked();
        redraw(cx);
    }

    fn pick(bar: &Entity<FilterBar>, cx: &mut VisualTestContext) -> TagPick {
        cx.update(|_, cx| bar.read(cx).pick(cx))
    }

    /// Each row is one Tab stop: the arrows move along it, Enter presses a
    /// chip and keeps focus on it, Tab leaves for the next row, and coming
    /// back lands on the chip the row was left from.
    #[gpui_kit::test]
    fn a_chip_row_is_one_tab_stop_with_arrows_inside(cx: &mut TestAppContext) {
        let (cx, bar) = bar(cx);
        let facets = cx.update(|_, cx| bar.read(cx).facets(cx));
        assert!(facets.genres.len() >= 2 && !facets.moods.is_empty());
        let (first, second) = (&facets.genres[0], &facets.genres[1]);

        press(cx, "a");
        tab(cx);
        tab(cx);
        settle(cx);
        press(cx, "right");
        settle(cx);
        press(cx, "enter");
        settle(cx);
        let picked = pick(&bar, cx).genre.map(|g| g.key);
        assert_eq!(picked.as_ref(), Some(&second.value), "the second genre");

        // Focus stayed on it: pressing again clears it.
        press(cx, "enter");
        settle(cx);
        assert!(pick(&bar, cx).is_empty());

        press(cx, "home");
        settle(cx);
        press(cx, "space");
        settle(cx);
        let picked = pick(&bar, cx).genre.map(|g| g.key);
        assert_eq!(picked.as_ref(), Some(&first.value), "Home: the first genre");

        // Tab leaves the genres for the moods' first chip.
        press(cx, "right");
        press(cx, "tab");
        settle(cx);
        press(cx, "enter");
        settle(cx);
        assert_eq!(pick(&bar, cx).mood, Some(facets.moods[0].value));

        // Shift+Tab comes back to the genre chip it left from, the second.
        press(cx, "shift-tab");
        settle(cx);
        press(cx, "enter");
        settle(cx);
        let picked = pick(&bar, cx).genre.map(|g| g.key);
        assert_eq!(
            picked.as_ref(),
            Some(&second.value),
            "back on the chip it left"
        );
    }

    /// A chip picked from "More" goes first; cleared from the keyboard, it
    /// leaves the row, and focus goes to the row's first chip.
    #[gpui_kit::test]
    fn focus_stays_in_a_row_whose_pressed_chip_leaves_it(cx: &mut TestAppContext) {
        let (cx, bar) = bar_at(rems(14.), cx);
        let facets = cx.update(|_, cx| bar.read(cx).facets(cx));
        let last = facets.moods.last().expect("moods").value;
        assert_ne!(last, facets.moods[0].value);
        cx.update(|_, cx| {
            bar.update(cx, |bar, cx| {
                bar.picks.entry(ReadingStatus::WantToRead).or_default().mood = Some(last);
                cx.notify();
            });
        });
        settle(cx);

        // Into the genres, then the moods: the picked chip, drawn first.
        press(cx, "a");
        tab(cx);
        tab(cx);
        settle(cx);
        press(cx, "tab");
        settle(cx);
        press(cx, "enter");
        settle(cx);
        assert_eq!(pick(&bar, cx).mood, None, "cleared");
        press(cx, "enter");
        settle(cx);
        assert_eq!(
            pick(&bar, cx).mood,
            Some(facets.moods[0].value),
            "focus went to the row's first chip"
        );
    }

    /// A click toggles a chip; a disabled chip ignores it.
    #[gpui_kit::test]
    fn a_click_toggles_a_chip_but_not_a_disabled_one(cx: &mut TestAppContext) {
        let (cx, bar) = bar(cx);
        let click = |selector: String, cx: &mut VisualTestContext| {
            let bounds = cx
                .debug_bounds(Box::leak(selector.into_boxed_str()))
                .expect("the chip is drawn");
            cx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
            settle(cx);
        };
        let facets = cx.update(|_, cx| bar.read(cx).facets(cx));
        let first = facets.genres[0].value.clone();
        click(format!("filter-genre-{first}"), cx);
        let picked = pick(&bar, cx).genre.map(|g| g.key);
        assert_eq!(picked.as_ref(), Some(&first));
        click(format!("filter-genre-{first}"), cx);
        assert!(pick(&bar, cx).is_empty(), "a second click clears it");

        // A mood that leaves some drawn genre with no books.
        let found = facets.moods.iter().find_map(|mood| {
            cx.update(|_, cx| {
                bar.update(cx, |bar, cx| {
                    bar.picks.entry(ReadingStatus::WantToRead).or_default().mood = Some(mood.value);
                    cx.notify();
                });
            });
            settle(cx);
            let f = cx.update(|_, cx| bar.read(cx).facets(cx));
            f.genres[..2]
                .iter()
                .find(|g| g.disabled())
                .map(|g| g.value.clone())
        });
        let disabled = found.expect("a mood that disables a drawn genre");
        let before = pick(&bar, cx);
        click(format!("filter-genre-{disabled}"), cx);
        assert_eq!(pick(&bar, cx), before, "nothing happened");
    }
}
