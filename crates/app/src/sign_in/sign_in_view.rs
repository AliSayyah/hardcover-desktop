//! The sign-in screen: OAuth in the browser with a loopback redirect.
//!
//! `Sign in with Hardcover` binds the loopback listener, opens the authorize
//! URL and waits on a background thread. The URL stays visible (with
//! `Copy link`) in case the browser did not open. `Cancel` stops the wait.

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    Disableable as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, EventEmitter, IntoElement, ParentElement as _,
    Render, SharedString, Styled as _, Task, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::{
    CancelHandle, GrantedScopes, KeyringTokenStore, OAuthConfig, Session, begin_sign_in,
};
use hardcover_core::{Client, Error};

use crate::design::{TypeRoles as _, colors, size};
use crate::format::error_message;
use crate::ui::kit::RaisedField as _;

/// How long to wait for the browser before giving up.
pub(crate) const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Emitted once sign-in completes.
pub struct SignedIn {
    pub client: Client,
    pub session: Arc<Session>,
    pub scopes: GrantedScopes,
}

enum State {
    Idle { error: Option<SharedString> },
    Starting,
    Waiting { cancel: CancelHandle },
}

pub struct SignInView {
    state: State,
    url: Entity<InputState>,
    _task: Option<Task<()>>,
}

impl EventEmitter<SignedIn> for SignInView {}

impl SignInView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            state: State::Idle { error: None },
            url: cx.new(|cx| InputState::new(window, cx)),
            _task: None,
        }
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.state, State::Idle { .. }) {
            return;
        }
        self.state = State::Starting;
        cx.notify();
        self._task = Some(cx.spawn_in(window, async move |this, cx| {
            let config = OAuthConfig::default();
            let pending = cx
                .background_spawn({
                    let config = config.clone();
                    async move { begin_sign_in(&config) }
                })
                .await;
            let pending = match pending {
                Ok(p) => p,
                Err(e) => {
                    let _ = this.update(cx, |this, cx| this.fail(&e, cx));
                    return;
                }
            };
            let url = pending.authorize_url().to_owned();
            let cancel = pending.cancel_handle();
            let opened = this.update_in(cx, |this, window, cx| {
                cx.open_url(&url);
                this.url
                    .update(cx, |input, cx| input.set_value(url.clone(), window, cx));
                this.state = State::Waiting { cancel };
                cx.notify();
            });
            if opened.is_err() {
                pending.cancel_handle().cancel();
                return;
            }
            let result = cx
                .background_spawn(async move {
                    let tokens = pending.wait(SIGN_IN_TIMEOUT)?;
                    let store = KeyringTokenStore::new()?;
                    let session = Arc::new(Session::start(config, Arc::new(store), tokens)?);
                    let scopes = session.granted_scopes();
                    Ok::<_, Error>((session, scopes))
                })
                .await;
            let _ = this.update(cx, |this, cx| match result {
                Ok((session, scopes)) => {
                    this.state = State::Idle { error: None };
                    cx.emit(SignedIn {
                        client: Client::new(session.clone()),
                        session,
                        scopes,
                    });
                }
                Err(Error::Cancelled) => {
                    this.state = State::Idle { error: None };
                    cx.notify();
                }
                Err(e) => this.fail(&e, cx),
            });
        }));
    }

    fn fail(&mut self, error: &Error, cx: &mut Context<Self>) {
        let message = match error {
            Error::Timeout => "Sign-in timed out. Try again.".to_owned(),
            e => error_message(e),
        };
        self.state = State::Idle {
            error: Some(message.into()),
        };
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if let State::Waiting { cancel } = &self.state {
            // The waiting task sees the flag and returns `Cancelled`.
            cancel.cancel();
        }
        cx.notify();
    }

    fn copy_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.url.read(cx).value().to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(url));
        window.push_notification(Notification::new().message("Link copied"), cx);
    }
}

impl Render for SignInView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = colors(cx);
        // The one command on the screen, so it isn't primary (primary marks a
        // dialog's commit). While the browser opens it keeps its label and
        // shows that it is busy.
        let sign_in = |loading: bool| {
            Button::new("sign-in")
                .label("Sign in with Hardcover")
                .loading(loading)
                .disabled(loading)
        };
        let body = match &self.state {
            State::Idle { error } => v_flex()
                .gap_3()
                .items_center()
                .child(
                    sign_in(false)
                        .on_click(cx.listener(|this, _, window, cx| this.start(window, cx))),
                )
                .when_some(error.clone(), |this, error| {
                    this.child(div().text_center().text_color(colors.danger).child(error))
                }),
            State::Starting => v_flex().items_center().child(sign_in(true)),
            State::Waiting { .. } => v_flex()
                .gap_3()
                .w_full()
                .child(
                    h_flex()
                        .gap_2()
                        .justify_center()
                        .child(Spinner::new().small().color(colors.fg_muted))
                        .child("Waiting for approval in your browser…"),
                )
                .child(
                    div()
                        .caption(cx)
                        .text_center()
                        .child("If the browser didn't open, copy this link into it."),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Input::new(&self.url)
                                .raised_field(false, cx)
                                .readonly(true)
                                .flex_1()
                                .min_w_0(),
                        )
                        .child(Button::new("copy-link").label("Copy link").on_click(
                            cx.listener(|this, _, window, cx| this.copy_link(window, cx)),
                        )),
                )
                .child(
                    h_flex().justify_center().child(
                        Button::new("cancel-sign-in")
                            .ghost()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    ),
                ),
        };

        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(colors.bg)
            .body()
            .child(
                v_flex()
                    .w(size::DIALOG_FORM)
                    .gap_6()
                    .p_8()
                    .items_center()
                    .child(
                        v_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .page_title()
                                    .child(
                                        Icon::new(IconName::BookOpen)
                                            .size_6()
                                            .text_color(colors.accent),
                                    )
                                    .child("Hardcover"),
                            )
                            .child(div().text_center().text_color(colors.fg_muted).child(
                                "Sign in with your Hardcover account in the browser to keep your reading library up to date.",
                            )),
                    )
                    .child(body),
            )
    }
}
