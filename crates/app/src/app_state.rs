//! Application services shared by every feature: the API client, the session
//! and the library cache, and the cover service (`cover_pipeline`).
//!
//! Every core call blocks, so features run them with `cx.background_spawn` and
//! apply results on the foreground. The services are GPUI globals:
//! [`CoverService`] lives for the whole run, [`Services`] only while signed in.

use std::sync::Arc;

use gpui_kit::{App, Global};
use hardcover_core::auth::{
    DEFAULT_SCOPES, GrantedScopes, KeyringTokenStore, OAuthConfig, Session, StaticToken,
};
use hardcover_core::store::LibraryCache;
use hardcover_core::{Client, Error};

/// The cover service, a global like the others, and what features use of it.
pub use crate::cover_pipeline::{CoverService, release_cover_image, release_cover_medium};
use crate::format;

/// Services available while signed in.
#[derive(Clone)]
pub struct Services {
    client: Client,
    /// `None` when signed in with the development token.
    session: Option<Arc<Session>>,
    /// `None` if the cache could not be opened; the app then works online only.
    cache: Option<Arc<LibraryCache>>,
    /// A snapshot of the session's grant: asking the session itself can block
    /// behind a token refresh.
    scopes: GrantedScopes,
    /// Scopes Hardcover refused with `insufficient_scope` although the grant
    /// lists them. The server is the authority, so these count as missing.
    refused: Vec<&'static str>,
}

impl Global for Services {}

impl Services {
    pub fn new(
        client: Client,
        session: Option<Arc<Session>>,
        cache: Option<Arc<LibraryCache>>,
        scopes: GrantedScopes,
    ) -> Self {
        Self {
            client,
            session,
            cache,
            scopes,
            refused: Vec::new(),
        }
    }

    /// True if Hardcover granted `scope` (one of `hardcover_core::auth::scopes`).
    /// Cheap: features call it while rendering, to hide or disable what the
    /// session can't do.
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.contains(scope) && !self.refused.contains(&scope)
    }

    /// The scopes the app requests that this session lacks. Only a session can
    /// sign in again to get them; a personal token never lacks any it knows of.
    pub fn missing_scopes(&self) -> Vec<&'static str> {
        DEFAULT_SCOPES
            .into_iter()
            .filter(|s| !self.has_scope(s))
            .collect()
    }

    /// Records that Hardcover refused a request for lack of `scope`, so the
    /// feature turns into its "sign in again" state.
    pub fn scope_refused(scope: &'static str, cx: &mut App) {
        if let Some(services) = cx.try_global::<Self>()
            && !services.refused.contains(&scope)
        {
            cx.global_mut::<Self>().refused.push(scope);
            cx.refresh_windows();
        }
    }

    /// The message to show for a failed request. A scope Hardcover refused is
    /// recorded first (the one the error names, else `fallback`), so features
    /// that need it turn off and the permissions notice offers to sign in
    /// again. Where no notice can (a personal token, or a scope the app
    /// doesn't request), the message says to sign out and in again.
    pub fn error_message(error: &Error, fallback: Option<&'static str>, cx: &mut App) -> String {
        if let Error::InsufficientScope { scope } = error
            // The fallback stands in only for an unnamed scope: a named one
            // the app never requests can't be granted by signing in again.
            && let Some(scope) = match scope {
                Some(_) => format::requested_scope(scope.as_deref()),
                None => fallback,
            }
        {
            Self::scope_refused(scope, cx);
            if cx.try_global::<Self>().is_some_and(|s| s.session.is_some()) {
                return format::scope_refused_message(scope);
            }
        }
        format::error_message(error)
    }

    /// After a new sign-in replaced the session's tokens.
    pub fn set_scopes(scopes: GrantedScopes, cx: &mut App) {
        if cx.has_global::<Self>() {
            let services = cx.global_mut::<Self>();
            services.scopes = scopes;
            services.refused.clear();
            cx.refresh_windows();
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn session(&self) -> Option<&Arc<Session>> {
        self.session.as_ref()
    }

    pub fn cache(&self) -> Option<&Arc<LibraryCache>> {
        self.cache.as_ref()
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }
}

/// How the app starts, decided off the UI thread.
pub enum Startup {
    SignedIn {
        client: Client,
        session: Option<Arc<Session>>,
        cache: Option<Arc<LibraryCache>>,
        scopes: GrantedScopes,
    },
    SignedOut {
        cache: Option<Arc<LibraryCache>>,
    },
    /// The keyring couldn't be read (no secret service, or a locked one), so
    /// a saved session may be there: the sign-in screen offers to try again
    /// ([`restore_session`]) rather than a new sign-in.
    KeyringUnavailable {
        cache: Option<Arc<LibraryCache>>,
    },
}

/// A session restored from the keyring, with its client and grant.
pub struct Restored {
    pub client: Client,
    pub session: Arc<Session>,
    pub scopes: GrantedScopes,
}

/// Blocking: the session saved in the keyring, if any.
///
/// # Errors
///
/// The keyring's error if it can't be read, which may pass.
pub fn restore_session() -> Result<Option<Restored>, Error> {
    let config = OAuthConfig::default();
    let store = KeyringTokenStore::for_client(config.client_id())?;
    Ok(Session::restore(config, Arc::new(store))?.map(|session| {
        let session = Arc::new(session);
        Restored {
            client: Client::new(session.clone()),
            scopes: session.granted_scopes(),
            session,
        }
    }))
}

/// Blocking: reads the environment, the keyring and opens the caches.
pub fn startup() -> Startup {
    // Decided before anything real is opened, so a demo build never shows or
    // writes the account's library.
    #[cfg(feature = "demo")]
    return demo_startup();
    #[cfg(not(feature = "demo"))]
    real_startup()
}

/// The demo library, with a cache that lives only in memory, and the faults
/// `HARDCOVER_DEMO_FAULTS` asks for.
#[cfg(feature = "demo")]
fn demo_startup() -> Startup {
    let faults = hardcover_core::demo::env_faults();
    if faults.is_empty() {
        eprintln!("hardcover: demo build, showing the demo library");
    } else {
        eprintln!("hardcover: demo build, showing the demo library with faults: {faults}");
    }
    Startup::SignedIn {
        client: hardcover_core::demo::client_with(faults),
        session: None,
        cache: LibraryCache::open_in_memory().ok().map(Arc::new),
        scopes: faults.granted_scopes(),
    }
}

// Unused in demo builds, which never read the real account.
#[cfg_attr(feature = "demo", allow(dead_code))]
fn real_startup() -> Startup {
    let cache = match LibraryCache::open_default() {
        Ok(cache) => Some(Arc::new(cache)),
        Err(e) => {
            eprintln!("hardcover: library cache unavailable: {e}");
            None
        }
    };
    if let Some(token) = StaticToken::from_env() {
        eprintln!("hardcover: using HARDCOVER_TOKEN (development only)");
        // A personal token carries no scope information and acts for its
        // owner with full access.
        return Startup::SignedIn {
            client: Client::new(Arc::new(token)),
            session: None,
            cache,
            scopes: GrantedScopes::all(),
        };
    }
    match restore_session() {
        Ok(Some(Restored {
            client,
            session,
            scopes,
        })) => Startup::SignedIn {
            client,
            session: Some(session),
            cache,
            scopes,
        },
        Ok(None) => Startup::SignedOut { cache },
        Err(error) => {
            eprintln!("hardcover: could not restore the saved session: {error}");
            Startup::KeyringUnavailable { cache }
        }
    }
}
