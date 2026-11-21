//! The sign-in screen: OAuth in the browser with a loopback redirect.
//!
//! `Sign in with Hardcover` first checks that the system keyring can store
//! the sign-in (and says so if it can't, before the browser opens), then
//! binds the loopback listener, opens the authorize URL and waits on a
//! background thread. The URL stays visible (with `Copy link`) in case the
//! browser did not open. `Cancel` stops the wait; a sign-in that finishes
//! after Cancel is revoked, never used.
//!
//! When the keyring couldn't be read at startup, the screen says so with
//! `Try again` instead: a saved sign-in may still be there.

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
    AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FocusHandle,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Task, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::{
    self, CancelHandle, GrantedScopes, KeyringTokenStore, OAuthConfig, PendingSignIn, Session,
    begin_sign_in,
};
use hardcover_core::{Client, Error};

use super::{KEYRING_UNAVAILABLE, SAVED_SIGN_IN_UNREADABLE};
use crate::app_state::{Restored, restore_session};
use crate::design::{TypeRoles as _, colors, size};
use crate::format::error_message;
use crate::io::{self, Priority};
use crate::ui::kit::RaisedField as _;

/// How long to wait for the browser before giving up.
pub(crate) const SIGN_IN_TIMEOUT: Duration = Duration::from_mins(10);

/// Emitted once sign-in completes.
pub struct SignedIn {
    pub client: Client,
    pub session: Arc<Session>,
    pub scopes: GrantedScopes,
}

enum State {
    Idle {
        error: Option<SharedString>,
    },
    Starting,
    Waiting {
        cancel: CancelHandle,
        /// Cancel was pressed; the wait ends at its next check, or once a
        /// code exchange under way has been answered (and revoked).
        cancelling: bool,
    },
    /// The keyring couldn't be read at startup.
    KeyringUnavailable {
        retrying: bool,
    },
}

pub struct SignInView {
    state: State,
    url: Entity<InputState>,
    task: Option<Task<()>>,
    /// The screen's root: focused when it is shown, so Tab reaches its buttons.
    focus_handle: FocusHandle,
}

impl EventEmitter<SignedIn> for SignInView {}

impl SignInView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            state: State::Idle { error: None },
            url: cx.new(|cx| InputState::new(window, cx)),
            task: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Takes the keyboard focus, as the screen is shown.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
    }

    /// The screen for a keyring that couldn't be read at startup, with
    /// `Try again`.
    pub fn keyring_unavailable(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            state: State::KeyringUnavailable { retrying: false },
            ..Self::new(window, cx)
        }
    }

    /// Reads the keyring again: a saved session signs in, none shows the
    /// sign-in, and another failure says so again.
    fn retry_keyring(&mut self, cx: &mut Context<Self>) {
        let State::KeyringUnavailable { retrying, .. } = &mut self.state else {
            return;
        };
        if *retrying {
            return;
        }
        *retrying = true;
        cx.notify();
        let restored = io::run(Priority::Interactive, restore_session);
        self.task = Some(cx.spawn(async move |this, cx| {
            let Some(restored) = restored.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                match restored {
                    Ok(Some(Restored {
                        client,
                        session,
                        scopes,
                    })) => {
                        this.state = State::Idle { error: None };
                        cx.emit(SignedIn {
                            client,
                            session,
                            scopes,
                        });
                    }
                    Ok(None) => this.state = State::Idle { error: None },
                    Err(e) => {
                        eprintln!("hardcover: could not restore the saved session: {e}");
                        this.state = State::KeyringUnavailable { retrying: false };
                    }
                }
                cx.notify();
            });
        }));
    }

    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.state, State::Idle { .. }) {
            return;
        }
        self.state = State::Starting;
        cx.notify();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let config = OAuthConfig::default();
            // The keyring first: without one the sign-in can't be kept, and
            // the user should hear so before approving in the browser.
            let pending = io::run(Priority::Interactive, {
                let config = config.clone();
                move || {
                    KeyringTokenStore::check_available()?;
                    let store = KeyringTokenStore::for_client(config.client_id())?;
                    Ok::<_, Error>((store, begin_sign_in(&config)?))
                }
            })
            .await;
            let (store, pending) = match pending {
                Some(Ok(p)) => p,
                None => return,
                Some(Err(e)) => {
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
                this.state = State::Waiting {
                    cancel: cancel.clone(),
                    cancelling: false,
                };
                cx.notify();
            });
            if opened.is_err() {
                cancel.cancel();
                return;
            }
            // The browser may take minutes: a thread of its own, not one of
            // the I/O threads.
            let Some(result) = io::run_on_own_thread("sign-in", {
                let cancel = cancel.clone();
                move || finish_sign_in(pending, config, store, &cancel)
            })
            .await
            else {
                return;
            };
            let _ = this.update(cx, |this, cx| match result {
                // Cancelled just as the sign-in landed: forget it.
                Ok((session, _)) if cancel.is_cancelled() => {
                    io::spawn(Priority::Interactive, move || {
                        if let Err(e) = session.sign_out() {
                            eprintln!("hardcover: dropping a cancelled sign-in: {e}");
                        }
                    });
                    this.state = State::Idle { error: None };
                    cx.notify();
                }
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
            // The sign-in only ever stores to the keyring.
            Error::Storage(detail) => {
                eprintln!("hardcover: sign-in: {detail}");
                KEYRING_UNAVAILABLE.to_owned()
            }
            e => error_message(e),
        };
        self.state = State::Idle {
            error: Some(message.into()),
        };
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if let State::Waiting { cancel, cancelling } = &mut self.state {
            // The waiting task sees the flag and returns `Cancelled`.
            cancel.cancel();
            *cancelling = true;
        }
        cx.notify();
    }

    fn copy_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.url.read(cx).value().to_string();
        cx.write_to_clipboard(ClipboardItem::new_string(url));
        window.push_notification(Notification::new().message("Link copied"), cx);
    }
}

/// Blocking: waits for the browser, then saves the new session. Checks the
/// cancel flag after each step, so a sign-in cancelled meanwhile is revoked
/// (and cleared from the keyring) rather than used.
fn finish_sign_in(
    pending: PendingSignIn,
    config: OAuthConfig,
    store: KeyringTokenStore,
    cancel: &CancelHandle,
) -> Result<(Arc<Session>, GrantedScopes), Error> {
    let tokens = pending.wait(SIGN_IN_TIMEOUT)?;
    if cancel.is_cancelled() {
        if let Err(e) = auth::revoke(&config, &tokens) {
            eprintln!("hardcover: revoking a cancelled sign-in: {e}");
        }
        return Err(Error::Cancelled);
    }
    // Revokes the new tokens itself if they can't be saved.
    let session = Arc::new(Session::start(config, Arc::new(store), tokens)?);
    if cancel.is_cancelled() {
        if let Err(e) = session.sign_out() {
            eprintln!("hardcover: dropping a cancelled sign-in: {e}");
        }
        return Err(Error::Cancelled);
    }
    let scopes = session.granted_scopes();
    Ok((session, scopes))
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
            State::KeyringUnavailable { retrying } => v_flex()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .text_center()
                        .text_color(colors.danger)
                        .child(SAVED_SIGN_IN_UNREADABLE),
                )
                .child(
                    Button::new("retry-keyring")
                        .label("Try again")
                        .loading(*retrying)
                        .disabled(*retrying)
                        .on_click(cx.listener(|this, _, _, cx| this.retry_keyring(cx))),
                ),
            State::Waiting { cancelling, .. } => v_flex()
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
                        .child("If the browser didn't open, paste this link into it."),
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
                            .loading(*cancelling)
                            .disabled(*cancelling)
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    ),
                ),
        };

        div()
            .track_focus(&self.focus_handle)
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
                            // Not above "Try again": it isn't asking for a sign-in.
                            .when(
                                !matches!(self.state, State::KeyringUnavailable { .. }),
                                |this| {
                                    this.child(div().text_center().text_color(colors.fg_muted).child(
                                        "Sign in with your Hardcover account in the browser to keep your reading library up to date.",
                                    ))
                                },
                            ),
                    )
                    .child(body),
            )
    }
}
