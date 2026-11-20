//! Which OAuth scopes a session holds, and whether they cover a feature.
//!
//! Sign-in requests [`super::DEFAULT_SCOPES`]. A session saved by an older
//! version holds fewer, so features check [`GrantedScopes::contains`] and the
//! app asks the user to sign in again for the rest, instead of letting
//! requests fail with `insufficient_scope`.

use std::collections::BTreeSet;

/// Profile: who is signed in.
pub const READ_ME: &str = "read:me";
/// Catalog lookups and search.
pub const READ_CATALOG: &str = "read:catalog";
/// The user's library and reading progress.
pub const READ_LIBRARY: &str = "read:library";
/// Adding, changing and removing library entries.
pub const WRITE_LIBRARY: &str = "write:library";
/// Writing reviews. Grants no operations of its own, only the `review*`
/// arguments of `insert_user_book` and `update_user_book`, which also need
/// [`WRITE_LIBRARY`].
pub const WRITE_REVIEWS: &str = "write:reviews";
/// Reading goals.
pub const READ_GOALS: &str = "read:goals";
/// Creating, changing and deleting reading goals.
pub const WRITE_GOALS: &str = "write:goals";
/// Lists (the user's own and followed ones).
pub const READ_LISTS: &str = "read:lists";
/// Creating and changing lists.
pub const WRITE_LISTS: &str = "write:lists";
/// The user's reading journal (notes, quotes, progress events). Writing it
/// needs only [`WRITE_LIBRARY`].
pub const READ_JOURNAL: &str = "read:journal";
/// Other users' content: their library entries, reviews and activity.
pub const READ_SOCIAL: &str = "read:social";
/// Other users' public profiles.
pub const READ_USERS: &str = "read:users";
/// The user's notifications.
pub const READ_NOTIFICATIONS: &str = "read:notifications";
/// Marking notifications read.
pub const WRITE_NOTIFICATIONS: &str = "write:notifications";

/// What versions up to 0.1.x requested. A session saved without a granted
/// scope is assumed to hold exactly these.
pub const LEGACY_SCOPES: [&str; 4] = [READ_ME, READ_CATALOG, READ_LIBRARY, WRITE_LIBRARY];

/// Grants everything, including scopes added later.
const ALL: &str = "all";

/// Scope -> the narrower scopes it implies, from `implies` in Hardcover's scope
/// map (`schema/hardcover-capabilities.json`). `all` is handled separately.
/// A test keeps this in step with the committed map.
const IMPLIES: &[(&str, &[&str])] = &[
    ("email", &["openid"]),
    ("offline_access", &["openid"]),
    ("profile", &["openid"]),
    (
        "read:catalog",
        &["read:catalog:data", "read:catalog:search"],
    ),
    (
        "read:goals",
        &[
            "read:goals:following",
            "read:goals:private",
            "read:goals:public",
        ],
    ),
    (
        "read:journal",
        &[
            "read:journal:following",
            "read:journal:private",
            "read:journal:public",
        ],
    ),
    (
        "read:library",
        &[
            "read:library:following",
            "read:library:private",
            "read:library:public",
        ],
    ),
    (
        "read:lists",
        &[
            "read:lists:following",
            "read:lists:private",
            "read:lists:public",
        ],
    ),
    (
        "read:me",
        &["read:me:content", "read:me:email", "read:me:roles"],
    ),
    ("read:me:email", &["read:me:content"]),
    (
        "read:prompts",
        &[
            "read:prompts:following",
            "read:prompts:private",
            "read:prompts:public",
        ],
    ),
    (
        "read:vibes",
        &[
            "read:vibes:following",
            "read:vibes:private",
            "read:vibes:public",
        ],
    ),
    (
        "write:catalog",
        &[
            "write:catalog:append",
            "write:catalog:edit",
            "write:catalog:map",
            "write:catalog:split",
        ],
    ),
    ("write:catalog:edit", &["write:catalog:append"]),
];

fn implied_by(scope: &str) -> &'static [&'static str] {
    IMPLIES
        .iter()
        .find(|(s, _)| *s == scope)
        .map_or(&[], |(_, implied)| implied)
}

/// The scopes a token was granted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantedScopes {
    /// `None`: every scope (a personal access token, or `all`).
    scopes: Option<BTreeSet<String>>,
}

impl GrantedScopes {
    /// Everything. Personal access tokens carry no scope information; they act
    /// for their owner with full access.
    pub fn all() -> Self {
        Self { scopes: None }
    }

    /// A space-separated `scope` value from the token endpoint (RFC 6749 §3.3).
    pub fn parse(scope: &str) -> Self {
        let scopes: BTreeSet<String> = scope.split_whitespace().map(str::to_owned).collect();
        if scopes.contains(ALL) {
            return Self::all();
        }
        Self {
            scopes: Some(scopes),
        }
    }

    /// The scopes of a saved session: its recorded scope, or [`LEGACY_SCOPES`]
    /// when none was recorded. Sessions saved since this version always have
    /// one (see [`super::Tokens::scope`]), so `None` means an older version
    /// saved it, and that version asked for exactly those.
    pub fn for_session(scope: Option<&str>) -> Self {
        match scope {
            Some(scope) => Self::parse(scope),
            None => Self::parse(&LEGACY_SCOPES.join(" ")),
        }
    }

    /// True if `scope` was granted, directly or through a broader scope that
    /// implies it, or because every narrower scope it stands for was granted
    /// (a server may list `read:lists:public read:lists:following
    /// read:lists:private` instead of `read:lists`).
    pub fn contains(&self, scope: &str) -> bool {
        let Some(granted) = &self.scopes else {
            return true;
        };
        if granted.contains(scope) || granted.iter().any(|g| implies(g, scope)) {
            return true;
        }
        let parts = implied_by(scope);
        !parts.is_empty() && parts.iter().all(|part| self.contains(part))
    }

    /// The scopes of `required` that were not granted, in order.
    pub fn missing<'a>(&self, required: &[&'a str]) -> Vec<&'a str> {
        required
            .iter()
            .copied()
            .filter(|s| !self.contains(s))
            .collect()
    }

    /// True if every scope of `required` was granted.
    pub fn covers(&self, required: &[&str]) -> bool {
        self.missing(required).is_empty()
    }
}

/// True if `broad` implies `narrow`, directly or through another scope.
fn implies(broad: &str, narrow: &str) -> bool {
    implied_by(broad)
        .iter()
        .any(|s| *s == narrow || implies(s, narrow))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::auth::DEFAULT_SCOPES;

    fn granted(scope: &str) -> GrantedScopes {
        GrantedScopes::parse(scope)
    }

    #[test]
    fn exact_names() {
        let g = granted("read:me read:catalog read:library write:library");
        assert!(g.contains(READ_LIBRARY));
        assert!(!g.contains(READ_GOALS));
        assert!(!g.contains("read"), "no prefix matching");
        assert!(!g.contains("read:library:private:x"));
        assert_eq!(
            g.missing(&DEFAULT_SCOPES),
            [
                WRITE_REVIEWS,
                READ_GOALS,
                WRITE_GOALS,
                READ_LISTS,
                WRITE_LISTS,
                READ_JOURNAL,
                READ_SOCIAL,
                READ_USERS,
                READ_NOTIFICATIONS,
                WRITE_NOTIFICATIONS
            ]
        );
        assert!(granted(&DEFAULT_SCOPES.join(" ")).covers(&DEFAULT_SCOPES));
    }

    #[test]
    fn broad_scopes_imply_narrow_ones_but_not_the_reverse() {
        let g = granted("read:lists read:me");
        assert!(g.contains("read:lists:public"));
        assert!(g.contains("read:me:content"), "through read:me:email");
        let g = granted("read:lists:public");
        assert!(g.contains("read:lists:public"));
        assert!(!g.contains(READ_LISTS));
        assert!(!g.contains(WRITE_LISTS), "reading never implies writing");
    }

    #[test]
    fn every_narrow_part_stands_for_the_broad_scope() {
        let g = granted("read:goals:public read:goals:following read:goals:private");
        assert!(g.contains(READ_GOALS));
        assert!(!granted("read:goals:public read:goals:private").contains(READ_GOALS));
        // write:reviews has no narrower parts, so nothing stands for it.
        assert!(!granted("write:library").contains(WRITE_REVIEWS));
    }

    #[test]
    fn all_and_personal_tokens_grant_everything() {
        assert!(granted("all").covers(&DEFAULT_SCOPES));
        assert!(granted("read:me all").contains("write:anything-added-later"));
        assert!(GrantedScopes::all().covers(&DEFAULT_SCOPES));
    }

    #[test]
    fn a_session_without_a_recorded_scope_has_the_legacy_ones() {
        let g = GrantedScopes::for_session(None);
        assert!(g.covers(&LEGACY_SCOPES));
        assert!(!g.contains(READ_GOALS));
        assert!(
            GrantedScopes::for_session(Some(&DEFAULT_SCOPES.join(" "))).covers(&DEFAULT_SCOPES)
        );
        // An empty grant is not "everything".
        assert!(!GrantedScopes::for_session(Some("")).contains(READ_ME));
    }

    #[test]
    fn tolerates_extra_whitespace() {
        assert!(granted("  read:me\twrite:goals \n").covers(&[READ_ME, WRITE_GOALS]));
    }

    /// The committed scope map, as scope -> implies.
    fn committed_implies() -> BTreeMap<String, Vec<String>> {
        let caps: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../schema/hardcover-capabilities.json"
        ))
        .unwrap();
        caps["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                let mut implies: Vec<String> = s["implies"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|i| i.as_str().unwrap().to_owned())
                    .collect();
                implies.sort();
                (s["scope"].as_str().unwrap().to_owned(), implies)
            })
            .collect()
    }

    #[test]
    fn implications_match_the_committed_scope_map() {
        let map = committed_implies();
        for (scope, implies) in &map {
            if scope == ALL {
                continue;
            }
            let mut ours: Vec<String> = implied_by(scope).iter().map(|s| (*s).to_owned()).collect();
            ours.sort();
            assert_eq!(&ours, implies, "implies of {scope}");
        }
        for (scope, _) in IMPLIES {
            assert!(map.contains_key(*scope), "{scope} is not in the scope map");
        }
        // `all` implies every other scope, which `contains` assumes.
        let all = &map[ALL];
        for scope in map.keys().filter(|s| *s != ALL) {
            assert!(all.contains(scope), "all does not imply {scope}");
        }
    }

    #[test]
    fn requested_scopes_exist() {
        let map = committed_implies();
        for scope in DEFAULT_SCOPES.iter().chain(&LEGACY_SCOPES) {
            assert!(map.contains_key(*scope), "{scope} is not in the scope map");
        }
    }
}
