//! One sign-in's scope: the client, the OAuth session, the library cache,
//! the granted scopes, and whether the session is still alive.
//!
//! The workspace builds a [`SessionScope`] when the user signs in and hands
//! it to the models it creates; it is also installed as a GPUI global for
//! views ([`SessionScope::current`], never a panicking lookup). Every call
//! the app makes to `hardcover-core` for the account goes through
//! [`SessionScope::run`], on the I/O threads (`crate::io`).
//!
//! Signing out [ends](SessionScope::end) the scope before anything else: a
//! call not started yet never runs, and a result that lands afterwards
//! resolves to `None`, so it is dropped, never applied. Callers only have to
//! return on `None`.

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::{App, BorrowAppContext as _, Context, Global, Subscription};
use hardcover_core::auth::{DEFAULT_SCOPES, GrantedScopes, Session};
use hardcover_core::store::LibraryCache;
use hardcover_core::{Client, Error};

use crate::format;
use crate::io::{self, IoPool, Priority};

/// One sign-in's services and lifetime. Cheap to clone; clones share the
/// grant and the alive flag.
#[derive(Clone)]
pub struct SessionScope {
    inner: Rc<Inner>,
}

struct Inner {
    client: Client,
    /// `None` when signed in with the development token or in the demo.
    auth: Option<Arc<Session>>,
    /// `None` if the cache could not be opened; the app then works online
    /// only.
    cache: Option<Arc<LibraryCache>>,
    /// False once signed out. Read on the I/O threads.
    alive: Arc<AtomicBool>,
    io: Io,
    grant: RefCell<Grant>,
}

/// What the session may do.
struct Grant {
    /// A snapshot of the OAuth session's grant: asking the session itself
    /// can block behind a token refresh.
    scopes: GrantedScopes,
    /// Scopes Hardcover refused with `insufficient_scope` although the grant
    /// lists them. The server is the authority, so these count as missing.
    refused: Vec<&'static str>,
}

/// Where the session's blocking work runs.
#[derive(Clone, Copy)]
pub(crate) enum Io {
    /// The I/O threads, and a thread of the library cache's own.
    Threads,
    /// On the calling thread, at once: GPUI's test executor doesn't wait for
    /// threads of ours, so tests stay deterministic.
    #[cfg(test)]
    Inline,
}

struct CurrentSession(SessionScope);

impl Global for CurrentSession {}

impl SessionScope {
    pub fn new(
        client: Client,
        auth: Option<Arc<Session>>,
        cache: Option<Arc<LibraryCache>>,
        scopes: GrantedScopes,
    ) -> Self {
        Self::with_io(client, auth, cache, scopes, Io::Threads)
    }

    /// A session whose calls run inline (see [`Io::Inline`]).
    #[cfg(test)]
    pub fn for_tests(client: Client, cache: Option<Arc<LibraryCache>>) -> Self {
        Self::with_io(client, None, cache, GrantedScopes::all(), Io::Inline)
    }

    fn with_io(
        client: Client,
        auth: Option<Arc<Session>>,
        cache: Option<Arc<LibraryCache>>,
        scopes: GrantedScopes,
        io: Io,
    ) -> Self {
        Self {
            inner: Rc::new(Inner {
                client,
                auth,
                cache,
                alive: Arc::new(AtomicBool::new(true)),
                io,
                grant: RefCell::new(Grant {
                    scopes,
                    refused: Vec::new(),
                }),
            }),
        }
    }

    // ----- the global -----

    /// Makes this the session views find with [`Self::current`].
    pub fn install(&self, cx: &mut App) {
        cx.set_global(CurrentSession(self.clone()));
    }

    /// Forgets the installed session, once signed out.
    pub fn uninstall(cx: &mut App) {
        if cx.has_global::<CurrentSession>() {
            cx.remove_global::<CurrentSession>();
        }
    }

    /// The signed-in session, or `None` once signed out.
    pub fn current(cx: &App) -> Option<&SessionScope> {
        cx.try_global::<CurrentSession>().map(|c| &c.0)
    }

    // ----- lifetime -----

    /// False once signed out: nothing more is sent for this session, and
    /// results still to land are dropped.
    pub fn is_alive(&self) -> bool {
        self.inner.alive.load(Ordering::Acquire)
    }

    /// Ends the session, first thing on signing out.
    pub fn end(&self) {
        self.inner.alive.store(false, Ordering::Release);
    }

    // ----- services -----

    /// The OAuth session; `None` with a personal token or in the demo.
    pub fn auth(&self) -> Option<&Arc<Session>> {
        self.inner.auth.as_ref()
    }

    pub fn cache(&self) -> Option<&Arc<LibraryCache>> {
        self.inner.cache.as_ref()
    }

    pub(crate) fn io(&self) -> Io {
        self.inner.io
    }

    /// Runs `call` with the session's client on the I/O threads, its reads
    /// waiting for the rate limit at `priority` too (`Client::with_priority`;
    /// writes and searches are always interactive there).
    ///
    /// Resolves to `None`, and `call` doesn't run, if the session ended
    /// first; a result that lands after the session ended is `None` too.
    /// Dropping the future before the call starts skips it.
    pub fn run<R, F>(
        &self,
        priority: Priority,
        call: F,
    ) -> impl Future<Output = Option<R>> + use<R, F>
    where
        R: Send + 'static,
        F: FnOnce(&Client) -> R + Send + 'static,
    {
        let alive = self.inner.alive.clone();
        let client = self.inner.client.with_priority(priority);
        let result = self.is_alive().then(|| match self.inner.io {
            Io::Threads => {
                let alive = alive.clone();
                io::submit(IoPool::global(), priority, move || {
                    alive.load(Ordering::Acquire).then(|| call(&client))
                })
            }
            #[cfg(test)]
            Io::Inline => {
                let (tx, rx) = async_channel::bounded(1);
                let _ = tx.try_send(Some(call(&client)));
                rx
            }
        });
        async move {
            let result = result?.recv().await.ok()??;
            alive.load(Ordering::Acquire).then_some(result)
        }
    }

    /// [`Self::run`] for what the user is waiting on: a write, a search, a
    /// page they opened.
    pub fn interactive<R, F>(&self, call: F) -> impl Future<Output = Option<R>> + use<R, F>
    where
        R: Send + 'static,
        F: FnOnce(&Client) -> R + Send + 'static,
    {
        self.run(Priority::Interactive, call)
    }

    /// [`Self::run`] for everything else: syncs, reloads, prefetches.
    pub fn background<R, F>(&self, call: F) -> impl Future<Output = Option<R>> + use<R, F>
    where
        R: Send + 'static,
        F: FnOnce(&Client) -> R + Send + 'static,
    {
        self.run(Priority::Background, call)
    }

    // ----- scopes -----

    /// True if Hardcover granted `scope` (one of `hardcover_core::auth::scopes`).
    pub fn has_scope(&self, scope: &str) -> bool {
        let grant = self.inner.grant.borrow();
        grant.scopes.contains(scope) && !grant.refused.contains(&scope)
    }

    /// True if a request needing `scope` may go out: the session is alive
    /// and has it. Models check this before starting work.
    pub fn allows(&self, scope: &str) -> bool {
        self.is_alive() && self.has_scope(scope)
    }

    /// The scopes the app requests that this session lacks.
    pub fn missing_scopes(&self) -> Vec<&'static str> {
        DEFAULT_SCOPES
            .into_iter()
            .filter(|s| !self.has_scope(s))
            .collect()
    }

    /// Records that Hardcover refused a request for lack of `scope`, so the
    /// features that need it turn into their "sign in again" state.
    pub fn scope_refused(&self, scope: &'static str, cx: &mut App) {
        let mut grant = self.inner.grant.borrow_mut();
        if !grant.refused.contains(&scope) {
            grant.refused.push(scope);
            drop(grant);
            cx.refresh_windows();
        }
    }

    /// After a new sign-in replaced the session's tokens. Views observing
    /// the grant ([`observe_grant`]) hear of it.
    pub fn set_scopes(&self, scopes: GrantedScopes, cx: &mut App) {
        *self.inner.grant.borrow_mut() = Grant {
            scopes,
            refused: Vec::new(),
        };
        if cx.has_global::<CurrentSession>() {
            cx.update_global::<CurrentSession, _>(|_, _| {});
        }
        cx.refresh_windows();
    }

    /// The message to show for a failed request. A scope Hardcover refused
    /// is recorded first (the one the error names, else `fallback`), so the
    /// features that need it turn off and the permissions notice offers to
    /// sign in again. Where no notice can (a personal token, or a scope the
    /// app doesn't request), the message says to sign out and in again.
    pub fn error_message(
        &self,
        error: &Error,
        fallback: Option<&'static str>,
        cx: &mut App,
    ) -> String {
        if let Error::InsufficientScope { scope } = error
            // The fallback stands in only for an unnamed scope: a named one
            // the app never requests can't be granted by signing in again.
            && let Some(scope) = match scope {
                Some(_) => format::requested_scope(scope.as_deref()),
                None => fallback,
            }
        {
            self.scope_refused(scope, cx);
            if self.auth().is_some() {
                return format::scope_refused_message(scope);
            }
        }
        format::error_message(error)
    }
}

/// Calls `f` when a session is signed in, and when its grant changes
/// ("Sign in again…").
pub fn observe_grant<V: 'static>(
    cx: &mut Context<V>,
    f: impl Fn(&mut V, &mut Context<V>) + 'static,
) -> Subscription {
    cx.observe_global::<CurrentSession>(move |this, cx| {
        if cx.has_global::<CurrentSession>() {
            f(this, cx);
        }
    })
}

/// True if the signed-in session has `scope`; false once signed out. Cheap:
/// views call it while rendering, to hide or disable what the session can't
/// do.
pub fn has_scope(cx: &App, scope: &str) -> bool {
    SessionScope::current(cx).is_some_and(|s| s.has_scope(scope))
}

/// [`SessionScope::error_message`] on the signed-in session; once signed
/// out, the plain message.
pub fn error_message(error: &Error, fallback: Option<&'static str>, cx: &mut App) -> String {
    match SessionScope::current(cx).cloned() {
        Some(session) => session.error_message(error, fallback, cx),
        None => format::error_message(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardcover_core::auth::scopes::{READ_GOALS, READ_LISTS};

    fn session() -> SessionScope {
        SessionScope::for_tests(hardcover_core::demo::client(), None)
    }

    #[test]
    fn a_call_after_the_session_ended_never_runs() {
        let session = session();
        session.end();
        let ran = Arc::new(AtomicBool::new(false));
        let call = session.interactive({
            let ran = ran.clone();
            move |_| ran.store(true, Ordering::Release)
        });
        assert_eq!(gpui_kit::block_on(call), None);
        assert!(!ran.load(Ordering::Acquire));
    }

    #[test]
    fn a_result_that_lands_after_the_session_ended_is_dropped() {
        let session = session();
        // Inline: the call has run and its result waits to be picked up.
        let call = session.interactive(|_| 42);
        session.end();
        assert_eq!(gpui_kit::block_on(call), None);
    }

    #[test]
    fn a_scope_change_is_seen_through_every_clone() {
        let session = session();
        let model_copy = session.clone();
        gpui_kit::TestAppContext::single().update(|cx| {
            session.scope_refused(READ_GOALS, cx);
            assert!(!model_copy.has_scope(READ_GOALS));
            assert!(model_copy.has_scope(READ_LISTS));
            session.set_scopes(GrantedScopes::all(), cx);
            assert!(model_copy.has_scope(READ_GOALS));
        });
    }
}
