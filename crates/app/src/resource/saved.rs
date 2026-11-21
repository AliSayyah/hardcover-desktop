//! Resources kept in the library cache between launches: the last good
//! goals, lists, list membership index, upcoming books and tracked series,
//! in its companions table, one row per key. At launch they show at once,
//! offline too, and are fetched again as stale ([`Resource::hydrate`]).
//! Writes go through the session's cache worker, in order with the
//! library's, so sign-out's clear removes them and nothing is written after
//! it. Only values fetched or written this run are saved.

use std::hash::Hash;

use gpui_kit::{Context, Task};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Binding, Landing};
use crate::session::SessionScope;

/// A [`Resource::persist`] hook saving each value as `kind`'s row `key(k)`.
pub fn saver<K, V>(
    session: &SessionScope,
    kind: &'static str,
    key: fn(&K) -> String,
) -> impl Fn(&K, Option<&V>) + 'static
where
    K: 'static,
    V: Serialize + Clone + Send + 'static,
{
    let session = session.clone();
    move |k, value| {
        let key = key(k);
        match value {
            Some(value) => {
                let value = value.clone();
                session.cache_write(move |cache| cache.save_companion(kind, &key, &value));
            }
            None => session.cache_write(move |cache| cache.remove_companion(kind, &key)),
        }
    }
}

/// Reads `kind`'s saved rows into the owner's resource, a key at a time
/// through its `landed` hook (as [`Landing::Changed`]); never over a value
/// of this run. Dropping the task forgets what it would have shown (the
/// cache turned out to be another account's).
pub fn hydrate<T, K, V>(
    session: &SessionScope,
    kind: &'static str,
    parse: fn(&str) -> Option<K>,
    bind: Binding<T, K, V>,
    cx: &mut Context<T>,
) -> Task<()>
where
    T: 'static,
    K: Clone + Eq + Hash + 'static,
    V: PartialEq + DeserializeOwned + Send + 'static,
{
    let Some(worker) = session.cache_worker() else {
        return Task::ready(());
    };
    let read = worker.ask(move |cache| cache.load_companions::<V>(kind));
    cx.spawn(async move |this, cx| {
        let rows = match read.await {
            Some(Ok(rows)) => rows,
            Some(Err(e)) => {
                eprintln!("hardcover: could not read the saved {kind}: {e}");
                return;
            }
            None => return,
        };
        let _ = this.update(cx, |owner, cx| {
            for (key, value) in rows {
                let Some(key) = parse(&key) else {
                    continue;
                };
                if (bind.get)(owner).hydrate(key.clone(), value) {
                    (bind.landed)(owner, &key, Landing::Changed, cx);
                }
            }
        });
    })
}

/// The key of a resource with one value.
pub fn single(_: &()) -> String {
    String::new()
}

/// Parses [`single`]'s key.
pub fn parse_single(key: &str) -> Option<()> {
    key.is_empty().then_some(())
}
