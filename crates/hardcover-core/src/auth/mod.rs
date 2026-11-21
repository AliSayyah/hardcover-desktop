//! Signing in to Hardcover.
//!
//! The desktop app is an OAuth *public client*: authorization code + PKCE (S256)
//! with a loopback redirect (`http://127.0.0.1:<random port>/callback`; Hardcover
//! ignores the port for loopback URIs). Flow:
//!
//! 1. [`begin_sign_in`] binds the loopback listener and builds the authorize URL.
//! 2. The UI opens [`PendingSignIn::authorize_url`] in the browser (and shows it).
//! 3. [`PendingSignIn::wait`] blocks until the browser comes back, checks `state`
//!    and `iss`, and exchanges the code for [`Tokens`].
//! 4. [`Session::start`] persists them in a [`TokenStore`] and acts as the
//!    [`TokenSource`](crate::api::TokenSource) for the API client, refreshing
//!    (with rotation) as needed. [`Session::sign_out`] revokes and forgets them.
//!
//! [`StaticToken`] is a development fallback that reads a personal access token
//! from `HARDCOVER_TOKEN`.
//!
//! [`GrantedScopes`] says which scopes a session holds. Sessions saved by an
//! older version hold fewer than [`DEFAULT_SCOPES`]; [`Session::replace`] swaps
//! in the tokens of a new sign-in that grants the rest.

mod callback;
mod oauth;
mod pkce;
pub mod scopes;
mod session;
mod store;

pub use callback::{CancelHandle, PendingSignIn, begin_sign_in};
pub use scopes::GrantedScopes;
pub use session::{Session, StaticToken};
pub use store::{KeyringTokenStore, MemoryTokenStore, TokenStore};

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::api::Timestamp;

/// The OAuth client id registered for hardcover-desktop (public, not a secret).
pub const DEFAULT_CLIENT_ID: &str = "18d30dcc-9d91-4022-8464-d1939ed15ad2";

/// Environment variable that overrides the client id at runtime.
pub const CLIENT_ID_ENV: &str = "HARDCOVER_CLIENT_ID";

/// Scopes requested at sign-in: profile, catalog, read/write on the library,
/// reviews, reading goals, lists, the journal, other readers and notifications. Adding one makes every existing session
/// ask the user to sign in again, so add them sparingly.
pub const DEFAULT_SCOPES: [&str; 14] = [
    scopes::READ_ME,
    scopes::READ_CATALOG,
    scopes::READ_LIBRARY,
    scopes::WRITE_LIBRARY,
    scopes::WRITE_REVIEWS,
    scopes::READ_GOALS,
    scopes::WRITE_GOALS,
    scopes::READ_LISTS,
    scopes::WRITE_LISTS,
    scopes::READ_JOURNAL,
    scopes::READ_SOCIAL,
    scopes::READ_USERS,
    scopes::READ_NOTIFICATIONS,
    scopes::WRITE_NOTIFICATIONS,
];

/// `HARDCOVER_CLIENT_ID` if set and non-empty, otherwise `default`.
#[must_use]
pub fn client_id_from_env_or(default: &str) -> String {
    std::env::var(CLIENT_ID_ENV)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

/// Hardcover's OAuth endpoints. [`OAuthEndpoints::default`] is the production
/// server, as published at `https://api.hardcover.app/.well-known/oauth-authorization-server`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OAuthEndpoints {
    /// Expected `iss` on the callback (RFC 9207).
    pub issuer: String,
    pub authorize: String,
    pub token: String,
    pub revoke: String,
}

impl Default for OAuthEndpoints {
    fn default() -> Self {
        Self {
            issuer: "https://api.hardcover.app".into(),
            authorize: "https://hardcover.app/oauth2/authorize".into(),
            token: "https://api.hardcover.app/oauth2/token".into(),
            revoke: "https://api.hardcover.app/oauth2/revoke".into(),
        }
    }
}

impl OAuthEndpoints {
    /// Fetches the discovery document and checks that its issuer is Hardcover's.
    /// Optional: the defaults are correct today.
    ///
    /// # Errors
    ///
    /// Returns an error if the issuer metadata cannot be retrieved or validated.
    pub fn discover() -> crate::Result<Self> {
        oauth::discover(&crate::http::UreqTransport::new(), &Self::default().issuer)
    }
}

/// What to ask for at sign-in.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    client_id: String,
    scopes: Vec<String>,
    redirect_path: String,
    endpoints: OAuthEndpoints,
}

impl OAuthConfig {
    /// Default scopes, `/callback` redirect path, production endpoints.
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            scopes: DEFAULT_SCOPES.iter().map(|s| (*s).to_owned()).collect(),
            redirect_path: "/callback".into(),
            endpoints: OAuthEndpoints::default(),
        }
    }

    #[must_use]
    pub fn with_scopes<I, S>(mut self, scopes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.scopes = scopes.into_iter().map(Into::into).collect();
        self
    }

    /// Must start with `/`, and match the path registered for the app.
    #[must_use]
    pub fn with_redirect_path(mut self, path: impl Into<String>) -> Self {
        self.redirect_path = path.into();
        self
    }

    #[must_use]
    pub fn with_endpoints(mut self, endpoints: OAuthEndpoints) -> Self {
        self.endpoints = endpoints;
        self
    }

    #[must_use]
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    #[must_use]
    pub fn scopes(&self) -> &[String] {
        &self.scopes
    }

    #[must_use]
    pub fn redirect_path(&self) -> &str {
        &self.redirect_path
    }

    #[must_use]
    pub fn endpoints(&self) -> &OAuthEndpoints {
        &self.endpoints
    }
}

impl Default for OAuthConfig {
    /// [`DEFAULT_CLIENT_ID`], overridable with `HARDCOVER_CLIENT_ID`.
    fn default() -> Self {
        Self::new(client_id_from_env_or(DEFAULT_CLIENT_ID))
    }
}

/// An access token and (for OAuth) its refresh token. `Debug` redacts both.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<Timestamp>,
    #[serde(default)]
    scope: Option<String>,
}

impl Tokens {
    pub fn new(
        access_token: impl Into<String>,
        refresh_token: Option<String>,
        expires_at: Option<Timestamp>,
    ) -> Self {
        Self {
            access_token: access_token.into(),
            refresh_token,
            expires_at,
            scope: None,
        }
    }

    #[must_use]
    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    #[must_use]
    pub fn refresh_token(&self) -> Option<&str> {
        self.refresh_token.as_deref()
    }

    #[must_use]
    pub fn expires_at(&self) -> Option<Timestamp> {
        self.expires_at
    }

    /// Space-separated scopes granted. Since this version a sign-in always
    /// records one (the requested scopes when the server's response left it
    /// out); `None` means an older version saved these tokens.
    #[must_use]
    pub fn scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }

    pub(crate) fn with_scope(mut self, scope: Option<String>) -> Self {
        self.scope = scope;
        self
    }

    /// True if the access token expires within `margin` of `now`.
    #[must_use]
    pub fn expires_within(&self, now: Timestamp, margin: chrono::Duration) -> bool {
        self.expires_at.is_some_and(|exp| exp - now <= margin)
    }
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_at", &self.expires_at)
            .field("scope", &self.scope)
            .finish()
    }
}

/// Revokes `tokens` at Hardcover (RFC 7009), e.g. those of a sign-in that
/// was cancelled once the browser had approved it.
///
/// # Errors
///
/// Returns an error if the revocation request fails.
pub fn revoke(cfg: &OAuthConfig, tokens: &Tokens) -> crate::Result<()> {
    oauth::revoke(&crate::http::UreqTransport::new(), cfg, tokens)
}

/// Short random string for temp file names.
pub(crate) fn random_suffix() -> String {
    pkce::random_urlsafe(6).unwrap_or_else(|_| std::process::id().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_debug_redacts_secrets() {
        let t = Tokens::new("hc_at_secret", Some("hc_rt_secret".into()), None);
        let dbg = format!("{t:?}");
        assert!(!dbg.contains("secret"), "{dbg}");
        assert!(dbg.contains("<redacted>"));
    }

    #[test]
    fn default_config() {
        let c = OAuthConfig::new("cid");
        assert_eq!(
            c.scopes(),
            [
                "read:me",
                "read:catalog",
                "read:library",
                "write:library",
                "write:reviews",
                "read:goals",
                "write:goals",
                "read:lists",
                "write:lists",
                "read:journal",
                "read:social",
                "read:users",
                "read:notifications",
                "write:notifications"
            ]
        );
        assert_eq!(c.redirect_path(), "/callback");
        assert_eq!(c.endpoints().issuer, "https://api.hardcover.app");
    }

    #[test]
    fn expiry_margin() {
        let now = chrono::Utc::now();
        let t = Tokens::new("a", None, Some(now + chrono::Duration::minutes(5)));
        assert!(t.expires_within(now, chrono::Duration::minutes(10)));
        assert!(!t.expires_within(now, chrono::Duration::minutes(1)));
        assert!(!Tokens::new("a", None, None).expires_within(now, chrono::Duration::days(365)));
    }
}
