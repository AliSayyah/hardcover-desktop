//! Signing in: the sign-in screen shown without a session, and the notice
//! that asks a session for scopes it lacks.

pub mod reauthorize;
pub mod sign_in_view;

/// Before or during a sign-in: the keyring can't store it, so signing in
/// can't work until it can. The keyring's own error goes to the log.
pub(crate) const KEYRING_UNAVAILABLE: &str = "Hardcover Desktop keeps your sign-in in the system keyring, and it isn't available. \
     Start or unlock your keyring, such as GNOME Keyring or KWallet, then try again.";

/// At startup: a saved sign-in may be in the keyring, which can't be read.
pub(crate) const SAVED_SIGN_IN_UNREADABLE: &str = "Couldn't read your saved sign-in from the system keyring. \
     Start or unlock your keyring, such as GNOME Keyring or KWallet, then try again.";
