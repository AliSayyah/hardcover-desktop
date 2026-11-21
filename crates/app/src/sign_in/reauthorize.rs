//! The sidebar notice for a session that lacks scopes the app now requests,
//! e.g. one saved by 0.1.x before goals, reviews and lists.
//!
//! `Sign in again…` runs the normal browser sign-in and swaps the new tokens
//! into the existing session ([`Session::replace`]), so the client, the cache
//! and the library on screen stay as they are. Only the same account may
//! replace the session: the cache and the library on screen are theirs.

use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Disableable as _, Icon, IconName, WindowExt as _, h_flex, v_flex};
use gpui_kit::{
    ClipboardItem, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Task, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::{
    CancelHandle, GrantedScopes, KeyringTokenStore, OAuthConfig, PendingSignIn, Session,
    StaticToken, Tokens, begin_sign_in,
};
use hardcover_core::{Client, Error, UserId};

use super::KEYRING_UNAVAILABLE;
use super::sign_in_view::SIGN_IN_TIMEOUT;
use crate::design::{TypeRoles as _, colors};
use crate::format::permissions_message;
use crate::io;
use crate::library::model::LibraryModel;
use crate::session::SessionScope;
use crate::ui::kit::SmallControl as _;

/// Emitted once the session holds the new grant.
pub struct Reauthorized {
    pub scopes: GrantedScopes,
}

enum State {
    Idle {
        error: Option<SharedString>,
    },
    Starting,
    Waiting {
        cancel: CancelHandle,
        url: SharedString,
    },
}

/// Why signing in again did not replace the session.
enum Failure {
    Core(Error),
    /// The browser approved a different account.
    OtherAccount(String),
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Self::Core(e)
    }
}

pub struct Reauthorize {
    session: SessionScope,
    /// Whose library is shown: only that account may replace the session.
    library: Entity<LibraryModel>,
    state: State,
    /// "Later" hides the notice until the next launch.
    dismissed: bool,
    task: Option<Task<()>>,
}

impl EventEmitter<Reauthorized> for Reauthorize {}

impl Reauthorize {
    pub fn new(session: SessionScope, library: Entity<LibraryModel>) -> Self {
        Self {
            session,
            library,
            state: State::Idle { error: None },
            dismissed: false,
            task: None,
        }
    }

    /// True while the notice should show: a session that lacks scopes, and
    /// the user hasn't put it off.
    pub fn is_needed(&self) -> bool {
        !self.dismissed
            && self.session.auth().is_some()
            && !self.session.missing_scopes().is_empty()
    }

    /// Signs in in the browser and replaces the session's tokens.
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.state, State::Idle { .. }) {
            return;
        }
        // Only the signed-in account's own id: a cached one may be another's.
        let user = self.library.read(cx).verified_user_id();
        if !self.session.is_alive() {
            return;
        }
        let Some(session) = self.session.auth().cloned() else {
            return;
        };
        let scope = self.session.clone();
        self.state = State::Starting;
        cx.notify();
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let config = OAuthConfig::default();
            // Whose session this is, before the browser opens: before the
            // first sync the library doesn't know yet, and only the same
            // account may replace it.
            let pending = scope
                .interactive({
                    let config = config.clone();
                    let session = session.clone();
                    move |client| {
                        // Signed out meanwhile, or the session expired: there
                        // is nothing to replace.
                        if !session.is_signed_in() {
                            return Err(Error::Unauthorized);
                        }
                        // The new tokens go to the keyring: say so now if
                        // it can't take them, not after the browser.
                        KeyringTokenStore::check_available()?;
                        let user = match user {
                            Some(user) => user,
                            None => client.me()?.id,
                        };
                        Ok((user, begin_sign_in(&config)?))
                    }
                })
                .await;
            // The session ended meanwhile: there is nothing to replace.
            let Some(pending) = pending else {
                return;
            };
            let (user, pending) = match pending {
                Ok(p) => p,
                Err(e) => {
                    let _ = this.update(cx, |this, cx| this.fail(Failure::Core(e), cx));
                    return;
                }
            };
            let url = pending.authorize_url().to_owned();
            let cancel = pending.cancel_handle();
            let opened = this.update(cx, |this, cx| {
                cx.open_url(&url);
                this.state = State::Waiting {
                    cancel: cancel.clone(),
                    url: url.clone().into(),
                };
                cx.notify();
            });
            if opened.is_err() {
                // The notice is gone (signed out): stop listening.
                cancel.cancel();
                return;
            }
            // The browser may take minutes: a thread of its own, not one of
            // the I/O threads.
            let Some(result) = io::run_on_own_thread("sign-in-again", move || {
                replace_tokens(&session, pending, user, &cancel)
            })
            .await
            else {
                return;
            };
            let _ = this.update(cx, |this, cx| match result {
                Ok(scopes) => {
                    this.state = State::Idle { error: None };
                    cx.emit(Reauthorized { scopes });
                    cx.notify();
                }
                Err(Failure::Core(Error::Cancelled)) => {
                    this.state = State::Idle { error: None };
                    cx.notify();
                }
                Err(failure) => this.fail(failure, cx),
            });
        }));
    }

    fn fail(&mut self, failure: Failure, cx: &mut Context<Self>) {
        let message = match failure {
            Failure::Core(Error::Timeout) => "Sign-in timed out. Try again.".to_owned(),
            // The session's tokens live in the keyring.
            Failure::Core(Error::Storage(detail)) => {
                eprintln!("hardcover: signing in again: {detail}");
                KEYRING_UNAVAILABLE.to_owned()
            }
            Failure::Core(e) => self.session.error_message(&e, None, cx),
            Failure::OtherAccount(username) => format!(
                "You approved as @{username}, not the account shown here. To switch accounts, sign out first."
            ),
        };
        self.state = State::Idle {
            error: Some(message.into()),
        };
        cx.notify();
    }

    /// Stops a sign-in in progress. Also called on sign-out, before the
    /// session forgets its tokens.
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        match &self.state {
            // The waiting task sees the flag and returns `Cancelled`.
            State::Waiting { cancel, .. } => cancel.cancel(),
            // Not listening yet: dropping the task drops the listener once
            // it is set up.
            State::Starting => {
                self.task = None;
                self.state = State::Idle { error: None };
            }
            State::Idle { .. } => {}
        }
        cx.notify();
    }

    fn copy_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let State::Waiting { url, .. } = &self.state {
            cx.write_to_clipboard(ClipboardItem::new_string(url.to_string()));
            window.push_notification(Notification::new().message("Link copied"), cx);
        }
    }

    /// "Sign in again…" from the account menu, after "Later": brings the
    /// notice back and starts the sign-in.
    pub fn sign_in_again(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismissed = false;
        cx.notify();
        self.start(window, cx);
    }

    /// True after "Later": the account menu then offers the notice again.
    pub fn is_dismissed(&self) -> bool {
        self.dismissed
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.dismissed = true;
        cx.notify();
    }
}

impl Drop for Reauthorize {
    fn drop(&mut self) {
        // The browser wait blocks a background thread and would outlive the
        // notice: stop it, so an approval that comes later changes nothing.
        if let State::Waiting { cancel, .. } = &self.state {
            cancel.cancel();
        }
    }
}

/// Blocking: waits for the browser, checks the account, then swaps the tokens
/// into the session, unless the sign-in was cancelled meanwhile.
fn replace_tokens(
    session: &Session,
    pending: PendingSignIn,
    user: UserId,
    cancel: &CancelHandle,
) -> Result<GrantedScopes, Failure> {
    // A cancel during the code exchange revokes the new tokens there.
    let tokens = pending.wait(SIGN_IN_TIMEOUT)?;
    let result = adopt(session, &tokens, user, cancel);
    if result.is_err() {
        // `Session::replace` saves before it swaps, so after any failure the
        // session still holds its old tokens and nobody will use these ones:
        // revoke them, as signing out does. Best effort; the failure to
        // report is the one above.
        if let Err(e) = session.discard(&tokens) {
            eprintln!("hardcover: revoking an unused sign-in: {e}");
        }
    }
    result?;
    Ok(session.granted_scopes())
}

/// Swaps `tokens` into `session` if they belong to `user` and the sign-in
/// wasn't cancelled.
fn adopt(
    session: &Session,
    tokens: &Tokens,
    user: UserId,
    cancel: &CancelHandle,
) -> Result<(), Failure> {
    // A throwaway client on the new access token: the session's own client
    // remembers the old account's id.
    let probe = StaticToken::new(tokens.access_token())
        .ok_or_else(|| Error::Auth("the server sent an empty access token".into()))?;
    let me = Client::new(Arc::new(probe)).me()?;
    if me.id != user {
        return Err(Failure::OtherAccount(me.username));
    }
    if cancel.is_cancelled() {
        return Err(Failure::Core(Error::Cancelled));
    }
    // Refused once the user signed out meanwhile.
    session.replace(tokens.clone()).map_err(Failure::from)
}

impl Render for Reauthorize {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = colors(cx);
        let message: SharedString = permissions_message(&self.session.missing_scopes()).into();
        let sign_in = |loading: bool| {
            Button::new("reauthorize")
                .small_control()
                .label("Sign in again…")
                .loading(loading)
                .disabled(loading)
        };
        let (text, buttons) = match &self.state {
            State::Idle { error } => (
                v_flex()
                    .gap_1()
                    .child(message)
                    .when_some(error.clone(), |this, error| {
                        this.child(div().text_color(colors.danger).child(error))
                    }),
                h_flex()
                    .gap_2()
                    .child(
                        sign_in(false)
                            .on_click(cx.listener(|this, _, window, cx| this.start(window, cx))),
                    )
                    .child(
                        Button::new("reauthorize-later")
                            .ghost()
                            .small_control()
                            .label("Later")
                            .tooltip(
                                "Hide this until the next start; the account menu brings it back",
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
                    ),
            ),
            State::Starting => (v_flex().child(message), h_flex().child(sign_in(true))),
            State::Waiting { .. } => (
                v_flex().child("Waiting for approval in your browser…"),
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("reauthorize-copy-link")
                            .small_control()
                            .label("Copy link")
                            .tooltip("If the browser didn't open, paste this link into it")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.copy_link(window, cx)),
                            ),
                    )
                    .child(
                        Button::new("reauthorize-cancel")
                            .ghost()
                            .small_control()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    ),
            ),
        };
        // Part of the sidebar, not a card on it: the heading on the icon and
        // label spines, the words and buttons under the label.
        v_flex()
            .w_full()
            .gap_1()
            .px_2()
            .pb_2()
            .child(
                h_flex()
                    .gap_2()
                    .body_strong()
                    .child(
                        Icon::new(IconName::Info)
                            .size_4()
                            .flex_none()
                            .text_color(colors.fg_muted),
                    )
                    .child("New permissions needed"),
            )
            .child(
                v_flex()
                    .pl_6()
                    .gap_2()
                    .child(text.caption(cx))
                    .child(buttons),
            )
    }
}
