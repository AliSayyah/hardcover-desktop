//! Signing in: the sign-in screen shown without a session, and the notice
//! that asks a session for scopes it lacks.

use hardcover_core::Error;

pub mod reauthorize;
pub mod sign_in_view;

/// Why the keyring can't keep the sign-in, from its error: the keyring's own
/// words, without the prefix every one of them has.
fn keyring_detail(error: &Error) -> String {
    let detail = match error {
        Error::Storage(msg) => msg.as_str(),
        other => return other.to_string(),
    };
    detail
        .trim_start_matches("keyring:")
        .trim()
        .trim_end_matches('.')
        .to_owned()
}

/// Before or during a sign-in: the keyring can't store it, so signing in
/// can't work until it can.
pub(crate) fn keyring_message(error: &Error) -> String {
    format!(
        "Hardcover Desktop keeps your sign-in in the system keyring, which isn't available ({}). \
         Start or unlock your keyring, such as GNOME Keyring or KWallet, then try again.",
        keyring_detail(error)
    )
}

/// At startup: a saved sign-in may be in the keyring, which can't be read.
pub(crate) fn saved_sign_in_message(error: &Error) -> String {
    format!(
        "Couldn't read your saved sign-in from the system keyring ({}). \
         Start or unlock your keyring, such as GNOME Keyring or KWallet, then try again.",
        keyring_detail(error)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyring_messages_say_what_failed_and_what_to_do() {
        let error = Error::Storage("keyring: Platform secure storage failure: no service.".into());
        let message = keyring_message(&error);
        assert!(
            message.contains("(Platform secure storage failure: no service)"),
            "{message}"
        );
        assert!(message.ends_with("then try again."));
        assert!(
            saved_sign_in_message(&error).starts_with("Couldn't read your saved sign-in"),
            "{}",
            saved_sign_in_message(&error)
        );
    }
}
