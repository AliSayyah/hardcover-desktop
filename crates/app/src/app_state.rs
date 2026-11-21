//! How the app starts (the saved session, the development token or the
//! sign-in screen), and the cover service (`cover_pipeline`), a GPUI global
//! for the whole run. What lives only while signed in is the
//! `crate::session::SessionScope`.
//!
//! Every core call blocks, so it runs on the I/O threads (`crate::io`) and
//! its result is applied on the foreground.

use std::sync::Arc;

use hardcover_core::auth::{GrantedScopes, KeyringTokenStore, OAuthConfig, Session, StaticToken};
use hardcover_core::store::LibraryCache;
use hardcover_core::{Client, Error};

/// The cover service, a global, and what features use of it.
pub use crate::cover_pipeline::{CoverService, release_cover_image, release_cover_medium};

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
