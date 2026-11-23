//! The session's vibes, as an entity: the user's system vibes
//! (Recommendations, shown as "For you", and Top Picks), the vibes Hardcover
//! features, and any other vibe opened by id. Each is a [`Resource`]; the
//! system and featured vibes are kept in the library cache
//! (`resource::saved`), so they show at once at the next launch, and are
//! fetched again when stale. Everything needs `read:vibes`.
//!
//! A user without system vibes gets them made once a session
//! (`ensure_system_vibes`, `write:vibes`), after a read that found none;
//! they stay empty until Hardcover builds them.
//!
//! "Never recommend…" ([`VibesModel::exclude`]) writes the exclusion and
//! records it in the preferences, which hide the book from every vibe built
//! before it until Hardcover builds the vibe again; each fetch of the
//! system vibes forgets the exclusions Hardcover has caught up with.

use std::collections::HashSet;

use gpui_kit::{App, AppContext as _, Context, Entity, Global, SharedString, Task, WeakEntity};
use hardcover_core::auth::scopes::{READ_VIBES, WRITE_VIBES};
use hardcover_core::prefs::{VibeExclusion, shown_books};
use hardcover_core::{BookId, Error, Vibe, VibeId, VibeKind};

use crate::format::needs_permission;
use crate::io::Priority;
use crate::library::model::{AccountChanged, LibraryModel};
use crate::preferences::AppPreferences;
use crate::resource::{Binding, Demand, Landing, LoadStatus, Resource, saved};
use crate::session::SessionScope;

/// The system vibes' row in the library cache.
const SYSTEM_SAVED_AS: &str = "system_vibes";
/// The featured vibes' row.
const FEATURED_SAVED_AS: &str = "featured_vibes";

pub struct VibesModel {
    session: SessionScope,
    system: Resource<(), Vec<Vibe>>,
    featured: Resource<(), Vec<Vibe>>,
    /// Vibes opened that are neither the user's nor featured.
    others: Resource<VibeId, Vibe>,
    /// `ensure_system_vibes` was sent this session.
    ensured: bool,
    /// Exclusions being written, by vibe and book.
    excluding: HashSet<(VibeId, BookId)>,
    _hydrating: Vec<Task<()>>,
    _subscriptions: Vec<gpui_kit::Subscription>,
}

struct VibesGlobal(WeakEntity<VibesModel>);

impl Global for VibesGlobal {}

impl VibesModel {
    /// Creates the model for a new workspace, replacing any earlier one,
    /// and shows the vibes of the last launch while they load. The caller
    /// owns it.
    pub fn init(
        session: SessionScope,
        library: &Entity<LibraryModel>,
        cx: &mut App,
    ) -> Entity<Self> {
        let model = cx.new(|cx| {
            let system = Resource::new(session.clone(), READ_VIBES, Priority::Background)
                .persist(saved::saver(&session, SYSTEM_SAVED_AS, saved::single));
            let featured = Resource::new(session.clone(), READ_VIBES, Priority::Background)
                .persist(saved::saver(&session, FEATURED_SAVED_AS, saved::single));
            let hydrating = vec![
                saved::hydrate(
                    &session,
                    SYSTEM_SAVED_AS,
                    saved::parse_single,
                    Self::bind_saved_system(),
                    cx,
                ),
                saved::hydrate(
                    &session,
                    FEATURED_SAVED_AS,
                    saved::parse_single,
                    Self::bind_featured(),
                    cx,
                ),
            ];
            let subscriptions = vec![cx.subscribe(library, |this, _, _: &AccountChanged, cx| {
                this._hydrating.clear();
                this.system.forget_from_disk();
                this.featured.forget_from_disk();
                cx.notify();
            })];
            Self {
                others: Resource::new(session.clone(), READ_VIBES, Priority::Interactive),
                session,
                system,
                featured,
                ensured: false,
                excluding: HashSet::new(),
                _hydrating: hydrating,
                _subscriptions: subscriptions,
            }
        });
        cx.set_global(VibesGlobal(model.downgrade()));
        model
    }

    /// Forgets the workspace's model, on signing out.
    pub fn release(cx: &mut App) {
        if cx.has_global::<VibesGlobal>() {
            cx.remove_global::<VibesGlobal>();
        }
    }

    /// The workspace's model, once signed in.
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<VibesGlobal>()?.0.upgrade()
    }

    fn bind_system() -> Binding<Self, (), Vec<Vibe>> {
        Binding {
            get: |m| &mut m.system,
            landed: |this, _, landing, cx| {
                if matches!(landing, Landing::Changed | Landing::Same) {
                    this.system_fetched(cx);
                }
                cx.notify();
            },
        }
    }

    /// The system vibes of the last launch: shown, but they decide nothing
    /// (they may be days old).
    fn bind_saved_system() -> Binding<Self, (), Vec<Vibe>> {
        Binding {
            get: |m| &mut m.system,
            landed: |_, _, _, cx| cx.notify(),
        }
    }

    fn bind_featured() -> Binding<Self, (), Vec<Vibe>> {
        Binding {
            get: |m| &mut m.featured,
            landed: |_, _, _, cx| cx.notify(),
        }
    }

    fn bind_others() -> Binding<Self, VibeId, Vibe> {
        Binding {
            get: |m| &mut m.others,
            landed: |_, _, _, cx| cx.notify(),
        }
    }

    // ----- reads -----

    /// The user's system vibe of `kind`, once loaded.
    pub fn system_vibe(&self, kind: VibeKind) -> Option<&Vibe> {
        self.system
            .value(&())
            .and_then(|vibes| vibes.iter().find(|v| v.kind == kind))
    }

    /// Whether the system vibes have been read (this run or the last), so
    /// a missing one is missing, not loading.
    pub fn system_loaded(&self) -> bool {
        self.system.value(&()).is_some()
    }

    pub fn system_status(&self) -> LoadStatus {
        self.system.status(&())
    }

    /// The vibes Hardcover features, once read (this run or the last):
    /// Discover's "Featured on Hardcover".
    pub fn featured_vibes(&self) -> Option<&[Vibe]> {
        self.featured.value(&()).map(|v| v.as_slice())
    }

    /// Vibe `id`, wherever it was loaded.
    pub fn vibe(&self, id: VibeId) -> Option<&Vibe> {
        let in_list = |list: Option<&std::sync::Arc<Vec<Vibe>>>| -> Option<usize> {
            list?.iter().position(|v| v.id == id)
        };
        if let Some(ix) = in_list(self.system.value(&())) {
            return self.system.value(&()).map(|v| &v[ix]);
        }
        if let Some(ix) = in_list(self.featured.value(&())) {
            return self.featured.value(&()).map(|v| &v[ix]);
        }
        self.others.value(&id).map(|v| &**v)
    }

    /// Where loading vibe `id` stands: as one of the system vibes, or on
    /// its own.
    pub fn vibe_status(&self, id: VibeId) -> LoadStatus {
        if self.vibe(id).is_some() {
            return LoadStatus::Loaded;
        }
        match self.others.status(&id) {
            LoadStatus::Idle => self.system_status(),
            status => status,
        }
    }

    /// The ids of `vibe`'s books to show: less the books the user left
    /// out that Hardcover may still list.
    pub fn shown_books(vibe: &Vibe, cx: &App) -> Vec<BookId> {
        shown_books(vibe, AppPreferences::global(cx).read(cx).vibe_exclusions())
    }

    /// Whether an exclusion of `book` from `vibe` is being written.
    pub fn is_excluding(&self, vibe: VibeId, book: BookId) -> bool {
        self.excluding.contains(&(vibe, book))
    }

    // ----- loading -----

    /// Asks for the user's system vibes and the featured ones where they
    /// are stale or failed (launch, Refresh, a new grant).
    pub fn load(&mut self, cx: &mut Context<Self>) {
        self.request_system(Demand::Shown, cx);
        self.request_featured(Demand::Shown, cx);
    }

    /// Asks for the system vibes as `demand` calls for.
    pub fn request_system(&mut self, demand: Demand, cx: &mut Context<Self>) {
        if self
            .system
            .request((), demand, |c| c.system_vibes(), Self::bind_system(), cx)
        {
            cx.notify();
        }
    }

    /// Asks for the featured vibes as `demand` calls for.
    pub fn request_featured(&mut self, demand: Demand, cx: &mut Context<Self>) {
        if self.featured.request(
            (),
            demand,
            |c| c.featured_vibes(),
            Self::bind_featured(),
            cx,
        ) {
            cx.notify();
        }
    }

    /// Asks for vibe `id` as `demand` calls for: the system vibes if it is
    /// one of them, else the vibe on its own (a featured one is fetched
    /// with the featured list).
    pub fn request_vibe(&mut self, id: VibeId, demand: Demand, cx: &mut Context<Self>) {
        match self.vibe(id) {
            Some(v) if v.kind.is_system() => self.request_system(demand, cx),
            Some(v) if v.featured && self.others.value(&id).is_none() => {
                self.request_featured(demand, cx);
            }
            _ => {
                let sent = self.others.request(
                    id,
                    demand,
                    move |c| {
                        c.vibe(id)?
                            .ok_or_else(|| Error::Api("Hardcover has no such vibe.".into()))
                    },
                    Self::bind_others(),
                    cx,
                );
                if sent {
                    cx.notify();
                }
            }
        }
    }

    /// The system vibes were fetched: forget the exclusions Hardcover has
    /// caught up with, and make the vibes if the user has none.
    fn system_fetched(&mut self, cx: &mut Context<Self>) {
        let vibes = self.system.value(&()).cloned().unwrap_or_default();
        if !vibes.is_empty() {
            AppPreferences::global(cx)
                .clone()
                .update(cx, |p, cx| p.settle_vibe_exclusions(&vibes, cx));
            return;
        }
        if self.ensured || !self.session.allows(WRITE_VIBES) {
            return;
        }
        self.ensured = true;
        let ensure = self.session.background(|c| c.ensure_system_vibes());
        cx.spawn(async move |this, cx| match ensure.await {
            Some(Ok(ids)) if !ids.is_empty() => {
                let _ = this.update(cx, |this, cx| this.request_system(Demand::Now, cx));
            }
            Some(Err(e)) => eprintln!("hardcover: could not make the system vibes: {e}"),
            _ => {}
        })
        .detach();
    }

    // ----- writes -----

    /// "Never recommend…": leaves `book` out of `vibe`. Once it lands the
    /// book is hidden from every vibe built before now (the preferences
    /// keep it), so its row leaves; `None` when dropped because the session
    /// ended, else the result or the message to show.
    pub fn exclude(
        &mut self,
        vibe: VibeId,
        book: BookId,
        cx: &mut Context<Self>,
    ) -> Task<Option<Result<(), SharedString>>> {
        if !self.session.is_alive() {
            return Task::ready(None);
        }
        if !self.session.has_scope(WRITE_VIBES) {
            return Task::ready(Some(Err(needs_permission(WRITE_VIBES).into())));
        }
        if !self.excluding.insert((vibe, book)) {
            return Task::ready(Some(Err(crate::library::model::STILL_SAVING.into())));
        }
        cx.notify();
        let call = self
            .session
            .interactive(move |c| c.exclude_from_vibe(vibe, book));
        cx.spawn(async move |this, cx| {
            let result = call.await;
            this.update(cx, |this, cx| {
                this.excluding.remove(&(vibe, book));
                cx.notify();
                Some(match result? {
                    Ok(_) => {
                        let exclusion = VibeExclusion {
                            book,
                            vibe,
                            at: chrono::Utc::now(),
                        };
                        AppPreferences::global(cx)
                            .clone()
                            .update(cx, |p, cx| p.add_vibe_exclusion(exclusion, cx));
                        Ok(())
                    }
                    Err(e) => Err(this.session.error_message(&e, Some(WRITE_VIBES), cx).into()),
                })
            })
            .ok()
            .flatten()
        })
    }
}
