//! Token sources for the API client.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::oauth;
use super::{GrantedScopes, OAuthConfig, TokenStore, Tokens};
use crate::api::TokenSource;
use crate::http::{HttpTransport, UreqTransport};
use crate::lock::FileLock;
use crate::{Error, Result};

/// Refresh this long before the access token expires (they last a week).
const REFRESH_MARGIN_MINUTES: i64 = 10;

/// A signed-in OAuth session.
///
/// Refresh tokens rotate on every use, and replaying a spent one makes
/// Hardcover reject it (and may revoke the whole grant). Every process that
/// uses the saved tokens (a second copy of the app, the examples,
/// `api_check`) therefore refreshes the same way:
/// - one refresh at a time: in this process under the session's lock, and
///   across processes under the store's ([`TokenStore::lock`]);
/// - under that lock the store is read again first, and a token another
///   process rotated is adopted instead of refreshing with a spent one;
/// - a refresh is never retried automatically;
/// - only a dead grant (`invalid_grant`, `invalid_client`,
///   `unauthorized_client`) signs the session out, and the store is cleared
///   only if it still holds the token that was rejected;
/// - the rotated pair is used at once; if saving it fails, the save is tried
///   again on a later call (at most every [`SAVE_RETRY_INTERVAL`]) until it
///   succeeds, and a refresh in between never adopts the spent token still
///   in the store.
pub struct Session {
    cfg: OAuthConfig,
    store: Arc<dyn TokenStore>,
    http: Arc<dyn HttpTransport>,
    state: Mutex<State>,
    save_retry_interval: Duration,
}

/// How often a failed save is tried again. Each try reads and writes the
/// keyring, which may ask to unlock it, so not on every request.
const SAVE_RETRY_INTERVAL: Duration = Duration::from_secs(30);

/// How many spent refresh tokens a session remembers: more than enough, as
/// only the last one can still be in the store.
const SPENT_KEPT: usize = 8;

#[derive(Default)]
struct State {
    tokens: Option<Tokens>,
    /// Refresh tokens this session has spent, the newest last. The store may
    /// still hold one (a save failed); it is never adopted.
    spent: Vec<String>,
    /// The tokens in memory are newer than the store's: saving them failed.
    unsaved: bool,
    /// When saving last failed.
    save_failed_at: Option<Instant>,
}

impl State {
    fn has_spent(&self, refresh_token: Option<&str>) -> bool {
        refresh_token.is_some_and(|rt| self.spent.iter().any(|s| s == rt))
    }

    fn spend(&mut self, refresh_token: String) {
        if self.spent.len() == SPENT_KEPT {
            self.spent.remove(0);
        }
        self.spent.push(refresh_token);
    }

    /// True if `stored` came after this session's tokens: another process
    /// rotated them, or signed in again. Not the same refresh token, and not
    /// one this session has already spent.
    fn is_newer(&self, stored: &Tokens) -> bool {
        let Some(ours) = &self.tokens else {
            return false;
        };
        let same = match (stored.refresh_token(), ours.refresh_token()) {
            (Some(a), Some(b)) => a == b,
            _ => stored.access_token() == ours.access_token(),
        };
        !same && !self.has_spent(stored.refresh_token())
    }

    /// Takes over tokens another process saved; the store already holds them.
    fn adopt(&mut self, stored: Tokens) {
        self.tokens = Some(stored);
        self.unsaved = false;
    }
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("client_id", &self.cfg.client_id())
            .field("signed_in", &self.is_signed_in())
            .finish_non_exhaustive()
    }
}

impl Session {
    /// A session for freshly obtained tokens (from [`super::PendingSignIn::wait`]);
    /// saves them to `store`.
    ///
    /// # Errors
    ///
    /// Returns the storage error if the new tokens cannot be saved. They are
    /// revoked then (best effort), so a failed save leaves no live grant
    /// behind.
    pub fn start(cfg: OAuthConfig, store: Arc<dyn TokenStore>, tokens: Tokens) -> Result<Self> {
        Self::start_with(cfg, store, tokens, Arc::new(UreqTransport::new()))
    }

    /// The session saved by a previous run, if any.
    ///
    /// # Errors
    ///
    /// Returns an error if the store can't be read (the keyring is missing or
    /// locked). Saved tokens that can't be decoded count as none.
    pub fn restore(cfg: OAuthConfig, store: Arc<dyn TokenStore>) -> Result<Option<Self>> {
        Self::restore_with(cfg, store, Arc::new(UreqTransport::new()))
    }

    pub(crate) fn start_with(
        cfg: OAuthConfig,
        store: Arc<dyn TokenStore>,
        tokens: Tokens,
        http: Arc<dyn HttpTransport>,
    ) -> Result<Self> {
        let saved = {
            let _lock = store_lock(&*store);
            store.save(&tokens)
        };
        if let Err(e) = saved {
            if let Err(revoke) = oauth::revoke(&*http, &cfg, &tokens) {
                eprintln!("hardcover: revoking a sign-in that couldn't be saved: {revoke}");
            }
            return Err(e);
        }
        Ok(Self::with_tokens(cfg, store, http, tokens))
    }

    pub(crate) fn restore_with(
        cfg: OAuthConfig,
        store: Arc<dyn TokenStore>,
        http: Arc<dyn HttpTransport>,
    ) -> Result<Option<Self>> {
        let Some(tokens) = store.load()? else {
            return Ok(None);
        };
        Ok(Some(Self::with_tokens(cfg, store, http, tokens)))
    }

    fn with_tokens(
        cfg: OAuthConfig,
        store: Arc<dyn TokenStore>,
        http: Arc<dyn HttpTransport>,
        tokens: Tokens,
    ) -> Self {
        Self {
            cfg,
            store,
            http,
            state: Mutex::new(State {
                tokens: Some(tokens),
                ..State::default()
            }),
            save_retry_interval: SAVE_RETRY_INTERVAL,
        }
    }

    #[cfg(test)]
    fn with_save_retry_interval(mut self, interval: Duration) -> Self {
        self.save_retry_interval = interval;
        self
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// False after [`Session::sign_out`] or after the refresh token was rejected.
    pub fn is_signed_in(&self) -> bool {
        self.lock().tokens.is_some()
    }

    pub fn expires_at(&self) -> Option<crate::api::Timestamp> {
        self.lock().tokens.as_ref().and_then(Tokens::expires_at)
    }

    /// The granted scopes as recorded (see [`Tokens::scope`]).
    ///
    /// Blocks while a refresh is running, which can take as long as a network
    /// request: read it off the UI thread, or use a snapshot.
    pub fn scope(&self) -> Option<String> {
        self.lock()
            .tokens
            .as_ref()
            .and_then(|t| t.scope().map(str::to_owned))
    }

    /// The granted scopes, for checking features against. Blocks like
    /// [`Session::scope`].
    pub fn granted_scopes(&self) -> GrantedScopes {
        GrantedScopes::for_session(self.scope().as_deref())
    }

    /// Replaces the tokens with those of a new sign-in, e.g. one that granted
    /// more scopes, and saves them. On `Err` the session keeps its old tokens.
    ///
    /// The session stays the same object, so every [`crate::Client`] built on it
    /// uses the new tokens from the next request on. A second session would
    /// keep the old tokens alive in those clients, and its next refresh would
    /// write the old grant back over the new one in the store.
    ///
    /// The old tokens are not revoked: they belong to a different grant, and
    /// are dropped here.
    ///
    /// # Errors
    ///
    /// `Error::Unauthorized` once the session holds no tokens (signed out, or
    /// its refresh token was rejected): a sign-in that finishes after that
    /// must not sign the user back in. Revoke those tokens with
    /// [`Session::discard`].
    pub fn replace(&self, tokens: Tokens) -> Result<()> {
        let mut state = self.lock();
        if state.tokens.is_none() {
            return Err(Error::Unauthorized);
        }
        let _lock = store_lock(&*self.store);
        self.store.save(&tokens)?;
        state.tokens = Some(tokens);
        state.unsaved = false;
        Ok(())
    }

    /// Revokes tokens from a sign-in that will not replace this session's,
    /// e.g. ones another account approved. The session itself is unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error if token revocation fails.
    pub fn discard(&self, tokens: &Tokens) -> Result<()> {
        oauth::revoke(&*self.http, &self.cfg, tokens)
    }

    /// Revokes the tokens at Hardcover, then forgets them locally (store and memory).
    ///
    /// # Errors
    ///
    /// The local sign-out always happens. An `Err` means the server-side revoke
    /// (or clearing the store) failed; the user is signed out of this app either way.
    pub fn sign_out(&self) -> Result<()> {
        let mut state = self.lock();
        let tokens = state.tokens.take();
        state.unsaved = false;
        let _lock = store_lock(&*self.store);
        let revoked = match &tokens {
            Some(t) => oauth::revoke(&*self.http, &self.cfg, t),
            None => Ok(()),
        };
        self.store.clear()?;
        revoked
    }

    /// The store's tokens if another process saved newer ones than this
    /// session holds. A store that can't be read says nothing.
    fn newer_in_store(&self, state: &State) -> Option<Tokens> {
        match self.store.load() {
            Ok(Some(stored)) if state.is_newer(&stored) => Some(stored),
            Ok(_) => None,
            Err(e) => {
                eprintln!("hardcover: could not read the saved sign-in: {e}");
                None
            }
        }
    }

    /// Saves the tokens in memory. A failure is remembered and tried again on
    /// the next call: the tokens work, and failing the request that needed
    /// them would help nobody.
    fn save(&self, state: &mut State) {
        let Some(tokens) = &state.tokens else {
            return;
        };
        match self.store.save(tokens) {
            Ok(()) => {
                state.unsaved = false;
                state.save_failed_at = None;
            }
            Err(e) => {
                eprintln!("hardcover: could not save the refreshed sign-in, will retry: {e}");
                state.unsaved = true;
                state.save_failed_at = Some(Instant::now());
            }
        }
    }

    /// Tries again to save tokens whose save failed, unless another process
    /// has saved newer ones meanwhile.
    fn retry_save(&self, state: &mut State) {
        let _lock = store_lock(&*self.store);
        match self.newer_in_store(state) {
            Some(stored) => state.adopt(stored),
            None => self.save(state),
        }
    }

    /// Refreshes while holding the session's lock, under the store's.
    /// `enough` says whether tokens another process saved meanwhile will do
    /// without a refresh of their own. On a dead grant the session is dropped.
    fn refresh_locked(&self, state: &mut State, enough: impl Fn(&Tokens) -> bool) -> Result<()> {
        let _lock = store_lock(&*self.store);
        if let Some(stored) = self.newer_in_store(state) {
            let done = enough(&stored);
            state.adopt(stored);
            if done {
                return Ok(());
            }
        }
        let current = state.tokens.as_ref().ok_or(Error::Unauthorized)?;
        let refresh_token = current
            .refresh_token()
            .map(str::to_owned)
            .ok_or(Error::Unauthorized)?;
        let previous_scope = current.scope().map(str::to_owned);
        match oauth::refresh(&*self.http, &self.cfg, &refresh_token) {
            Ok(new) => {
                // Hardcover always rotates, but keep the old refresh token if a
                // server ever omits it. A response without a scope did not
                // change it (RFC 6749 §5.1): keep the recorded one.
                let scope = new.scope().map(str::to_owned).or(previous_scope);
                let rotated = new.refresh_token().map(str::to_owned);
                let new = Tokens::new(
                    new.access_token(),
                    rotated.clone().or_else(|| Some(refresh_token.clone())),
                    new.expires_at(),
                )
                .with_scope(scope);
                if rotated.is_some_and(|rt| rt != refresh_token) {
                    state.spend(refresh_token);
                }
                // In memory first: the old refresh token is spent either way.
                state.tokens = Some(new);
                self.save(state);
                Ok(())
            }
            Err(e) if e.grant_is_dead() => {
                // A process that doesn't take the store's lock (an older
                // version) may have rotated the token meanwhile: then this
                // rejection says nothing about the token in the store.
                match self.store.load() {
                    Ok(Some(stored)) if state.is_newer(&stored) => {
                        state.adopt(stored);
                        return Ok(());
                    }
                    // The rejected token, or one this session spent: dead.
                    Ok(Some(_)) => {
                        if let Err(e) = self.store.clear() {
                            eprintln!("hardcover: could not clear the rejected sign-in: {e}");
                        }
                    }
                    Ok(None) => {}
                    // Can't tell what is there: leave it alone.
                    Err(e) => eprintln!("hardcover: could not read the saved sign-in: {e}"),
                }
                state.tokens = None;
                state.unsaved = false;
                Err(Error::Unauthorized)
            }
            Err(e) => Err(e.into_error()),
        }
    }
}

/// The store's cross-process lock. Failing to take it is logged and
/// otherwise ignored: the store is read again under it anyway, which is
/// what protects the tokens; the lock only keeps two refreshes apart.
fn store_lock(store: &dyn TokenStore) -> Option<FileLock> {
    store.lock().unwrap_or_else(|e| {
        eprintln!("hardcover: refreshing the sign-in without the cross-process lock: {e}");
        None
    })
}

impl TokenSource for Session {
    fn access_token(&self) -> Result<String> {
        let mut state = self.lock();
        if state.unsaved
            && state
                .save_failed_at
                .is_none_or(|at| at.elapsed() >= self.save_retry_interval)
        {
            self.retry_save(&mut state);
        }
        let tokens = state.tokens.as_ref().ok_or(Error::Unauthorized)?;
        let now = chrono::Utc::now();
        let margin = chrono::Duration::minutes(REFRESH_MARGIN_MINUTES);
        if tokens.expires_within(now, margin) && tokens.refresh_token().is_some() {
            match self.refresh_locked(&mut state, |t| !t.expires_within(now, margin)) {
                Ok(()) | Err(Error::Unauthorized) => {}
                // Anything else (the network, a proxy's page, the server's
                // trouble) during a proactive refresh: keep using the token
                // while it is still valid.
                Err(e) => {
                    return state
                        .tokens
                        .as_ref()
                        .filter(|t| !t.expires_within(now, chrono::Duration::zero()))
                        .map(|t| t.access_token().to_owned())
                        .ok_or(e);
                }
            }
        }
        state
            .tokens
            .as_ref()
            .map(|t| t.access_token().to_owned())
            .ok_or(Error::Unauthorized)
    }

    fn on_unauthorized(&self, rejected: &str) -> Result<bool> {
        let mut state = self.lock();
        match state.tokens.as_ref() {
            None => Ok(false),
            // Someone else refreshed since that request was sent.
            Some(t) if t.access_token() != rejected => Ok(true),
            Some(t) if t.refresh_token().is_none() => Ok(false),
            Some(_) => match self.refresh_locked(&mut state, |t| t.access_token() != rejected) {
                Ok(()) => Ok(true),
                Err(Error::Unauthorized) => Ok(false),
                Err(e) => Err(e),
            },
        }
    }
}

/// A fixed bearer token, e.g. a personal access token for development. No refresh.
#[derive(Clone)]
pub struct StaticToken {
    token: String,
}

impl StaticToken {
    /// Environment variable read by [`StaticToken::from_env`].
    pub const ENV: &'static str = "HARDCOVER_TOKEN";

    /// Accepts the token with or without a `Bearer ` prefix or surrounding quotes.
    pub fn new(token: impl AsRef<str>) -> Option<Self> {
        let t = token
            .as_ref()
            .trim()
            .trim_matches(|c| c == '"' || c == '\'')
            .trim();
        let t = t
            .strip_prefix("Bearer ")
            .or_else(|| t.strip_prefix("bearer "))
            .unwrap_or(t)
            .trim();
        (!t.is_empty()).then(|| Self {
            token: t.to_owned(),
        })
    }

    /// `HARDCOVER_TOKEN`, if set.
    pub fn from_env() -> Option<Self> {
        std::env::var(Self::ENV).ok().and_then(Self::new)
    }
}

impl fmt::Debug for StaticToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StaticToken(<redacted>)")
    }
}

impl TokenSource for StaticToken {
    fn access_token(&self) -> Result<String> {
        Ok(self.token.clone())
    }

    fn on_unauthorized(&self, _rejected: &str) -> Result<bool> {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::MemoryTokenStore;
    use crate::http::fake::FakeTransport;

    fn soon(minutes: i64) -> crate::api::Timestamp {
        chrono::Utc::now() + chrono::Duration::minutes(minutes)
    }

    fn session(tokens: Tokens) -> (Session, Arc<FakeTransport>, Arc<MemoryTokenStore>) {
        let http = Arc::new(FakeTransport::new());
        let store = Arc::new(MemoryTokenStore::new());
        let s = Session::start_with(OAuthConfig::new("cid"), store.clone(), tokens, http.clone())
            .unwrap();
        (s, http, store)
    }

    const ROTATED: &str = r#"{"access_token":"at2","refresh_token":"rt2","expires_in":604800}"#;

    #[test]
    fn fresh_token_is_used_as_is() {
        let (s, http, _) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(60 * 24))));
        assert_eq!(s.access_token().unwrap(), "at1");
        assert!(http.requests().is_empty());
    }

    #[test]
    fn refreshes_near_expiry_and_persists_rotation() {
        let (s, http, store) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(2))));
        http.push(200, ROTATED);
        assert_eq!(s.access_token().unwrap(), "at2");
        let req = &http.requests()[0];
        assert_eq!(req.form_value("grant_type"), Some("refresh_token"));
        assert_eq!(req.form_value("refresh_token"), Some("rt1"));
        assert_eq!(req.form_value("client_id"), Some("cid"));
        let saved = store.load().unwrap().unwrap();
        assert_eq!(
            (saved.access_token(), saved.refresh_token()),
            ("at2", Some("rt2"))
        );
        // Next call uses the new token without another refresh.
        assert_eq!(s.access_token().unwrap(), "at2");
        assert_eq!(http.requests().len(), 1);
    }

    #[test]
    fn proactive_refresh_network_error_falls_back_while_valid() {
        let (s, http, _) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(2))));
        http.push_err(Error::Network("down".into()));
        assert_eq!(s.access_token().unwrap(), "at1");
        let (s, http, _) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(-1))));
        http.push_err(Error::Network("down".into()));
        assert!(matches!(s.access_token(), Err(Error::Network(_))));
    }

    #[test]
    fn unauthorized_refreshes_once_per_token() {
        let (s, http, _) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(600))));
        http.push(200, ROTATED);
        assert!(s.on_unauthorized("at1").unwrap());
        // A second 401 for the old token (a concurrent request) does not refresh again.
        assert!(s.on_unauthorized("at1").unwrap());
        assert_eq!(http.requests().len(), 1);
        assert_eq!(s.access_token().unwrap(), "at2");
    }

    #[test]
    fn rejected_refresh_signs_out() {
        let (s, http, store) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(600))));
        http.push(400, r#"{"error":"invalid_grant"}"#);
        assert!(!s.on_unauthorized("at1").unwrap());
        assert!(!s.is_signed_in());
        assert!(store.load().unwrap().is_none());
        assert_eq!(s.access_token(), Err(Error::Unauthorized));
    }

    #[test]
    fn concurrent_callers_refresh_once() {
        let (s, http, _) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(1))));
        http.push(200, ROTATED);
        let s = Arc::new(s);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let s = s.clone();
                std::thread::spawn(move || s.access_token().unwrap())
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), "at2");
        }
        assert_eq!(http.requests().len(), 1);
    }

    #[test]
    fn sign_out_revokes_and_clears() {
        let (s, http, store) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(600))));
        http.push(200, "");
        s.sign_out().unwrap();
        assert!(!s.is_signed_in());
        assert!(store.load().unwrap().is_none());
        assert_eq!(http.requests()[0].form_value("token"), Some("rt1"));
        // Revoke failure still signs out locally.
        let (s, http, store) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(600))));
        http.push(503, "");
        assert!(s.sign_out().is_err());
        assert!(!s.is_signed_in());
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn refresh_keeps_the_scope_unless_the_server_reports_a_new_one() {
        let granted = "read:me read:library";
        let (s, http, store) = session(
            Tokens::new("at1", Some("rt1".into()), Some(soon(2))).with_scope(Some(granted.into())),
        );
        // ROTATED has no `scope`.
        http.push(200, ROTATED);
        assert_eq!(s.access_token().unwrap(), "at2");
        assert_eq!(s.scope().as_deref(), Some(granted));
        assert_eq!(store.load().unwrap().unwrap().scope(), Some(granted));

        let (s, http, store) = session(
            Tokens::new("at1", Some("rt1".into()), Some(soon(2))).with_scope(Some(granted.into())),
        );
        http.push(
            200,
            r#"{"access_token":"at2","refresh_token":"rt2","expires_in":604800,"scope":"read:me"}"#,
        );
        assert_eq!(s.access_token().unwrap(), "at2");
        assert_eq!(s.scope().as_deref(), Some("read:me"));
        assert_eq!(store.load().unwrap().unwrap().scope(), Some("read:me"));
    }

    #[test]
    fn refresh_without_a_rotated_token_keeps_the_old_one_and_the_scope() {
        let (s, http, store) = session(
            Tokens::new("at1", Some("rt1".into()), Some(soon(2)))
                .with_scope(Some("read:me".into())),
        );
        http.push(200, r#"{"access_token":"at2","expires_in":604800}"#);
        assert_eq!(s.access_token().unwrap(), "at2");
        let saved = store.load().unwrap().unwrap();
        assert_eq!(saved.refresh_token(), Some("rt1"));
        assert_eq!(saved.scope(), Some("read:me"));
    }

    #[test]
    fn granted_scopes_follow_the_recorded_scope() {
        use crate::auth::scopes::{READ_GOALS, READ_LIBRARY};
        let (s, _, _) = session(Tokens::new("at1", None, None));
        // Saved by an older version: the scopes it requested.
        assert!(s.granted_scopes().contains(READ_LIBRARY));
        assert!(!s.granted_scopes().contains(READ_GOALS));
        let (s, _, _) =
            session(Tokens::new("at1", None, None).with_scope(Some("read:goals".into())));
        assert!(s.granted_scopes().contains(READ_GOALS));
        assert!(!s.granted_scopes().contains(READ_LIBRARY));
    }

    #[test]
    fn replace_swaps_tokens_for_every_user_of_the_session() {
        let (s, http, store) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(600))));
        let s = Arc::new(s);
        let client_side = s.clone();
        s.replace(
            Tokens::new("at9", Some("rt9".into()), Some(soon(600)))
                .with_scope(Some("read:me read:goals".into())),
        )
        .unwrap();
        assert_eq!(client_side.access_token().unwrap(), "at9");
        assert_eq!(client_side.scope().as_deref(), Some("read:me read:goals"));
        let saved = store.load().unwrap().unwrap();
        assert_eq!(
            (saved.access_token(), saved.refresh_token()),
            ("at9", Some("rt9"))
        );
        // The old grant is neither refreshed nor revoked.
        assert!(http.requests().is_empty());
        // A 401 for the old token now just retries with the new one.
        assert!(client_side.on_unauthorized("at1").unwrap());
        assert!(http.requests().is_empty());
    }

    #[test]
    fn replace_keeps_the_old_tokens_when_saving_fails() {
        struct FailingStore;
        impl TokenStore for FailingStore {
            fn load(&self) -> Result<Option<Tokens>> {
                Ok(None)
            }
            fn save(&self, _: &Tokens) -> Result<()> {
                Err(Error::Storage("locked".into()))
            }
            fn clear(&self) -> Result<()> {
                Ok(())
            }
        }
        let s = Session::with_tokens(
            OAuthConfig::new("cid"),
            Arc::new(FailingStore),
            Arc::new(FakeTransport::new()),
            Tokens::new("at1", None, None),
        );
        assert!(s.replace(Tokens::new("at2", None, None)).is_err());
        assert_eq!(s.access_token().unwrap(), "at1");
    }

    #[test]
    fn replace_after_sign_out_is_refused() {
        let (s, http, store) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(600))));
        http.push(200, "");
        s.sign_out().unwrap();
        assert_eq!(
            s.replace(Tokens::new("at2", Some("rt2".into()), Some(soon(600)))),
            Err(Error::Unauthorized)
        );
        assert!(!s.is_signed_in());
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn discard_revokes_other_tokens_and_keeps_the_session() {
        let (s, http, store) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(600))));
        http.push(200, "");
        s.discard(&Tokens::new("at9", Some("rt9".into()), Some(soon(600))))
            .unwrap();
        assert_eq!(http.requests()[0].form_value("token"), Some("rt9"));
        assert_eq!(s.access_token().unwrap(), "at1");
        assert_eq!(store.load().unwrap().unwrap().access_token(), "at1");
    }

    #[test]
    fn restore_reads_store() {
        let store = Arc::new(MemoryTokenStore::new());
        let http = Arc::new(FakeTransport::new());
        assert!(
            Session::restore_with(OAuthConfig::new("c"), store.clone(), http.clone())
                .unwrap()
                .is_none()
        );
        store.save(&Tokens::new("at", None, None)).unwrap();
        let s = Session::restore_with(OAuthConfig::new("c"), store, http)
            .unwrap()
            .unwrap();
        assert_eq!(s.access_token().unwrap(), "at");
        assert!(!s.on_unauthorized("at").unwrap());
    }

    /// A store shared by two sessions, as the keyring is by two processes.
    fn two_processes(
        tokens: Tokens,
    ) -> (Arc<MemoryTokenStore>, [(Session, Arc<FakeTransport>); 2]) {
        let store = Arc::new(MemoryTokenStore::new());
        store.save(&tokens).unwrap();
        let open = || {
            let http = Arc::new(FakeTransport::new());
            let s = Session::restore_with(OAuthConfig::new("cid"), store.clone(), http.clone())
                .unwrap()
                .unwrap();
            (s, http)
        };
        let pair = [open(), open()];
        (store, pair)
    }

    fn saved(store: &MemoryTokenStore) -> Option<(String, Option<String>)> {
        store.load().unwrap().map(|t| {
            (
                t.access_token().to_owned(),
                t.refresh_token().map(str::to_owned),
            )
        })
    }

    #[test]
    fn a_second_process_adopts_the_rotation_instead_of_replaying_it() {
        let (store, [(a, a_http), (b, b_http)]) =
            two_processes(Tokens::new("at1", Some("rt1".into()), Some(soon(2))));
        a_http.push(200, ROTATED);
        assert_eq!(a.access_token().unwrap(), "at2");
        // B still holds rt1 in memory. It must not send it (nothing is
        // scripted for B: a request would panic), and must keep rt2 saved.
        assert_eq!(b.access_token().unwrap(), "at2");
        assert!(b_http.requests().is_empty());
        assert!(b.is_signed_in());
        assert_eq!(saved(&store), Some(("at2".into(), Some("rt2".into()))));
    }

    #[test]
    fn a_401_adopts_another_process_s_rotation() {
        let (store, [(a, a_http), (b, b_http)]) =
            two_processes(Tokens::new("at1", Some("rt1".into()), Some(soon(600))));
        a_http.push(200, ROTATED);
        assert!(a.on_unauthorized("at1").unwrap());
        assert!(b.on_unauthorized("at1").unwrap());
        assert!(b_http.requests().is_empty());
        assert_eq!(b.access_token().unwrap(), "at2");
        assert_eq!(saved(&store), Some(("at2".into(), Some("rt2".into()))));
    }

    #[test]
    fn a_rejection_while_another_process_rotated_keeps_its_token() {
        // An older version, which takes no lock, rotates rt1 into rt9 while
        // this session's refresh with rt1 is in flight.
        let (store, [(b, b_http), _]) =
            two_processes(Tokens::new("at1", Some("rt1".into()), Some(soon(600))));
        let other = store.clone();
        b_http.on_request(move |_| {
            other
                .save(&Tokens::new("at9", Some("rt9".into()), Some(soon(600))))
                .unwrap();
        });
        b_http.push(400, r#"{"error":"invalid_grant"}"#);
        assert!(b.on_unauthorized("at1").unwrap());
        assert!(b.is_signed_in());
        assert_eq!(b.access_token().unwrap(), "at9");
        assert_eq!(saved(&store), Some(("at9".into(), Some("rt9".into()))));
    }

    #[test]
    fn a_proxy_s_error_page_keeps_the_session() {
        // A captive portal, a WAF or a CDN answering for the token endpoint.
        let portal = "<html><body>Sign in to the Wi-Fi</body></html>";
        let (s, http, store) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(600))));
        http.push(403, portal);
        assert!(matches!(s.on_unauthorized("at1"), Err(Error::Network(_))));
        assert!(s.is_signed_in());
        assert_eq!(saved(&store), Some(("at1".into(), Some("rt1".into()))));
        // Proactively, the token still in date is used.
        let (s, http, store) = session(Tokens::new("at1", Some("rt1".into()), Some(soon(2))));
        http.push(403, portal);
        assert_eq!(s.access_token().unwrap(), "at1");
        http.push(400, r#"{"error":"invalid_request"}"#);
        assert!(matches!(s.on_unauthorized("at1"), Err(Error::Auth(_))));
        assert!(s.is_signed_in());
        assert!(store.load().unwrap().is_some());
    }

    /// A store whose saves fail while `failing` is set, like a secret service
    /// that is restarting.
    #[derive(Default)]
    struct FlakyStore {
        inner: MemoryTokenStore,
        failing: std::sync::atomic::AtomicBool,
    }

    impl FlakyStore {
        fn fail(&self, failing: bool) {
            self.failing
                .store(failing, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl TokenStore for FlakyStore {
        fn load(&self) -> Result<Option<Tokens>> {
            self.inner.load()
        }
        fn save(&self, tokens: &Tokens) -> Result<()> {
            if self.failing.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(Error::Storage("keyring: not available".into()));
            }
            self.inner.save(tokens)
        }
        fn clear(&self) -> Result<()> {
            self.inner.clear()
        }
    }

    #[test]
    fn a_failed_save_of_a_rotation_is_retried_and_never_replayed() {
        let store = Arc::new(FlakyStore::default());
        let http = Arc::new(FakeTransport::new());
        let s = Session::start_with(
            OAuthConfig::new("cid"),
            store.clone(),
            Tokens::new("at1", Some("rt1".into()), Some(soon(2))),
            http.clone(),
        )
        .unwrap()
        .with_save_retry_interval(Duration::ZERO);
        store.fail(true);
        http.push(200, ROTATED);
        // The tokens are fine in memory: the call succeeds.
        assert_eq!(s.access_token().unwrap(), "at2");
        assert_eq!(
            store.inner.load().unwrap().unwrap().refresh_token(),
            Some("rt1")
        );
        // A refresh meanwhile uses rt2; the spent rt1 in the store is never adopted.
        http.push(
            200,
            r#"{"access_token":"at3","refresh_token":"rt3","expires_in":604800}"#,
        );
        assert!(s.on_unauthorized("at2").unwrap());
        assert_eq!(http.requests()[1].form_value("refresh_token"), Some("rt2"));
        assert_eq!(
            store.inner.load().unwrap().unwrap().refresh_token(),
            Some("rt1")
        );
        // Once the keyring is back, the next try saves.
        store.fail(false);
        assert_eq!(s.access_token().unwrap(), "at3");
        let stored = store.inner.load().unwrap().unwrap();
        assert_eq!(
            (stored.access_token(), stored.refresh_token()),
            ("at3", Some("rt3"))
        );
        assert_eq!(http.requests().len(), 2);
    }

    #[test]
    fn failed_saves_are_retried_at_most_every_interval() {
        let store = Arc::new(FlakyStore::default());
        let http = Arc::new(FakeTransport::new());
        let s = Session::start_with(
            OAuthConfig::new("cid"),
            store.clone(),
            Tokens::new("at1", Some("rt1".into()), Some(soon(2))),
            http.clone(),
        )
        .unwrap();
        store.fail(true);
        http.push(200, ROTATED);
        assert_eq!(s.access_token().unwrap(), "at2");
        // The keyring is back, but the last try was just now.
        store.fail(false);
        assert_eq!(s.access_token().unwrap(), "at2");
        assert_eq!(
            store.inner.load().unwrap().unwrap().refresh_token(),
            Some("rt1")
        );
    }

    #[test]
    fn a_sign_in_that_cannot_be_saved_is_revoked() {
        let store = Arc::new(FlakyStore::default());
        store.fail(true);
        let http = Arc::new(FakeTransport::new());
        http.push(200, "");
        let started = Session::start_with(
            OAuthConfig::new("cid"),
            store,
            Tokens::new("at1", Some("rt1".into()), Some(soon(600))),
            http.clone(),
        );
        assert!(matches!(started, Err(Error::Storage(_))));
        let revoke = &http.requests()[0];
        assert_eq!(revoke.url, "https://api.hardcover.app/oauth2/revoke");
        assert_eq!(revoke.form_value("token"), Some("rt1"));
    }

    #[test]
    fn a_rejected_token_is_cleared_only_if_the_store_still_holds_it() {
        // The store holds the token this session spent (its last save
        // failed): that one is dead too, so it goes.
        let store = Arc::new(FlakyStore::default());
        let http = Arc::new(FakeTransport::new());
        let s = Session::start_with(
            OAuthConfig::new("cid"),
            store.clone(),
            Tokens::new("at1", Some("rt1".into()), Some(soon(2))),
            http.clone(),
        )
        .unwrap();
        store.fail(true);
        http.push(200, ROTATED);
        assert_eq!(s.access_token().unwrap(), "at2");
        store.fail(false);
        // The store still holds rt1, spent; rt2 is rejected, and the spent
        // token is cleared with it.
        assert_eq!(
            store.inner.load().unwrap().unwrap().refresh_token(),
            Some("rt1")
        );
        http.push(400, r#"{"error":"invalid_grant"}"#);
        assert!(!s.on_unauthorized("at2").unwrap());
        assert!(!s.is_signed_in());
        assert!(store.load().unwrap().is_none());
    }

    #[test]
    fn static_token_normalizes_and_redacts() {
        assert_eq!(
            StaticToken::new(" \"hc_pat_x\" \n")
                .unwrap()
                .access_token()
                .unwrap(),
            "hc_pat_x"
        );
        assert_eq!(
            StaticToken::new("Bearer abc")
                .unwrap()
                .access_token()
                .unwrap(),
            "abc"
        );
        assert!(StaticToken::new("  ").is_none());
        let t = StaticToken::new("secret-value").unwrap();
        assert!(!format!("{t:?}").contains("secret"));
        assert!(!t.on_unauthorized("secret-value").unwrap());
    }
}
