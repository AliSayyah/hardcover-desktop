//! The dialog that adds a past read or changes a read's dates.
//!
//! It only collects and checks the dates; the detail view makes the write when
//! the dialog reports [`ReadDatesChosen`], then closes it.

use crate::ui::kit::open_dialog;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::calendar::Date as CalendarDate;
use gpui_kit::component::date_picker::{DatePicker, DatePickerEvent, DatePickerState};
use gpui_kit::component::dialog::{Dialog, DialogFooter};
use gpui_kit::component::form::{Field, Form};
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, v_flex};
use gpui_kit::{
    App, AppContext as _, Axis, Context, Entity, EventEmitter, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
    rems,
};
use hardcover_core::{Date, ReadId, ReadSession};

/// What the dialog is for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ReadDialogKind {
    /// A new, finished read (a re-read, or one from before the account).
    Add,
    /// Changing the dates of an existing read.
    Edit {
        read: ReadId,
        /// A finished read keeps a finish date, so it never becomes a second open read.
        finished: bool,
    },
}

/// The dates the user confirmed.
pub struct ReadDatesChosen {
    pub kind: ReadDialogKind,
    pub started: Option<Date>,
    pub finished: Option<Date>,
}

pub struct ReadDialog {
    kind: ReadDialogKind,
    started: Entity<DatePickerState>,
    finished: Entity<DatePickerState>,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ReadDatesChosen> for ReadDialog {}

impl ReadDialog {
    pub fn new(
        kind: ReadDialogKind,
        read: Option<&ReadSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let picker = |date: Option<Date>, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut state = DatePickerState::new(window, cx).date_format("%b %-d, %Y");
                state.set_date(CalendarDate::Single(date), window, cx);
                state
            })
        };
        let started = picker(read.and_then(|r| r.started_at), window, cx);
        let finished = picker(read.and_then(|r| r.finished_at), window, cx);
        let clear_error = |this: &mut Self,
                           _: Entity<DatePickerState>,
                           _: &DatePickerEvent,
                           cx: &mut Context<Self>| {
            if this.error.take().is_some() {
                cx.notify();
            }
        };
        let subscriptions = vec![
            cx.subscribe(&started, clear_error),
            cx.subscribe(&finished, clear_error),
        ];
        Self {
            kind,
            started,
            finished,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    /// Opens `view` in a dialog.
    pub fn open(view: Entity<Self>, window: &mut Window, cx: &mut App) {
        let (title, confirm) = match view.read(cx).kind {
            ReadDialogKind::Add => ("Add a read", "Add"),
            ReadDialogKind::Edit { .. } => ("Change read dates", "Save"),
        };
        open_dialog(window, cx, move |dialog: Dialog, window, _| {
            let confirm_view = view.clone();
            let ok_view = view.clone();
            dialog
                .title(title)
                .w(rems(24.).to_pixels(window.rem_size()))
                .child(view.clone())
                .on_ok(move |_, _, cx| ok_view.update(cx, |this, cx| this.confirm(cx)))
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("read-dialog-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("read-dialog-confirm")
                                .primary()
                                .label(confirm)
                                .on_click(move |_, window, cx| {
                                    if confirm_view.update(cx, |this, cx| this.confirm(cx)) {
                                        window.close_dialog(cx);
                                    }
                                }),
                        ),
                )
        });
    }

    /// Checks the dates and reports them. `false` (with an error shown) when
    /// they can't be saved.
    fn confirm(&mut self, cx: &mut Context<Self>) -> bool {
        let started = self.started.read(cx).date().start();
        let finished = self.finished.read(cx).date().start();
        let needs_finish = match self.kind {
            ReadDialogKind::Add => true,
            ReadDialogKind::Edit { finished, .. } => finished,
        };
        let error = match (started, finished) {
            (_, None) if needs_finish => Some("Choose the date you finished."),
            (Some(s), Some(f)) if f < s => Some("The finish date can't be before the start date."),
            _ => None,
        };
        if let Some(error) = error {
            self.error = Some(error.into());
            cx.notify();
            return false;
        }
        cx.emit(ReadDatesChosen {
            kind: self.kind,
            started,
            finished,
        });
        true
    }
}

impl Render for ReadDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let needs_finish = match self.kind {
            ReadDialogKind::Add => true,
            ReadDialogKind::Edit { finished, .. } => finished,
        };
        let label_width = window.rem_size() * 5.;
        v_flex()
            .gap_3()
            .child(
                Form::new()
                    .label_layout(Axis::Horizontal)
                    .label_width(label_width)
                    .child(
                        Field::new().label("Started").child(
                            DatePicker::new(&self.started)
                                .placeholder("Not set")
                                .cleanable(true),
                        ),
                    )
                    .child(
                        Field::new().label("Finished").child(
                            DatePicker::new(&self.finished)
                                .placeholder(if needs_finish { "Required" } else { "Not set" })
                                .cleanable(!needs_finish),
                        ),
                    ),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
    }
}
