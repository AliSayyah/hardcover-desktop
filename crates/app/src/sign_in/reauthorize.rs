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
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex,
    v_flex,
};
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, EventEmitter, FontWeight, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Task, Window, div,
    prelude::FluentBuilder as _,
};
use hardcover_core::auth::{
    CancelHandle, GrantedScopes, OAuthConfig, Session, StaticToken, Tokens, begin_sign_in,
};
use hardcover_core::{Client, Error, UserId};

use super::sign_in_view::SIGN_IN_TIMEOUT;
use crate::app_state::Services;
use crate::format::{error_message, permissions_message};
use crate::library::model::LibraryModel;

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
    /// Whose library is shown: only that account may replace the session.
    library: Entity<LibraryModel>,
    state: State,
    /// "Later" hides the notice until the next launch.
    dismissed: bool,
    _task: Option<Task<()>>,
}

impl EventEmitter<Reauthorized> for Reauthorize {}

impl Reauthorize {
    pub fn new(library: Entity<LibraryModel>) -> Self {
        Self {
            library,
            state: State::Idle { error: None },
            dismissed: false,
            _task: None,
        }
    }

    /// True while the notice should show: a session that lacks scopes, and
    /// the user hasn't put it off.
    pub fn is_needed(&self, services: &Services) -> bool {
        !self.dismissed && services.session().is_some() && !services.missing_scopes().is_empty()
    }

    /// Signs in in the browser and replaces the session's tokens.
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.state, State::Idle { .. }) {
            return;
        }
        let user = self.library.read(cx).user().map(|u| u.id);
        let services = Services::global(cx);
        let Some(session) = services.session().cloned() else {
            return;
        };
        let client = services.client().clone();
        self.state = State::Starting;
        cx.notify();
        self._task = Some(cx.spawn_in(window, async move |this, cx| {
            let config = OAuthConfig::default();
            // Whose session this is, before the browser opens: before the
            // first sync the library doesn't know yet, and only the same
            // account may replace it.
            let pending = cx
                .background_spawn({
                    let config = config.clone();
                    let session = session.clone();
                    async move {
                        // Signed out meanwhile, or the session expired: there
                        // is nothing to replace.
                        if !session.is_signed_in() {
                            return Err(Error::Unauthorized);
                        }
                        let user = match user {
                            Some(user) => user,
                            None => client.me()?.id,
                        };
                        Ok((user, begin_sign_in(&config)?))
                    }
                })
                .await;
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
                    cancel,
                    url: url.clone().into(),
                };
                cx.notify();
            });
            if opened.is_err() {
                // The notice is gone (signed out): stop listening.
                pending.cancel_handle().cancel();
                return;
            }
            let result = cx
                .background_spawn(async move { replace_tokens(&session, pending, user) })
                .await;
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
            Failure::Core(e) => error_message(&e),
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
                self._task = None;
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

    /// Brings the notice back after "Later".
    pub fn show(&mut self, cx: &mut Context<Self>) {
        self.dismissed = false;
        cx.notify();
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
/// into the session.
fn replace_tokens(
    session: &Session,
    pending: hardcover_core::auth::PendingSignIn,
    user: UserId,
) -> Result<GrantedScopes, Failure> {
    let tokens = pending.wait(SIGN_IN_TIMEOUT)?;
    let result = adopt(session, &tokens, user);
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

/// Swaps `tokens` into `session` if they belong to `user`.
fn adopt(session: &Session, tokens: &Tokens, user: UserId) -> Result<(), Failure> {
    // A throwaway client on the new access token: the session's own client
    // remembers the old account's id.
    let probe = StaticToken::new(tokens.access_token())
        .ok_or_else(|| Error::Auth("the server sent an empty access token".into()))?;
    let me = Client::new(Arc::new(probe)).me()?;
    if me.id != user {
        return Err(Failure::OtherAccount(me.username));
    }
    // Refused once the user signed out meanwhile.
    session.replace(tokens.clone()).map_err(Failure::from)
}

impl Render for Reauthorize {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let danger = theme.danger;
        let message: SharedString =
            permissions_message(&Services::global(cx).missing_scopes()).into();
        let body = match &self.state {
            State::Idle { error } => v_flex()
                .gap_2()
                .child(div().child(message))
                .when_some(error.clone(), |this, error| {
                    this.child(div().text_color(danger).child(error))
                })
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("reauthorize")
                                .outline()
                                .small()
                                .label("Sign in again…")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.start(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("reauthorize-later")
                                .ghost()
                                .small()
                                .label("Later")
                                .tooltip("Hide this until the next start; the account menu brings it back")
                                .on_click(cx.listener(|this, _, _, cx| this.dismiss(cx))),
                        ),
                ),
            State::Starting => v_flex().gap_2().child(div().child(message)).child(
                Button::new("reauthorize")
                    .outline()
                    .small()
                    .loading(true)
                    .disabled(true)
                    .label("Sign in again…"),
            ),
            State::Waiting { .. } => v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_shrink_0().child(Spinner::new().xsmall()))
                        // Wraps inside the narrow sidebar card.
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child("Waiting for approval in your browser…"),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("reauthorize-copy-link")
                                .small()
                                .label("Copy link")
                                .tooltip("If the browser didn't open, paste this link into it")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.copy_link(window, cx)),
                                ),
                        )
                        .child(
                            Button::new("reauthorize-cancel")
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                        ),
                ),
        };
        let theme = cx.theme();
        div().w_full().px_2().child(
            v_flex()
                .gap_2()
                .p_2()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.sidebar_border)
                .text_xs()
                .child(
                    h_flex()
                        .gap_1()
                        .text_color(theme.sidebar_foreground)
                        .font_weight(FontWeight::MEDIUM)
                        .child(Icon::new(IconName::Info).xsmall())
                        .child("New permissions needed"),
                )
                .child(div().text_color(muted).child(body)),
        )
    }
}
