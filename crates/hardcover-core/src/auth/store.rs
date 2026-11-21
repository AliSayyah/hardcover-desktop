//! Where tokens live between runs.

use std::path::PathBuf;
use std::sync::Mutex;

use super::{DEFAULT_CLIENT_ID, Tokens};
use crate::lock::FileLock;
use crate::{Error, Result};

/// Persistent storage for [`Tokens`].
pub trait TokenStore: Send + Sync {
    /// Loads the saved tokens, if present.
    ///
    /// # Errors
    ///
    /// Returns an error if the stored credentials cannot be read or decoded.
    fn load(&self) -> Result<Option<Tokens>>;
    /// Persists the tokens for the next session.
    ///
    /// # Errors
    ///
    /// Returns an error if the credentials cannot be persisted.
    fn save(&self, tokens: &Tokens) -> Result<()>;
    /// Succeeds if there was nothing to clear.
    ///
    /// # Errors
    ///
    /// Returns an error if the credentials cannot be removed.
    fn clear(&self) -> Result<()>;
    /// Holds off every other process that uses the same tokens (another
    /// copy of the app, an example, `api_check`) while this one reads,
    /// refreshes and saves them; released when dropped. `None` where nothing
    /// else can reach the store, as for the default.
    ///
    /// # Errors
    ///
    /// Returns an error if the lock can't be taken.
    fn lock(&self) -> Result<Option<FileLock>> {
        Ok(None)
    }
}

/// The desktop's secret service (GNOME Keyring, `KWallet`, ...) via `keyring`,
/// service `"hardcover-desktop"`. Tokens are stored as one JSON secret, in an
/// entry per OAuth client ([`KeyringTokenStore::account_for`]).
pub struct KeyringTokenStore {
    entry: keyring::Entry,
    /// The cross-process lock beside the entry: in `$XDG_RUNTIME_DIR`, else
    /// in the data directory.
    lock_path: Option<PathBuf>,
}

impl KeyringTokenStore {
    pub const SERVICE: &'static str = "hardcover-desktop";
    /// The registered client's entry ([`DEFAULT_CLIENT_ID`]).
    pub const DEFAULT_ACCOUNT: &'static str = "oauth-tokens";
    /// A throwaway entry for [`KeyringTokenStore::check_available`], never the tokens'.
    const PROBE_ACCOUNT: &'static str = "keyring-check";

    /// Opens the entry for the tokens `client_id` was issued. Refresh tokens
    /// are bound to their client, so a session saved by one client must never
    /// be refreshed by another (`HARDCOVER_CLIENT_ID`).
    ///
    /// # Errors
    ///
    /// Returns an error if the keyring cannot initialize the entry.
    pub fn for_client(client_id: &str) -> Result<Self> {
        let account = Self::account_for(client_id);
        let lock_path = crate::project_dirs().ok().map(|dirs| {
            dirs.runtime_dir()
                .unwrap_or_else(|| dirs.data_dir())
                .join(lock_file_name(&account))
        });
        Ok(Self {
            entry: keyring::Entry::new(Self::SERVICE, &account).map_err(keyring_err)?,
            lock_path,
        })
    }

    /// The keyring account holding `client_id`'s tokens: the registered
    /// client keeps [`Self::DEFAULT_ACCOUNT`], any other gets its own.
    #[must_use]
    pub fn account_for(client_id: &str) -> String {
        if client_id == DEFAULT_CLIENT_ID {
            Self::DEFAULT_ACCOUNT.to_owned()
        } else {
            format!("{}:{client_id}", Self::DEFAULT_ACCOUNT)
        }
    }

    /// Checks that a secret service is running and takes a write, by saving
    /// and deleting a throwaway entry beside the tokens. A locked keyring may
    /// ask the user to unlock it. Run it before a sign-in, so the user hears
    /// about a missing keyring before approving in the browser.
    ///
    /// # Errors
    ///
    /// The keyring's error if it can't store a secret.
    pub fn check_available() -> Result<()> {
        let probe = keyring::Entry::new(Self::SERVICE, Self::PROBE_ACCOUNT).map_err(keyring_err)?;
        probe.set_password("ok").map_err(keyring_err)?;
        match probe.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_err(e)),
        }
    }
}

/// The lock file's name for a keyring account: the account, with anything
/// but letters, digits, `-` and `_` replaced.
fn lock_file_name(account: &str) -> String {
    let safe: String = account
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}.lock")
}

impl std::fmt::Debug for KeyringTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyringTokenStore").finish_non_exhaustive()
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err passes the owned error to this conversion"
)]
fn keyring_err(e: keyring::Error) -> Error {
    // keyring errors describe the platform failure, never the secret.
    Error::Storage(format!("keyring: {e}"))
}

impl TokenStore for KeyringTokenStore {
    /// An entry that doesn't decode counts as none: it can't be used, and
    /// the next sign-in overwrites it. An `Err` is the keyring's own failure
    /// (no secret service, a locked collection), which may pass.
    fn load(&self) -> Result<Option<Tokens>> {
        match self.entry.get_password() {
            Ok(json) => Ok(serde_json::from_str(&json).ok().or_else(|| {
                eprintln!("hardcover: the saved sign-in is unreadable; ignoring it");
                None
            })),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_err(e)),
        }
    }

    fn save(&self, tokens: &Tokens) -> Result<()> {
        let json = serde_json::to_string(tokens).map_err(|e| Error::Storage(e.to_string()))?;
        self.entry.set_password(&json).map_err(keyring_err)
    }

    fn clear(&self) -> Result<()> {
        match self.entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_err(e)),
        }
    }

    fn lock(&self) -> Result<Option<FileLock>> {
        let Some(path) = &self.lock_path else {
            return Ok(None);
        };
        FileLock::acquire(path)
            .map(Some)
            .map_err(|e| Error::Storage(format!("could not lock {}: {e}", path.display())))
    }
}

/// In-memory store, for tests and for "don't remember me".
#[derive(Default)]
pub struct MemoryTokenStore {
    tokens: Mutex<Option<Tokens>>,
}

impl MemoryTokenStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for MemoryTokenStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryTokenStore").finish_non_exhaustive()
    }
}

impl TokenStore for MemoryTokenStore {
    fn load(&self) -> Result<Option<Tokens>> {
        Ok(self
            .tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone())
    }

    fn save(&self, tokens: &Tokens) -> Result<()> {
        *self
            .tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(tokens.clone());
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        *self
            .tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_client_has_its_own_entry_and_lock() {
        assert_eq!(
            KeyringTokenStore::account_for(DEFAULT_CLIENT_ID),
            "oauth-tokens"
        );
        let other = KeyringTokenStore::account_for("my-dev-client");
        assert_eq!(other, "oauth-tokens:my-dev-client");
        assert_eq!(lock_file_name("oauth-tokens"), "oauth-tokens.lock");
        assert_eq!(lock_file_name(&other), "oauth-tokens_my-dev-client.lock");
        assert_eq!(
            lock_file_name("oauth-tokens:../x"),
            "oauth-tokens____x.lock"
        );
    }
}
