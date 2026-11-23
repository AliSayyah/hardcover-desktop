//! The demo's vibes: the reader's Recommendations and Top Picks, built a
//! day ago, and a few featured vibes of invented readers, over the
//! catalogue's books. Titles and descriptions are the demo's own.
//!
//! "Never recommend…" (`update_vibe` with an exclusion) takes the book out
//! of the vibe at once. Hardcover drops it only at the vibe's next build,
//! which no API call starts; the app hides it itself until then, so the
//! demo shows what the app does either way.
//!
//! `HARDCOVER_DEMO_FAULTS=vibes=unbuilt` leaves the reader's vibes unbuilt
//! (no build time, no books), as a reader who never opened Discover on
//! hardcover.app sees them; `vibes=none` leaves the reader without system
//! vibes, so the app's `ensure_system_vibes` makes them, unbuilt.

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::state::State;

/// `vibe_type`s.
const RECOMMENDATIONS: i64 = 1;
const TOP_PICKS: i64 = 3;
/// Hardcover's "private" privacy setting, which the owner's system vibes had.
const PRIVATE: i64 = 3;
const PUBLIC: i64 = 1;

/// The catalogue's `vibes`.
#[derive(Deserialize, Default)]
pub(super) struct SeedVibes {
    recommendations: Option<SeedSystemVibe>,
    top_picks: Option<SeedSystemVibe>,
    #[serde(default)]
    featured: Vec<SeedFeatured>,
}

#[derive(Deserialize)]
struct SeedSystemVibe {
    id: i64,
    built_days_ago: i64,
    books: Vec<i64>,
}

#[derive(Deserialize)]
struct SeedFeatured {
    id: i64,
    title: String,
    description: String,
    owner: SeedOwner,
    likes: i64,
    built_days_ago: i64,
    featured_days_ago: i64,
    books: Vec<i64>,
}

#[derive(Deserialize)]
struct SeedOwner {
    id: i64,
    username: String,
}

/// How the reader's own vibes start (`vibes=` in `HARDCOVER_DEMO_FAULTS`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SystemVibes {
    /// Built a day ago.
    #[default]
    Built,
    /// There, but never built.
    Unbuilt,
    /// Not made yet: `ensure_system_vibes` makes them, unbuilt.
    Missing,
}

/// A `vibes` row.
pub(super) struct Vibe {
    pub(super) id: i64,
    pub(super) kind: i64,
    pub(super) title: String,
    pub(super) description: Option<String>,
    pub(super) slug: String,
    pub(super) built: Option<DateTime<Utc>>,
    pub(super) books: Vec<i64>,
    pub(super) user_id: i64,
    pub(super) username: String,
    pub(super) featured_at: Option<DateTime<Utc>>,
    pub(super) likes: i64,
    pub(super) privacy: i64,
}

impl Vibe {
    fn system(id: i64, kind: i64, user: &super::DemoUser) -> Self {
        let (title, description, slug) = if kind == RECOMMENDATIONS {
            (
                "Recommendations",
                "Books picked for you based on your reading history.",
                "recommendations",
            )
        } else {
            (
                "Top Picks",
                "The most popular books we think you'll love.",
                "top-picks",
            )
        };
        Self {
            id,
            kind,
            title: title.to_owned(),
            description: Some(description.to_owned()),
            slug: slug.to_owned(),
            built: None,
            books: Vec::new(),
            user_id: user.id,
            username: user.username.clone(),
            featured_at: None,
            likes: 0,
            privacy: PRIVATE,
        }
    }

    /// `VibeFields`. Hardcover's `books_generated_at` is a `timestamp`,
    /// UTC with no offset.
    pub(super) fn json(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "description": self.description,
            "slug": self.slug,
            "vibe_type": self.kind,
            "books_generated_at": self.built.map(|t| t.naive_utc().format("%Y-%m-%dT%H:%M:%S%.6f").to_string()),
            "cached_book_ids": self.books,
            "featured": self.featured_at.is_some(),
            "privacy_setting_id": self.privacy,
            "user_id": self.user_id,
            "likes_count": self.likes,
            "user": { "username": self.username },
        })
    }
}

/// The vibes a client starts with, dated from `now`.
pub(super) fn seed(
    seeds: &SeedVibes,
    user: &super::DemoUser,
    now: DateTime<Utc>,
    system: SystemVibes,
) -> Vec<Vibe> {
    let ago = |days: i64| now - Duration::days(days);
    let mut vibes = Vec::new();
    if system != SystemVibes::Missing {
        for (seed, kind) in [
            (&seeds.recommendations, RECOMMENDATIONS),
            (&seeds.top_picks, TOP_PICKS),
        ] {
            let Some(seed) = seed else { continue };
            let mut vibe = Vibe::system(seed.id, kind, user);
            if system == SystemVibes::Built {
                vibe.built = Some(ago(seed.built_days_ago));
                vibe.books.clone_from(&seed.books);
            }
            vibes.push(vibe);
        }
    }
    vibes.extend(seeds.featured.iter().map(|f| Vibe {
        id: f.id,
        kind: 0,
        title: f.title.clone(),
        description: Some(f.description.clone()),
        slug: super::slug(&f.title),
        built: Some(ago(f.built_days_ago)),
        books: f.books.clone(),
        user_id: f.owner.id,
        username: f.owner.username.clone(),
        featured_at: Some(ago(f.featured_days_ago)),
        likes: f.likes,
        privacy: PUBLIC,
    }));
    vibes
}

// ----- reads -----

/// `SystemVibes`: the user's vibes of type 1 and 3, by type, then id.
pub(super) fn system_vibes(state: &State, user: i64) -> Value {
    let mut vibes: Vec<&Vibe> = state
        .vibes
        .iter()
        .filter(|v| v.user_id == user && [RECOMMENDATIONS, TOP_PICKS].contains(&v.kind))
        .collect();
    vibes.sort_by_key(|v| (v.kind, v.id));
    json!({ "vibes": vibes.into_iter().map(Vibe::json).collect::<Vec<_>>() })
}

/// `FeaturedVibes`: the featured ones, the most recently featured first.
pub(super) fn featured_vibes(state: &State, limit: usize) -> Value {
    let mut vibes: Vec<&Vibe> = state
        .vibes
        .iter()
        .filter(|v| v.featured_at.is_some())
        .collect();
    vibes.sort_by(|a, b| b.featured_at.cmp(&a.featured_at).then(b.id.cmp(&a.id)));
    vibes.truncate(limit);
    json!({ "vibes": vibes.into_iter().map(Vibe::json).collect::<Vec<_>>() })
}

/// `VibeById`.
pub(super) fn vibe_by_id(state: &State, id: i64) -> Value {
    json!({ "vibes_by_pk": state.vibes.iter().find(|v| v.id == id).map(Vibe::json) })
}

// ----- writes -----

/// `EnsureSystemVibes`: the reader's system vibe ids, making those missing
/// (unbuilt, as Hardcover makes them).
pub(super) fn ensure_system_vibes(state: &mut State) -> Value {
    let user = &super::CATALOGUE.user;
    for kind in [RECOMMENDATIONS, TOP_PICKS] {
        let exists = state
            .vibes
            .iter()
            .any(|v| v.user_id == user.id && v.kind == kind);
        if !exists {
            let id = state.new_id();
            state.vibes.push(Vibe::system(id, kind, user));
        }
    }
    let mut ids: Vec<(i64, i64)> = state
        .vibes
        .iter()
        .filter(|v| v.user_id == user.id && [RECOMMENDATIONS, TOP_PICKS].contains(&v.kind))
        .map(|v| (v.kind, v.id))
        .collect();
    ids.sort_unstable();
    json!({ "ensure_system_vibes": { "ids": ids.into_iter().map(|(_, id)| id).collect::<Vec<_>>() } })
}

/// `ExcludeFromVibe` (`update_vibe`): sets the title, description and
/// privacy sent, and takes each excluded book out of the vibe's books.
/// Only the reader's own vibes can be changed; a field the demo doesn't
/// write is refused.
pub(super) fn update_vibe(state: &mut State, id: i64, object: &Value) -> Result<Value, String> {
    let object = super::writes::object(object)?;
    super::writes::only(
        object,
        &[
            "title",
            "description",
            "privacy_setting_id",
            "vibe_exclusions_attributes",
        ],
    )?;
    let me = super::CATALOGUE.user.id;
    let Some(vibe) = state.vibes.iter_mut().find(|v| v.id == id) else {
        return Ok(payload(None, &["Vibe not found"]));
    };
    if vibe.user_id != me {
        return Ok(payload(None, &["Not allowed"]));
    }
    let excluded = exclusions(object.get("vibe_exclusions_attributes"))?;
    for (key, value) in object {
        match key.as_str() {
            "title" => match value.as_str().map(str::trim).filter(|t| !t.is_empty()) {
                Some(title) => vibe.title = title.to_owned(),
                None => return Ok(payload(None, &["Title can't be blank"])),
            },
            "description" => vibe.description = value.as_str().map(str::to_owned),
            "privacy_setting_id" => {
                vibe.privacy = value.as_i64().ok_or("privacy_setting_id is not an id")?;
            }
            _ => {}
        }
    }
    vibe.books.retain(|b| !excluded.contains(b));
    Ok(payload(Some(&*vibe), &[]))
}

/// The books of `vibe_exclusions_attributes`: each `{excluded_type: "Book",
/// excluded_id}`; anything else is refused.
fn exclusions(value: Option<&Value>) -> Result<Vec<i64>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or("vibe_exclusions_attributes is not a list")?;
    items
        .iter()
        .map(|item| {
            let item: &Map<String, Value> = super::writes::object(item)?;
            super::writes::only(item, &["excluded_type", "excluded_id"])?;
            if item.get("excluded_type").and_then(Value::as_str) != Some("Book") {
                return Err("The demo only excludes books.".to_owned());
            }
            item.get("excluded_id")
                .and_then(Value::as_i64)
                .ok_or_else(|| "excluded_id is not an id".to_owned())
        })
        .collect()
}

fn payload(vibe: Option<&Vibe>, errors: &[&str]) -> Value {
    json!({ "update_vibe": {
        "id": vibe.map(|v| v.id),
        "errors": (!errors.is_empty()).then_some(errors),
        "vibe": vibe.map(Vibe::json),
    }})
}
