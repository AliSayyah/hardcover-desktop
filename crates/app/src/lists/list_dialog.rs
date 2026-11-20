//! The list form: a new list (optionally with the book it was started from)
//! or an existing list's name, description, privacy and ranking.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::form::{Field, Form};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Axis, Context, Entity, IntoElement, ParentElement as _, Pixels, Render,
    SharedString, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _, rems,
};
use hardcover_core::auth::scopes::WRITE_LISTS;
use hardcover_core::{BookSummary, ListDraft, ListId, Privacy};

use super::model::ListsModel;
use crate::app_state::Services;
use crate::format::needs_permission;
use crate::ui::kit::RaisedField as _;
use crate::ui::kit::open_dialog;

fn dialog_width(window: &Window) -> Pixels {
    rems(30.).to_pixels(window.rem_size())
}

/// Opens the form for a new list. With `book`, the book goes on the new list.
pub fn open_new_list(
    lists: Entity<ListsModel>,
    book: Option<BookSummary>,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| ListDialog::new(lists, Mode::Create(book), window, cx));
    open_dialog(window, cx, move |dialog, window, _| {
        dialog
            .title("New list")
            .w(dialog_width(window))
            .child(view.clone())
    });
}

/// Opens the form for one of the model's lists.
pub fn open_edit_list(lists: Entity<ListsModel>, id: ListId, window: &mut Window, cx: &mut App) {
    let Some(name) = lists.read(cx).list(id).map(|l| l.name.clone()) else {
        return;
    };
    let title: SharedString = format!("Edit “{name}”").into();
    let view = cx.new(|cx| ListDialog::new(lists, Mode::Edit(id), window, cx));
    open_dialog(window, cx, move |dialog, window, _| {
        dialog
            .title(title.clone())
            .w(dialog_width(window))
            .child(view.clone())
    });
}

enum Mode {
    Create(Option<BookSummary>),
    Edit(ListId),
}

struct ListDialog {
    lists: Entity<ListsModel>,
    mode: Mode,
    name: Entity<InputState>,
    description: Entity<TextareaState>,
    /// `None` for a list whose setting this version doesn't know: it is
    /// sent back unchanged unless one is chosen.
    privacy: Option<Privacy>,
    ranked: bool,
    error: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl ListDialog {
    fn new(
        lists: Entity<ListsModel>,
        mode: Mode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let existing = match &mode {
            Mode::Edit(id) => lists.read(cx).list(*id).cloned(),
            Mode::Create(_) => None,
        };
        let name = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("e.g. Summer reading");
            if let Some(list) = &existing {
                state.set_value(list.name.clone(), window, cx);
            }
            state
        });
        let description = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .placeholder("What the list is for (optional)")
                .auto_grow(3, 8);
            if let Some(text) = existing.as_ref().and_then(|l| l.description.clone()) {
                state.set_value(text, window, cx);
            }
            state
        });
        let subscriptions = vec![
            cx.observe(&lists, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &name,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.save(window, cx),
                    InputEvent::Change => {
                        this.error = None;
                        cx.notify();
                    }
                    _ => {}
                },
            ),
        ];
        name.update(cx, |input, cx| input.focus(window, cx));
        Self {
            lists,
            mode,
            name,
            description,
            // A new list is public, as on hardcover.app.
            privacy: match &existing {
                Some(list) => list.privacy(),
                None => Some(Privacy::Public),
            },
            ranked: existing.as_ref().is_some_and(|l| l.ranked),
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn draft(&self, cx: &App) -> Result<ListDraft, &'static str> {
        let name = self.name.read(cx).value().trim().to_owned();
        if name.is_empty() {
            return Err("Enter a name.");
        }
        Ok(ListDraft {
            name,
            description: self.description.read(cx).value().to_string(),
            privacy: self.privacy,
            ranked: self.ranked,
        })
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = match self.draft(cx) {
            Ok(draft) => draft,
            Err(message) => {
                self.error = Some(message.into());
                cx.notify();
                return;
            }
        };
        let task = match &self.mode {
            Mode::Create(book) => {
                let book = book.as_ref().map(|b| b.id);
                self.lists.update(cx, |m, cx| m.create(draft, book, cx))
            }
            Mode::Edit(id) => {
                let id = *id;
                let task = self.lists.update(cx, |m, cx| m.update(id, draft, cx));
                cx.spawn(async move |_, _| task.await.map(|()| None))
            }
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(warning) => {
                    window.close_dialog(cx);
                    if let Some(message) = warning {
                        window.push_notification(
                            Notification::warning(message)
                                .title("The list was created without the book"),
                            cx,
                        );
                    }
                }
                Err(message) => {
                    this.error = Some(message);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

const PRIVACY: [Privacy; 3] = Privacy::ALL;

impl Render for ListDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let writing = self.lists.read(cx).is_writing();
        let can_write = Services::global(cx).has_scope(WRITE_LISTS);
        let muted = cx.theme().muted_foreground;
        let view = cx.entity().downgrade();
        let privacy = RadioGroup::horizontal("list-privacy")
            .children(PRIVACY.map(|p| Radio::new(SharedString::from(p.label())).label(p.label())))
            .selected_index(PRIVACY.iter().position(|p| Some(*p) == self.privacy))
            .disabled(writing)
            .on_change(move |ix, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.privacy = Some(PRIVACY[*ix]);
                    cx.notify();
                });
            });
        let view = cx.entity().downgrade();
        let ranked = Checkbox::new("list-ranked")
            .label("Ranked")
            .checked(self.ranked)
            .disabled(writing)
            .on_click(move |checked, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.ranked = *checked;
                    cx.notify();
                });
            });
        let (commit, adds): (&str, Option<String>) = match &self.mode {
            Mode::Create(book) => ("Create", book.as_ref().map(|b| b.title.clone())),
            Mode::Edit(_) => ("Save", None),
        };
        v_flex()
            .gap_4()
            .child(
                Form::new()
                    .label_layout(Axis::Horizontal)
                    .label_width(rems(6.).to_pixels(window.rem_size()))
                    .child(
                        Field::new().label("Name").child(
                            Input::new(&self.name)
                                .disabled(writing)
                                .raised_field(writing, cx),
                        ),
                    )
                    .child(
                        Field::new().label("Description").child(
                            Textarea::new(&self.description)
                                .disabled(writing)
                                .raised_field(writing, cx),
                        ),
                    )
                    .child(Field::new().label("Visible to").child(privacy))
                    .child(
                        Field::new().label("Order").child(
                            v_flex().gap_1().child(ranked).child(
                                div()
                                    .text_xs()
                                    .text_color(muted)
                                    .child("A ranked list numbers its books."),
                            ),
                        ),
                    ),
            )
            .when_some(adds, |this, title| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(format!("“{title}” goes on the new list.")),
                )
            })
            .when(!can_write, |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(needs_permission(WRITE_LISTS)),
                )
            })
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("cancel-list")
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("save-list")
                            .primary()
                            .label(commit)
                            .loading(writing)
                            .disabled(writing || !can_write)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}
