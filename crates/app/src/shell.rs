//! The window's content: the sign-in screen or the workspace (sidebar plus the
//! current destination).

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariant};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{
    Disableable as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    Action, Anchor, AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement as _, IntoElement, Modifiers, ModifiersChangedEvent,
    ParentElement as _, Render, ScrollHandle, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::auth::scopes::READ_CATALOG;
use hardcover_core::store::LibraryCache;
use hardcover_core::{ListId, ReadingStatus};

use crate::about::open_about;
use crate::app_state::{CoverService, Services, Startup, startup};
use crate::book::detail_view::{BookDetailView, Subject};
use crate::catalog::author_view::AuthorView;
use crate::catalog::discover_view::DiscoverView;
use crate::catalog::navigation::{NavigationEvent, Navigator, Page, PageStack};
use crate::catalog::series_view::SeriesView;
use crate::design::type_roles::Family;
use crate::design::{TypeRoles as _, colors, size};
use crate::format::{SIDEBAR_SHELVES, count_text, relative_time, status_label};
use crate::freshness;
use crate::goals::model::GoalsModel;
use crate::goals::sidebar::GoalsSection;
use crate::home::home_view::HomeView;
use crate::library::library_view::LibraryView;
use crate::library::model::{LibraryModel, SyncStatus};
use crate::lists::list_view::ListView;
use crate::lists::model::{ListsEvent, ListsModel};
use crate::lists::sidebar::lists_section;
use crate::search::search_view::SearchView;
use crate::series::model::SeriesModel;
use crate::series::tracker_view::SeriesTrackerView;
use crate::settings_view::{SettingsView, SignOutRequested};
use crate::sign_in::reauthorize::{Reauthorize, Reauthorized};
use crate::sign_in::sign_in_view::{SignInView, SignedIn};
use crate::stats::stats_view::StatsView;
use crate::ui::focus::{remember_focus, return_focus};
use crate::ui::kit::SmallControl as _;
use crate::ui::kit::{
    gutter_scrollbar, open_alert_dialog, open_alert_dialog_on_result, open_dialog,
};
use crate::ui::reveal::RevealScroll as _;
use crate::ui::{
    AccountRow, Avatar, BackTo, ExternalLink, NavRow, ObjectBar, back_label, brand_row,
    sidebar_label, tools_of,
};
use crate::upcoming::model::UpcomingModel;
use crate::upcoming::upcoming_view::UpcomingView;
use crate::update::{UpdateEvent, UpdateModel, UpdateState, Updates};
use crate::{
    FocusSearch, GoBack, OpenSettings, Refresh, ShowAbout, ShowCurrentlyReading, ShowDidNotFinish,
    ShowDiscover, ShowHome, ShowIgnored, ShowPaused, ShowRead, ShowSeries, ShowStats,
    ShowWantToRead,
};

/// Key context of a catalogue page, where Escape goes back.
pub const PAGE_CONTEXT: &str = "CatalogPage";

/// How long Ctrl must be held before the sidebar shows its shortcuts, so a
/// quick Ctrl+1 doesn't flash them.
const KEY_HINT_DELAY: Duration = Duration::from_millis(500);

// ----- top level -----

enum Screen {
    Starting,
    SignIn(Entity<SignInView>),
    Workspace(Entity<Workspace>),
}

/// Owns the screen for the current sign-in state.
pub struct Shell {
    screen: Screen,
    cache: Option<Arc<LibraryCache>>,
    subscription: Option<Subscription>,
}

impl Shell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.spawn_in(window, async move |this, cx| {
            let (startup, covers) = cx
                .background_spawn(async move { (startup(), CoverService::open()) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                cx.set_global(covers);
                match startup {
                    Startup::SignedIn {
                        client,
                        session,
                        cache,
                        scopes,
                    } => {
                        this.cache.clone_from(&cache);
                        let services = Services::new(client, session, cache, scopes);
                        this.enter_workspace(services, window, cx);
                    }
                    Startup::SignedOut { cache } => {
                        this.cache = cache;
                        this.show_sign_in(false, window, cx);
                    }
                    Startup::KeyringUnavailable { cache } => {
                        this.cache = cache;
                        this.show_sign_in(true, window, cx);
                    }
                }
            });
        })
        .detach();
        Self {
            screen: Screen::Starting,
            cache: None,
            subscription: None,
        }
    }

    fn enter_workspace(&mut self, services: Services, window: &mut Window, cx: &mut Context<Self>) {
        cx.set_global(services);
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        self.subscription = Some(cx.subscribe_in(
            &workspace,
            window,
            |this, _, _: &SignedOut, window, cx| this.show_sign_in(false, window, cx),
        ));
        workspace.update(cx, |w, cx| w.focus_initial(window, cx));
        self.screen = Screen::Workspace(workspace);
        cx.notify();
    }

    /// The sign-in screen; with `keyring_unavailable`, the keyring couldn't
    /// be read at startup and the screen offers to try again.
    fn show_sign_in(
        &mut self,
        keyring_unavailable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cx.has_global::<Services>() {
            cx.remove_global::<Services>();
        }
        ListsModel::release(cx);
        let sign_in = cx.new(|cx| {
            if keyring_unavailable {
                SignInView::keyring_unavailable(window, cx)
            } else {
                SignInView::new(window, cx)
            }
        });
        self.subscription =
            Some(
                cx.subscribe_in(&sign_in, window, |this, _, event: &SignedIn, window, cx| {
                    let services = Services::new(
                        event.client.clone(),
                        Some(event.session.clone()),
                        this.cache.clone(),
                        event.scopes.clone(),
                    );
                    this.enter_workspace(services, window, cx);
                }),
            );
        sign_in.update(cx, |view, cx| view.focus(window, cx));
        self.screen = Screen::SignIn(sign_in);
        cx.notify();
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            // Inter over Vazirmatn for everything inside; dialogs and
            // popovers are outside, see `bidi::WrappingText`.
            .type_family(Family::Sans)
            .size_full()
            .bg(colors(cx).bg)
            .text_color(colors(cx).fg)
            .map(|this| match &self.screen {
                Screen::Starting => this,
                Screen::SignIn(view) => this.child(view.clone()),
                Screen::Workspace(view) => this.child(view.clone()),
            })
    }
}

// ----- workspace -----

/// Where the workspace's main area points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    /// Shown at launch.
    Home,
    Search,
    Discover,
    /// Every series the reader has started.
    Series,
    Stats,
    Shelf(ReadingStatus),
    List(ListId),
}

/// Emitted after signing out.
pub struct SignedOut;

pub struct Workspace {
    focus_handle: FocusHandle,
    model: Entity<LibraryModel>,
    destination: Destination,
    home: Entity<HomeView>,
    library: Entity<LibraryView>,
    search: Entity<SearchView>,
    discover: Entity<DiscoverView>,
    stats: Entity<StatsView>,
    /// Catalogue pages opened from the destination; the top one is shown.
    pages: PageStack,
    series_page: Entity<SeriesView>,
    author_page: Entity<AuthorView>,
    book_page: Entity<BookDetailView>,
    page_focus: FocusHandle,
    /// Asks a session that lacks scopes to sign in again.
    reauthorize: Entity<Reauthorize>,
    goals: Entity<GoalsModel>,
    goals_section: Entity<GoalsSection>,
    /// The sidebar's destinations, which scroll between the brand and the
    /// account; a row that gains focus scrolls into view in it.
    sidebar_scroll: ScrollHandle,
    /// The account row's focus: where the account menu, and the dialogs it
    /// opens, give focus back.
    account_focus: FocusHandle,
    lists: Entity<ListsModel>,
    list_page: Entity<ListView>,
    upcoming: Entity<UpcomingModel>,
    upcoming_page: Entity<UpcomingView>,
    series: Entity<SeriesModel>,
    series_view: Entity<SeriesTrackerView>,
    signing_out: bool,
    /// Whether the sidebar shows its keyboard shortcuts: only while Ctrl is
    /// held, so the rest of the time the rows stay clean.
    key_hints: bool,
    /// Shows the hints once Ctrl has been held for [`KEY_HINT_DELAY`].
    key_hint_delay: Option<Task<()>>,
    /// The open Settings dialog's sign-out request.
    settings_subscription: Option<Subscription>,
    /// Re-renders periodically so "Updated 5 min ago" stays true.
    _clock: Task<()>,
    subscriptions: Vec<Subscription>,
}

impl EventEmitter<SignedOut> for Workspace {}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Workspace {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let model = cx.new(|_| LibraryModel::new());
        // Before the views: the book page finds the lists through the global.
        let lists = ListsModel::init(cx);
        let list_page = cx.new(|cx| ListView::new(lists.clone(), model.clone(), window, cx));
        let library = cx.new(|cx| LibraryView::new(model.clone(), window, cx));
        let search = cx.new(|cx| SearchView::new(&model, window, cx));
        let discover = cx.new(|cx| DiscoverView::new(&model, window, cx));
        let stats = cx.new(|cx| StatsView::new(model.clone(), cx));
        let series_page = cx.new(|cx| SeriesView::new(model.clone(), window, cx));
        let author_page = cx.new(|cx| AuthorView::new(model.clone(), window, cx));
        let book_page = cx.new(|cx| {
            let mut page = BookDetailView::new(model.clone(), window, cx);
            page.set_tools_in_bar(true);
            page
        });
        let navigator = Navigator::init(cx);
        let reauthorize = cx.new(|_| Reauthorize::new(model.clone()));
        let goals = cx.new(|cx| GoalsModel::new(&model, cx));
        let sidebar_scroll = ScrollHandle::new();
        let goals_section = cx.new(|cx| GoalsSection::new(goals.clone(), model.clone(), cx));
        let upcoming = cx.new(|cx| UpcomingModel::new(model.clone(), cx));
        let upcoming_page = cx.new(|cx| UpcomingView::new(upcoming.clone(), model.clone(), cx));
        let home = cx.new(|cx| HomeView::new(model.clone(), goals.clone(), upcoming.clone(), cx));
        let series = cx.new(|cx| SeriesModel::new(model.clone(), upcoming.clone(), cx));
        let series_view = cx
            .new(|cx| SeriesTrackerView::new(series.clone(), model.clone(), &upcoming, window, cx));
        model.update(cx, LibraryModel::start);
        goals.update(cx, GoalsModel::load);
        lists.update(cx, ListsModel::load);
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(30))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        let mut workspace = Self {
            focus_handle: cx.focus_handle(),
            model,
            destination: Destination::Home,
            home,
            library,
            search,
            discover,
            stats,
            pages: PageStack::default(),
            series_page,
            author_page,
            book_page,
            page_focus: cx.focus_handle(),
            reauthorize,
            goals,
            goals_section,
            sidebar_scroll,
            account_focus: cx.focus_handle().tab_stop(true),
            lists,
            list_page,
            upcoming,
            upcoming_page,
            series,
            series_view,
            signing_out: false,
            key_hints: false,
            key_hint_delay: None,
            settings_subscription: None,
            _clock: clock,
            subscriptions: Vec::new(),
        };
        workspace.subscribe(&navigator, window, cx);
        workspace
    }

    fn subscribe(
        &mut self,
        navigator: &Entity<Navigator>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let updates = Updates::global(cx).clone();
        self.subscriptions = vec![
            cx.observe(&self.model, |_, _, cx| cx.notify()),
            // Coming back to the window catches up with changes made on the
            // website or another device, once the last sync is old enough.
            cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.model.update(cx, LibraryModel::sync_if_stale);
                    this.revalidate_visible(false, window, cx);
                }
            }),
            cx.observe(&updates, |_, _, cx| cx.notify()),
            cx.observe(&self.reauthorize, |_, _, cx| cx.notify()),
            cx.observe(&self.goals, |_, _, cx| cx.notify()),
            // The sidebar counts the series not finished.
            cx.observe(&self.series, |_, _, cx| cx.notify()),
            // A list shown that is gone after a reload, or no longer
            // readable, hands over to a shelf.
            cx.observe_in(&self.lists, window, |this, lists, window, cx| {
                if let Destination::List(id) = this.destination
                    && lists.read(cx).is_gone(id)
                {
                    this.show_shelf(ReadingStatus::CurrentlyReading, window, cx);
                }
                cx.notify();
            }),
            cx.subscribe_in(
                &self.lists,
                window,
                |this, _, event: &ListsEvent, window, cx| {
                    if let ListsEvent::Deleted(id) = event
                        && this.destination == Destination::List(*id)
                    {
                        this.show_shelf(ReadingStatus::CurrentlyReading, window, cx);
                    }
                },
            ),
            cx.subscribe(&self.reauthorize, |this, _, event: &Reauthorized, cx| {
                this.on_reauthorized(event, cx);
            }),
            cx.subscribe_in(&updates, window, |_, _, event, window, cx| {
                Self::on_update_event(event, window, cx);
            }),
            // A dialog or menu closing can drop focus (its trigger may be gone,
            // e.g. after removing a book). Keyboard shortcuts need a focused
            // element, so return to the current destination.
            cx.on_focus_lost(window, |this, window, cx| {
                this.focus_destination(window, cx);
            }),
            cx.subscribe_in(
                navigator,
                window,
                |this, _, event: &NavigationEvent, window, cx| match event {
                    NavigationEvent::Open(page) => this.open_page(page.clone(), window, cx),
                },
            ),
        ];
    }

    /// At launch focus sits on the workspace itself, ahead of the sidebar, so
    /// Tab walks the window in its visual order: the sidebar, then Home.
    /// Home binds no keys of its own, so nothing waits on its focus.
    fn focus_initial(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
    }

    /// Focuses the current destination's primary control.
    fn focus_destination(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pages.current().is_some() {
            self.focus_page(window, cx);
            return;
        }
        match self.destination {
            Destination::Home => self.home.update(cx, |h, cx| h.focus(window, cx)),
            Destination::Search => self.search.update(cx, |s, cx| s.focus_input(window, cx)),
            Destination::Discover => self.discover.update(cx, |d, cx| d.focus(window, cx)),
            Destination::Series => self.series_view.update(cx, |s, cx| s.focus(window, cx)),
            Destination::Stats => self.stats.update(cx, |s, cx| s.focus(window, cx)),
            Destination::Shelf(_) => self.library.update(cx, |l, cx| l.focus_list(window, cx)),
            Destination::List(_) => self.list_page.update(cx, |l, cx| l.focus(window, cx)),
        }
    }

    fn show(&mut self, destination: Destination, window: &mut Window, cx: &mut Context<Self>) {
        // A sidebar destination starts a new history.
        self.pages.clear();
        self.destination = destination;
        match destination {
            Destination::Home => self.home.update(cx, |h, cx| {
                h.opened(cx);
                h.focus(window, cx);
            }),
            Destination::Search => self.search.update(cx, |s, cx| s.focus_input(window, cx)),
            Destination::Discover => self.discover.update(cx, |d, cx| {
                d.show(window, cx);
                d.focus(window, cx);
            }),
            Destination::Series => self.series_view.update(cx, |s, cx| s.focus(window, cx)),
            Destination::Stats => self.stats.update(cx, |s, cx| s.focus(window, cx)),
            Destination::Shelf(status) => {
                self.library
                    .update(cx, |l, cx| l.show_shelf(status, window, cx));
                self.library.update(cx, |l, cx| l.focus_list(window, cx));
            }
            Destination::List(id) => {
                self.list_page.update(cx, |l, cx| {
                    l.show(id, window, cx);
                    l.focus(window, cx);
                });
            }
        }
        self.revalidate_visible(true, window, cx);
        cx.notify();
    }

    /// Asks the views on screen to fetch again what they show that is stale
    /// ([`crate::freshness`]): the top page (and its book pane), else the
    /// destination's catalogue content. Views can't tell whether they are
    /// shown, so only these are asked, on Refresh, when the window becomes
    /// active, and when a page or destination is shown again
    /// (`shown_again`).
    fn revalidate_visible(
        &mut self,
        shown_again: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.pages.current() {
            Some(Page::Series(_)) => self
                .series_page
                .update(cx, |p, cx| p.revalidate(shown_again, window, cx)),
            Some(Page::Author(_)) => self
                .author_page
                .update(cx, |p, cx| p.revalidate(shown_again, window, cx)),
            Some(Page::Book(_)) => self
                .book_page
                .update(cx, |p, cx| p.revalidate(shown_again, cx)),
            // Follows the library and the upcoming books.
            Some(Page::Upcoming) => {}
            None => match self.destination {
                Destination::Search => self
                    .search
                    .update(cx, |s, cx| s.revalidate(shown_again, cx)),
                Destination::Discover => self
                    .discover
                    .update(cx, |d, cx| d.revalidate(shown_again, window, cx)),
                Destination::Shelf(_) => self
                    .library
                    .update(cx, |l, cx| l.revalidate(shown_again, cx)),
                Destination::List(_) => self
                    .list_page
                    .update(cx, |l, cx| l.revalidate(shown_again, cx)),
                // Worked out from the library and models that follow it.
                Destination::Home | Destination::Series | Destination::Stats => {}
            },
        }
    }

    // ----- catalogue pages -----

    fn open_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        let from = remember_focus(window, cx);
        if self.pages.push(page, from) {
            self.show_page(window, cx);
        }
    }

    /// Back to the previous page, or to the destination after the first one,
    /// with focus on the control that opened the page left; on the page's or
    /// the destination's own focus if that control is gone.
    fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(opener) = self.pages.pop() else {
            return;
        };
        if self.pages.current().is_some() {
            self.show_page(window, cx);
        } else {
            self.revalidate_visible(true, window, cx);
            self.focus_destination(window, cx);
            cx.notify();
        }
        if let Some(opener) = opener {
            return_focus(opener, window, cx);
        }
    }

    fn show_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.pages.current().cloned() {
            Some(Page::Series(id)) => self.series_page.update(cx, |p, cx| p.show(id, window, cx)),
            Some(Page::Author(id)) => self.author_page.update(cx, |p, cx| p.show(id, window, cx)),
            Some(Page::Book(book)) => self.book_page.update(cx, |p, cx| {
                p.set_subject(Some(Subject::Book(book)), window, cx);
            }),
            Some(Page::Upcoming) => {}
            None => return,
        }
        self.revalidate_visible(true, window, cx);
        self.focus_page(window, cx);
        cx.notify();
    }

    /// The page's book list, or the page itself for a book.
    fn focus_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.pages.current() {
            Some(Page::Series(_)) => {
                let fallback = self.page_focus.clone();
                self.series_page
                    .update(cx, |p, cx| p.focus(&fallback, window, cx));
            }
            Some(Page::Author(_)) => {
                let fallback = self.page_focus.clone();
                self.author_page
                    .update(cx, |p, cx| p.focus(&fallback, window, cx));
            }
            Some(Page::Book(_) | Page::Upcoming) => window.focus(&self.page_focus, cx),
            None => {}
        }
    }

    /// The page over the destination, under its object bar: Back, and the
    /// page's own tools (a book's).
    fn render_page(
        &mut self,
        page: &Page,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let content: AnyElement = match page {
            Page::Series(_) => self.series_page.clone().into_any_element(),
            Page::Author(_) => self.author_page.clone().into_any_element(),
            Page::Book(_) => self.book_page.clone().into_any_element(),
            Page::Upcoming => self.upcoming_page.clone().into_any_element(),
        };
        let tools = match page {
            Page::Book(_) => tools_of(&self.book_page, window, cx),
            _ => Vec::new(),
        };
        v_flex()
            .id("catalog-page")
            .key_context(PAGE_CONTEXT)
            .track_focus(&self.page_focus)
            .size_full()
            .child(
                ObjectBar::new()
                    .hairline(true)
                    .back(
                        self.back_label(cx),
                        cx.listener(|this, _, window, cx| this.go_back(window, cx)),
                    )
                    .tools(tools),
            )
            .child(div().flex_1().min_h_0().child(content))
    }

    /// Where Back goes: the page under this one, or the destination.
    fn back_label(&self, cx: &App) -> SharedString {
        let destination = match self.destination {
            Destination::Home => SharedString::new_static("Home"),
            Destination::Search => SharedString::new_static("Search"),
            Destination::Discover => SharedString::new_static("Discover"),
            Destination::Series => SharedString::new_static("Series"),
            Destination::Stats => SharedString::new_static("Stats"),
            Destination::Shelf(status) => SharedString::new_static(status_label(status)),
            Destination::List(id) => self.lists.read(cx).list(id).map_or_else(
                || SharedString::new_static("Back"),
                |list| SharedString::from(list.name.clone()),
            ),
        };
        let previous = self.pages.previous().map(|page| match page {
            Page::Book(book) => BackTo::Named(&book.title),
            Page::Upcoming => BackTo::Named("Coming soon"),
            Page::Series(id) => self
                .series_page
                .read(cx)
                .name(*id)
                .map_or(BackTo::Unnamed, BackTo::Named),
            Page::Author(id) => self
                .author_page
                .read(cx)
                .name(*id)
                .map_or(BackTo::Unnamed, BackTo::Named),
        });
        back_label(previous, &destination)
    }

    fn show_shelf(&mut self, status: ReadingStatus, window: &mut Window, cx: &mut Context<Self>) {
        self.show(Destination::Shelf(status), window, cx);
    }

    /// The session now holds a new grant: features it adds come alive.
    fn on_reauthorized(&mut self, event: &Reauthorized, cx: &mut Context<Self>) {
        Services::set_scopes(event.scopes.clone(), cx);
        self.goals.update(cx, GoalsModel::load);
        self.lists.update(cx, ListsModel::reload);
        self.upcoming.update(cx, UpcomingModel::load);
        self.series.update(cx, SeriesModel::load);
        cx.notify();
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        freshness::refresh_all(cx);
        self.revalidate_visible(true, window, cx);
        self.model.update(cx, LibraryModel::sync);
        self.goals.update(cx, GoalsModel::load);
        self.lists.update(cx, ListsModel::reload);
        self.upcoming.update(cx, UpcomingModel::load);
        self.series.update(cx, SeriesModel::load);
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.clone();
        let view = cx.new(|cx| SettingsView::new(model, cx));
        self.settings_subscription =
            Some(
                cx.subscribe_in(&view, window, |_, _, _: &SignOutRequested, window, cx| {
                    // One overlay at a time: the confirmation replaces Settings.
                    window.close_all_dialogs(cx);
                    Self::confirm_sign_out(window, cx);
                }),
            );
        open_dialog(window, cx, move |dialog, window, _| {
            dialog
                .title("Settings")
                .w(size::DIALOG_FORM.to_pixels(window.rem_size()))
                .child(view.clone())
        });
    }

    fn confirm_sign_out(window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        open_alert_dialog(window, cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .title("Sign out of Hardcover?")
                .description("Your library is removed from this computer. It stays on Hardcover.")
                .confirm()
                .ok_text("Sign out")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, _, cx| {
                    let _ = view.update(cx, Workspace::sign_out);
                    true
                })
        });
    }

    fn sign_out(&mut self, cx: &mut Context<Self>) {
        if self.signing_out {
            return;
        }
        self.signing_out = true;
        // A "Sign in again…" still waiting on the browser must not sign the
        // user back in once the session is gone.
        self.reauthorize.update(cx, Reauthorize::cancel);
        cx.notify();
        let services = Services::global(cx).clone();
        cx.spawn(async move |this, cx| {
            cx.background_spawn(async move {
                if let Some(session) = services.session()
                    && let Err(e) = session.sign_out()
                {
                    // Signed out locally either way; only the revoke failed.
                    eprintln!("hardcover: sign-out: {e}");
                }
                if let Some(cache) = services.cache()
                    && let Err(e) = cache.clear()
                {
                    eprintln!("hardcover: could not clear the library cache: {e}");
                }
            })
            .await;
            let _ = this.update(cx, |_, cx| cx.emit(SignedOut));
        })
        .detach();
    }

    /// Shows the sidebar's shortcuts after Ctrl has been held alone (Shift
    /// may join it) for a moment, and hides them as soon as it's released.
    fn modifiers_changed(&mut self, modifiers: Modifiers, cx: &mut Context<Self>) {
        if !ctrl_held(modifiers) {
            self.key_hint_delay = None;
            if self.key_hints {
                self.key_hints = false;
                cx.notify();
            }
            return;
        }
        if self.key_hints || self.key_hint_delay.is_some() {
            return;
        }
        self.key_hint_delay = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(KEY_HINT_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.key_hint_delay = None;
                this.key_hints = true;
                cx.notify();
            });
        }));
    }

    fn sidebar_lists(&self, cx: &Context<Self>) -> Option<gpui_kit::Div> {
        lists_section(
            &self.lists,
            match self.destination {
                Destination::List(id) => Some(id),
                _ => None,
            },
            Rc::new({
                let view = cx.entity().downgrade();
                move |id, window, cx| {
                    let _ =
                        view.update(cx, |this, cx| this.show(Destination::List(id), window, cx));
                }
            }),
            cx,
        )
    }

    fn destination_rows(&self, window: &Window, cx: &mut Context<Self>) -> gpui_kit::Div {
        let model = self.model.read(cx);
        let view = cx.entity().downgrade();
        // The modifiers are read again here: a release while a dialog had
        // focus never reached this view, and must not leave the hints up.
        let key_hints = self.key_hints && ctrl_held(window.modifiers());
        // While Ctrl is held, a destination's shortcut takes the place of its
        // count. Read from the bindings, so the hint can't drift from them.
        let hint = |action: &dyn Action| {
            key_hints
                .then(|| Kbd::global_binding_for_action(action, window))
                .flatten()
        };
        let nav_row = |id: &'static str,
                       label: &'static str,
                       icon: Icon,
                       destination: Destination,
                       action: &dyn Action| {
            let view = view.clone();
            NavRow::new(id, icon, label)
                .selected(self.destination == destination)
                .hint(hint(action))
                .on_click(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.show(destination, window, cx));
                })
        };
        let shelves = SIDEBAR_SHELVES.map(|status| {
            let action = shelf_action(status);
            nav_row(
                shelf_row_id(status),
                status_label(status),
                crate::chip::status_icon(status).into(),
                Destination::Shelf(status),
                action.as_ref(),
            )
            .count(count_text(model.count(status)))
        });
        let goals = self
            .goals_section
            .read(cx)
            .is_visible(cx)
            .then(|| self.goals_section.clone());
        let lists = self.sidebar_lists(cx);
        // Hidden without the catalogue, which the page reads.
        let series_row = Services::global(cx).has_scope(READ_CATALOG).then(|| {
            let today = chrono::Local::now().date_naive();
            let count = self.series.update(cx, |s, cx| {
                s.is_loaded().then(|| {
                    s.summaries(today, cx)
                        .iter()
                        .filter(|s| !s.group.is_finished())
                        .count()
                })
            });
            nav_row(
                "nav-series",
                "Series",
                crate::icons::library().into(),
                Destination::Series,
                &ShowSeries,
            )
            // Nothing to count shows no count, as the empty page has nothing.
            .when_some(count.filter(|n| *n > 0), |row, n| row.count(count_text(n)))
        });
        v_flex()
            .p_2()
            .child(nav_row(
                "nav-home",
                "Home",
                IconName::LayoutDashboard.into(),
                Destination::Home,
                &ShowHome,
            ))
            .child(nav_row(
                "nav-search",
                "Search",
                IconName::Search.into(),
                Destination::Search,
                &FocusSearch,
            ))
            .child(nav_row(
                "nav-discover",
                "Discover",
                IconName::Globe.into(),
                Destination::Discover,
                &ShowDiscover,
            ))
            .children(series_row)
            .child(nav_row(
                "nav-stats",
                "Stats",
                IconName::ChartPie.into(),
                Destination::Stats,
                &ShowStats,
            ))
            .child(sidebar_label("Library", cx))
            .children(shelves)
            .children(goals)
            .children(lists)
    }

    fn render_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let destinations = self.destination_rows(window, cx);
        let colors = colors(cx);

        // The brand and the foot stay put; the destinations between them
        // scroll. The 8 inset sits inside the scroller, so a focus ring
        // isn't clipped and the scrollbar sits on the sidebar's edge.
        v_flex()
            .flex_none()
            .w(size::SIDEBAR)
            .h_full()
            .bg(colors.sidebar)
            .border_r(size::HAIRLINE)
            .border_color(colors.border)
            .child(div().flex_none().px_2().pt_2().child(brand_row(cx)))
            .child(
                div()
                    .id("sidebar-destinations")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.sidebar_scroll)
                    .child(destinations)
                    .child(gutter_scrollbar("sidebar-scrollbar", &self.sidebar_scroll))
                    .reveals_focus(&self.sidebar_scroll),
            )
            // When the destinations don't all fit (a short window), a
            // hairline over the account says the list goes on under it.
            .child(
                v_flex()
                    .flex_none()
                    .px_2()
                    .pt_2()
                    .pb_2()
                    .gap_2()
                    .when(
                        self.sidebar_scroll.max_offset().y > gpui_kit::Pixels::ZERO,
                        |this| this.border_t(size::HAIRLINE).border_color(colors.border),
                    )
                    .when(
                        self.reauthorize.read(cx).is_needed(Services::global(cx)),
                        |this| this.child(self.reauthorize.clone()),
                    )
                    .children(Self::render_update(cx))
                    .child(self.render_account(cx)),
            )
    }

    // ----- updates -----

    /// The sidebar's update row, while there is something to act on.
    fn render_update(cx: &mut Context<Self>) -> Option<AnyElement> {
        let updates = Updates::global(cx).read(cx);
        let view = cx.entity().downgrade();
        let button = match updates.state() {
            UpdateState::Idle | UpdateState::Checking => return None,
            UpdateState::Available(release) => Button::new("update")
                .icon(IconName::ArrowUp)
                .label(format!("Update to {}…", release.version))
                .on_click(move |_, window, cx| {
                    let _ = view.update(cx, |_, cx| Self::open_update_dialog(window, cx));
                }),
            UpdateState::Installing(release) => Button::new("update")
                .loading(true)
                .disabled(true)
                .label(format!("Installing {}…", release.version)),
            UpdateState::Installed(version) => Button::new("update")
                .icon(IconName::RotateCw)
                .label("Restart to update")
                .tooltip(format!("Version {version} is installed"))
                .on_click(|_, _, cx| cx.restart()),
        };
        Some(button.small_control().w_full().into_any_element())
    }

    fn check_for_updates(cx: &mut Context<Self>) {
        Updates::global(cx)
            .clone()
            .update(cx, |u, cx| u.check(true, cx));
    }

    fn on_update_event(event: &UpdateEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            UpdateEvent::UpToDate(version) => window.push_notification(
                Notification::success(format!(
                    "Hardcover Desktop {version} is the latest version."
                )),
                cx,
            ),
            UpdateEvent::Found => {
                // Found by "Check now" in Settings: show the release instead.
                window.close_all_dialogs(cx);
                Self::open_update_dialog(window, cx);
            }
            UpdateEvent::CheckFailed(message) => window.push_notification(
                Notification::error(message.clone()).title("Couldn't check for updates"),
                cx,
            ),
            UpdateEvent::InstallFailed(message) => window.push_notification(
                Notification::error(message.clone()).title("Couldn't install the update"),
                cx,
            ),
        }
    }

    fn open_update_dialog(window: &mut Window, cx: &mut Context<Self>) {
        let updates = Updates::global(cx).clone();
        let model = updates.read(cx);
        let UpdateState::Available(release) = model.state() else {
            return;
        };
        let title: SharedString = format!("Hardcover Desktop {}", release.version).into();
        let notes: SharedString = crate::update::release_notes(&release.notes).into();
        let page_url: SharedString = release.page_url.clone().into();
        let installable = model.can_install() && model.has_download(release);
        let description: SharedString = if installable {
            format!(
                "You have {}. The update downloads and installs in the background; restart the app to finish.",
                model.current()
            )
        } else if model.can_install() {
            format!(
                "You have {}. This release has no download for this computer; see the release page.",
                model.current()
            )
        } else {
            format!(
                "You have {}. Update it the same way you installed it.",
                model.current()
            )
        }
        .into();
        let notes_view = move |notes: &SharedString| {
            (!notes.is_empty()).then(|| {
                div()
                    .id("release-notes")
                    .max_h(size::DIALOG_NOTES_MAX)
                    .scroll_y_revealing_focus()
                    .child(TextView::markdown("release-notes-text", notes.clone()))
            })
        };
        if installable {
            open_alert_dialog_on_result(window, cx, move |alert, _, _| {
                let updates = updates.clone();
                let notes = notes.clone();
                alert
                    .title(title.clone())
                    .description(description.clone())
                    .content(move |content, _, _| content.children(notes_view(&notes)))
                    .confirm()
                    .cancel_text("Later")
                    .ok_text("Install")
                    .on_ok(move |_, _, cx| {
                        updates.update(cx, |u: &mut UpdateModel, cx| u.install(cx));
                        true
                    })
            });
        } else {
            // Nothing to install here: the release page is a link (it opens
            // the browser), and the dialog's one button closes it.
            open_alert_dialog(window, cx, move |alert, _, _| {
                let notes = notes.clone();
                let page_url = page_url.clone();
                alert
                    .title(title.clone())
                    .description(description.clone())
                    .content(move |content, _, _| {
                        content
                            .child(ExternalLink::new(
                                "release-page",
                                "Open the release page",
                                page_url.clone(),
                            ))
                            .children(notes_view(&notes))
                    })
                    .ok_text("Close")
            });
        }
    }

    fn render_account(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let user = model.user().cloned();
        let (status, sync_tooltip) = sync_status_line(model, cx);
        let username: SharedString = user
            .as_ref()
            .map_or_else(|| "Account".into(), |u| u.username.clone().into());
        // Initials from the name when there is one ("Jane Quill"), else the
        // username.
        let avatar = Avatar::for_user(user.as_ref(), username.clone());
        let profile_url = user
            .as_ref()
            .map(|u| format!("https://hardcover.app/@{}", u.username));
        let view = cx.entity().downgrade();
        let focus = self.account_focus.clone();
        // After "Later", the permissions notice is one menu item away.
        let reauthorize = self.reauthorize.clone();
        let offer_permissions = {
            let notice = self.reauthorize.read(cx);
            notice.is_dismissed() && {
                let services = Services::global(cx);
                services.session().is_some() && !services.missing_scopes().is_empty()
            }
        };
        AccountRow::new("account", avatar, username, status)
            .focus(&self.account_focus)
            .status_tooltip(sync_tooltip)
            // Above the row, so the row shows that its menu is open.
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, _| {
                let view = view.clone();
                let focus = focus.clone();
                // Menu commands run from the row, so the dialogs they open
                // give focus back to it.
                let menu = menu.action_context(focus.clone()).menu_with_icon(
                    "Refresh library",
                    IconName::RefreshCw,
                    Box::new(Refresh),
                );
                let menu = match &profile_url {
                    Some(url) => menu.link_with_icon(
                        "Open profile on Hardcover",
                        IconName::CircleUser,
                        url.clone(),
                    ),
                    None => menu,
                };
                let menu = if offer_permissions {
                    let reauthorize = reauthorize.clone();
                    menu.item(PopupMenuItem::new("Sign in again…").on_click(
                        move |_, window, cx| {
                            reauthorize.update(cx, |r, cx| r.sign_in_again(window, cx));
                        },
                    ))
                } else {
                    menu
                };
                let updates_view = view.clone();
                menu.separator()
                    .menu_with_icon("Settings…", IconName::Settings, Box::new(OpenSettings))
                    .item(
                        PopupMenuItem::new("Check for updates").on_click(move |_, _, cx| {
                            let _ =
                                updates_view.update(cx, |_, cx| Workspace::check_for_updates(cx));
                        }),
                    )
                    .menu_with_icon(
                        "About Hardcover Desktop…",
                        IconName::Info,
                        Box::new(ShowAbout),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::new("Sign out…").on_click(move |_, window, cx| {
                            window.focus(&focus, cx);
                            let _ = view.update(cx, |_, cx| Self::confirm_sign_out(window, cx));
                        }),
                    )
            })
    }
}

fn sync_status_line(model: &LibraryModel, cx: &App) -> (gpui_kit::Div, Option<SharedString>) {
    let (status_icon, status_text): (Option<IconName>, SharedString) = match model.sync_status() {
        SyncStatus::Syncing => (
            None,
            match model.sync_fetched() {
                0 => "Syncing…".into(),
                n => format!("Syncing… {} books", count_text(n)).into(),
            },
        ),
        SyncStatus::Failed { offline: true, .. } => (Some(IconName::CircleAlert), "Offline".into()),
        SyncStatus::Failed { .. } => (Some(IconName::CircleAlert), "Couldn't sync".into()),
        SyncStatus::Synced | SyncStatus::Idle => match model.last_synced_at() {
            Some(at) => (
                None,
                format!("Updated {}", relative_time(&chrono::Utc::now(), &at)).into(),
            ),
            None => (None, "Not synced yet".into()),
        },
    };
    let sync_tooltip = match model.sync_status() {
        SyncStatus::Failed {
            message,
            offline: true,
        } => Some(format!("Showing your saved library. {message}").into()),
        SyncStatus::Failed { message, .. } => Some(message.clone()),
        _ => None,
    };
    // While syncing, a 12 px spinner where the other states have their
    // icon, so the line shows the work moving, not only its count.
    let syncing = matches!(model.sync_status(), SyncStatus::Syncing);
    let status = h_flex()
        .min_w_0()
        .gap_1()
        .when(syncing, |this| {
            this.child(Spinner::new().xsmall().color(colors(cx).fg_muted))
        })
        .children(status_icon.map(|icon| Icon::new(icon).size_3().flex_none()))
        .child(div().min_w_0().truncate().tabular().child(status_text));

    (status, sync_tooltip)
}

/// Ctrl alone, or with Shift (Discover and Stats): what the sidebar hints are for.
fn ctrl_held(modifiers: Modifiers) -> bool {
    modifiers.control && !modifiers.alt && !modifiers.platform && !modifiers.function
}

/// A shelf's sidebar row, keyed by its status.
fn shelf_row_id(status: ReadingStatus) -> &'static str {
    match status {
        ReadingStatus::CurrentlyReading => "nav-currently-reading",
        ReadingStatus::WantToRead => "nav-want-to-read",
        ReadingStatus::Read => "nav-read",
        ReadingStatus::Paused => "nav-paused",
        ReadingStatus::DidNotFinish => "nav-did-not-finish",
        ReadingStatus::Ignored => "nav-ignored",
    }
}

/// The action that shows a shelf, bound to Ctrl+1…6 in `main`.
fn shelf_action(status: ReadingStatus) -> Box<dyn Action> {
    match status {
        ReadingStatus::CurrentlyReading => Box::new(ShowCurrentlyReading),
        ReadingStatus::WantToRead => Box::new(ShowWantToRead),
        ReadingStatus::Read => Box::new(ShowRead),
        ReadingStatus::Paused => Box::new(ShowPaused),
        ReadingStatus::DidNotFinish => Box::new(ShowDidNotFinish),
        ReadingStatus::Ignored => Box::new(ShowIgnored),
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                this.modifiers_changed(event.modifiers, cx);
            }))
            .size_full()
            .items_stretch()
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.show(Destination::Search, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Refresh, window, cx| this.refresh(window, cx)))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| this.go_back(window, cx)))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .on_action(cx.listener(|_, _: &ShowAbout, window, cx| open_about(window, cx)))
            .on_action(cx.listener(|this, _: &ShowCurrentlyReading, window, cx| {
                this.show_shelf(ReadingStatus::CurrentlyReading, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowWantToRead, window, cx| {
                this.show_shelf(ReadingStatus::WantToRead, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowRead, window, cx| {
                this.show_shelf(ReadingStatus::Read, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowPaused, window, cx| {
                this.show_shelf(ReadingStatus::Paused, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowDidNotFinish, window, cx| {
                this.show_shelf(ReadingStatus::DidNotFinish, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowIgnored, window, cx| {
                this.show_shelf(ReadingStatus::Ignored, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowDiscover, window, cx| {
                this.show(Destination::Discover, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowSeries, window, cx| {
                if Services::global(cx).has_scope(READ_CATALOG) {
                    this.show(Destination::Series, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ShowStats, window, cx| {
                this.show(Destination::Stats, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ShowHome, window, cx| {
                this.show(Destination::Home, window, cx);
            }))
            .child(self.render_sidebar(window, cx))
            .child(div().flex_1().min_w_0().h_full().map(|this| {
                match (self.pages.current().cloned(), self.destination) {
                    (Some(page), _) => this.child(self.render_page(&page, window, cx)),
                    (None, Destination::Home) => this.child(self.home.clone()),
                    (None, Destination::Search) => this.child(self.search.clone()),
                    (None, Destination::Discover) => this.child(self.discover.clone()),
                    (None, Destination::Series) => this.child(self.series_view.clone()),
                    (None, Destination::Stats) => this.child(self.stats.clone()),
                    (None, Destination::Shelf(_)) => this.child(self.library.clone()),
                    (None, Destination::List(_)) => this.child(self.list_page.clone()),
                }
            }))
    }
}
