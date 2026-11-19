//! The window's content: the sign-in screen or the workspace (sidebar plus the
//! current destination).

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::avatar::Avatar;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::sidebar::{
    Sidebar, SidebarFooter, SidebarGroup, SidebarItem, SidebarMenu, SidebarMenuItem,
};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{
    ActiveTheme as _, Collapsible, Disableable as _, Icon, IconName, Sizable as _, WindowExt as _,
    h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AnyView, App, AppContext as _, Context, ElementId, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window,
    div, prelude::FluentBuilder as _, rems,
};
use hardcover_core::ReadingStatus;
use hardcover_core::store::LibraryCache;

use crate::about::open_about;
use crate::app_state::{CoverService, Services, Startup, cover_file, startup};
use crate::book::detail_view::{BookDetailView, Subject};
use crate::catalog::author_view::AuthorView;
use crate::catalog::navigation::{NavigationEvent, Navigator, Page, PageStack};
use crate::catalog::series_view::SeriesView;
use crate::format::{SIDEBAR_SHELVES, relative_time, status_label};
use crate::goals::model::GoalsModel;
use crate::goals::sidebar::GoalsSection;
use crate::library::library_view::LibraryView;
use crate::library::model::{LibraryModel, SyncStatus};
use crate::search::search_view::SearchView;
use crate::settings_view::{SettingsView, SignOutRequested};
use crate::sign_in::reauthorize::{Reauthorize, Reauthorized};
use crate::sign_in::sign_in_view::{SignInView, SignedIn};
use crate::update::{UpdateEvent, UpdateModel, UpdateState, Updates};
use crate::{
    FocusSearch, GoBack, OpenSettings, Refresh, ShowAbout, ShowCurrentlyReading, ShowDidNotFinish,
    ShowIgnored, ShowPaused, ShowRead, ShowWantToRead,
};

/// Key context of a catalogue page, where Escape goes back.
pub const PAGE_CONTEXT: &str = "CatalogPage";

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
    _subscription: Option<Subscription>,
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
                        this.cache = cache.clone();
                        let services = Services::new(client, session, cache, scopes);
                        this.enter_workspace(services, window, cx);
                    }
                    Startup::SignedOut { cache } => {
                        this.cache = cache;
                        this.show_sign_in(window, cx);
                    }
                }
            });
        })
        .detach();
        Self {
            screen: Screen::Starting,
            cache: None,
            _subscription: None,
        }
    }

    fn enter_workspace(&mut self, services: Services, window: &mut Window, cx: &mut Context<Self>) {
        cx.set_global(services);
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        self._subscription = Some(cx.subscribe_in(
            &workspace,
            window,
            |this, _, _: &SignedOut, window, cx| this.show_sign_in(window, cx),
        ));
        workspace.update(cx, |w, cx| w.focus_initial(window, cx));
        self.screen = Screen::Workspace(workspace);
        cx.notify();
    }

    fn show_sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if cx.has_global::<Services>() {
            cx.remove_global::<Services>();
        }
        let sign_in = cx.new(|cx| SignInView::new(window, cx));
        self._subscription =
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
        self.screen = Screen::SignIn(sign_in);
        cx.notify();
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
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
    Search,
    Shelf(ReadingStatus),
}

/// Emitted after signing out.
pub struct SignedOut;

pub struct Workspace {
    focus_handle: FocusHandle,
    model: Entity<LibraryModel>,
    destination: Destination,
    library: Entity<LibraryView>,
    search: Entity<SearchView>,
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
    signing_out: bool,
    /// The open Settings dialog's sign-out request.
    _settings: Option<Subscription>,
    /// Re-renders periodically so "Updated 5 min ago" stays true.
    _clock: Task<()>,
    _subscriptions: Vec<Subscription>,
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
        let library = cx.new(|cx| LibraryView::new(model.clone(), window, cx));
        let search = cx.new(|cx| SearchView::new(model.clone(), window, cx));
        let series_page = cx.new(|cx| SeriesView::new(model.clone(), window, cx));
        let author_page = cx.new(|cx| AuthorView::new(model.clone(), window, cx));
        let book_page = cx.new(|cx| BookDetailView::new(model.clone(), window, cx));
        let navigator = Navigator::init(cx);
        let reauthorize = cx.new(|_| Reauthorize::new(model.clone()));
        let goals = cx.new(|cx| GoalsModel::new(model.clone(), cx));
        let goals_section = cx.new(|cx| GoalsSection::new(goals.clone(), model.clone(), cx));
        model.update(cx, |m, cx| m.start(cx));
        goals.update(cx, |g, cx| g.load(cx));
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
        let updates = Updates::global(cx).clone();
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.observe(&updates, |_, _, cx| cx.notify()),
            cx.observe(&reauthorize, |_, _, cx| cx.notify()),
            cx.observe(&goals, |_, _, cx| cx.notify()),
            cx.subscribe(&reauthorize, |this, _, event: &Reauthorized, cx| {
                this.on_reauthorized(event, cx)
            }),
            cx.subscribe_in(&updates, window, |this, _, event, window, cx| {
                this.on_update_event(event, window, cx)
            }),
            // A dialog or menu closing can drop focus (its trigger may be gone,
            // e.g. after removing a book). Keyboard shortcuts need a focused
            // element, so return to the current destination.
            cx.on_focus_lost(window, |this, window, cx| {
                this.focus_destination(window, cx)
            }),
            cx.subscribe_in(
                &navigator,
                window,
                |this, _, event: &NavigationEvent, window, cx| match event {
                    NavigationEvent::Open(page) => this.open_page(page.clone(), window, cx),
                },
            ),
        ];
        Self {
            focus_handle: cx.focus_handle(),
            model,
            destination: Destination::Shelf(ReadingStatus::CurrentlyReading),
            library,
            search,
            pages: PageStack::default(),
            series_page,
            author_page,
            book_page,
            page_focus: cx.focus_handle(),
            reauthorize,
            goals,
            goals_section,
            signing_out: false,
            _settings: None,
            _clock: clock,
            _subscriptions: subscriptions,
        }
    }

    fn focus_initial(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.library.update(cx, |l, cx| l.focus_list(window, cx));
    }

    /// Focuses the current destination's primary control.
    fn focus_destination(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pages.current().is_some() {
            self.focus_page(window, cx);
            return;
        }
        match self.destination {
            Destination::Search => self.search.update(cx, |s, cx| s.focus_input(window, cx)),
            Destination::Shelf(_) => self.library.update(cx, |l, cx| l.focus_list(window, cx)),
        }
    }

    fn show(&mut self, destination: Destination, window: &mut Window, cx: &mut Context<Self>) {
        // A sidebar destination starts a new history.
        self.pages.clear();
        self.destination = destination;
        match destination {
            Destination::Search => self.search.update(cx, |s, cx| s.focus_input(window, cx)),
            Destination::Shelf(status) => {
                self.library
                    .update(cx, |l, cx| l.show_shelf(status, window, cx));
                self.library.update(cx, |l, cx| l.focus_list(window, cx));
            }
        }
        cx.notify();
    }

    // ----- catalogue pages -----

    fn open_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.pages.push(page) {
            self.show_page(window, cx);
        }
    }

    /// Back to the previous page, or to the destination after the first one.
    fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pages.pop() {
            return;
        }
        if self.pages.current().is_some() {
            self.show_page(window, cx);
        } else {
            self.focus_destination(window, cx);
            cx.notify();
        }
    }

    fn show_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.pages.current().cloned() {
            Some(Page::Series(id)) => self.series_page.update(cx, |p, cx| p.show(id, window, cx)),
            Some(Page::Author(id)) => self.author_page.update(cx, |p, cx| p.show(id, window, cx)),
            Some(Page::Book(book)) => self.book_page.update(cx, |p, cx| {
                p.set_subject(Some(Subject::Book(book)), window, cx)
            }),
            None => return,
        }
        self.focus_page(window, cx);
        cx.notify();
    }

    /// The page's book list, or the page itself for a book.
    fn focus_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.pages.current() {
            Some(Page::Series(_)) => {
                let fallback = self.page_focus.clone();
                self.series_page
                    .update(cx, |p, cx| p.focus(&fallback, window, cx))
            }
            Some(Page::Author(_)) => {
                let fallback = self.page_focus.clone();
                self.author_page
                    .update(cx, |p, cx| p.focus(&fallback, window, cx))
            }
            Some(Page::Book(_)) => window.focus(&self.page_focus, cx),
            None => {}
        }
    }

    fn render_page(&self, page: &Page, cx: &mut Context<Self>) -> impl IntoElement {
        let content: AnyElement = match page {
            Page::Series(_) => self.series_page.clone().into_any_element(),
            Page::Author(_) => self.author_page.clone().into_any_element(),
            Page::Book(_) => self.book_page.clone().into_any_element(),
        };
        v_flex()
            .id("catalog-page")
            .key_context(PAGE_CONTEXT)
            .track_focus(&self.page_focus)
            .size_full()
            .child(
                h_flex()
                    .flex_shrink_0()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("go-back")
                            .ghost()
                            .small()
                            .icon(IconName::ArrowLeft)
                            .label("Back")
                            .tooltip_with_action("Back", &GoBack, Some("Workspace"))
                            .on_click(cx.listener(|this, _, window, cx| this.go_back(window, cx))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(content))
    }

    fn show_shelf(&mut self, status: ReadingStatus, window: &mut Window, cx: &mut Context<Self>) {
        self.show(Destination::Shelf(status), window, cx);
    }

    /// The session now holds a new grant: features it adds come alive.
    fn on_reauthorized(&mut self, event: &Reauthorized, cx: &mut Context<Self>) {
        Services::set_scopes(event.scopes.clone(), cx);
        self.goals.update(cx, |g, cx| g.load(cx));
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.model.update(cx, |m, cx| m.sync(cx));
        self.goals.update(cx, |g, cx| g.load(cx));
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.clone();
        let view = cx.new(|cx| SettingsView::new(model, cx));
        self._settings = Some(cx.subscribe_in(
            &view,
            window,
            |this, _, _: &SignOutRequested, window, cx| {
                // One overlay at a time: the confirmation replaces Settings.
                window.close_all_dialogs(cx);
                this.confirm_sign_out(window, cx);
            },
        ));
        window.open_dialog(cx, move |dialog, window, _| {
            dialog
                .title("Settings")
                .w(rems(30.).to_pixels(window.rem_size()))
                .child(view.clone())
        });
    }

    fn confirm_sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let view = view.clone();
            alert
                .title("Sign out of Hardcover?")
                .description("Your library is removed from this computer. It stays on Hardcover.")
                .confirm()
                .ok_text("Sign out")
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| this.sign_out(cx));
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
        self.reauthorize.update(cx, |r, cx| r.cancel(cx));
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

    fn render_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let view = cx.entity().downgrade();
        let nav_item = |label: &'static str, icon: IconName, destination: Destination| {
            let view = view.clone();
            SidebarMenuItem::new(label)
                .icon(icon)
                .active(self.destination == destination)
                .on_click(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.show(destination, window, cx));
                })
        };
        let shelves = SIDEBAR_SHELVES.map(|status| {
            let count = model.count(status);
            nav_item(
                status_label(status),
                shelf_icon(status),
                Destination::Shelf(status),
            )
            .suffix(move |_, cx| {
                div()
                    .text_xs()
                    .text_color(cx.theme().sidebar_foreground.opacity(0.7))
                    .child(count.to_string())
            })
        });

        Sidebar::new("sidebar")
            .collapsible(false)
            .w(rems(15.))
            .header(
                h_flex()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .child(Icon::new(IconName::BookOpen))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Hardcover")),
            )
            .child(NavSection::Menu(Box::new(SidebarMenu::new().child(
                nav_item("Search", IconName::Search, Destination::Search),
            ))))
            .child(NavSection::Group(
                SidebarGroup::new("Library").child(SidebarMenu::new().children(shelves)),
            ))
            .when(self.goals_section.read(cx).is_visible(cx), |this| {
                this.child(NavSection::View(self.goals_section.clone().into()))
            })
            .footer(
                v_flex()
                    .w_full()
                    .gap_2()
                    .when(
                        self.reauthorize.read(cx).is_needed(Services::global(cx)),
                        |this| this.child(self.reauthorize.clone()),
                    )
                    .children(self.render_update(cx))
                    .child(self.render_account(window, cx)),
            )
    }

    // ----- updates -----

    /// The sidebar's update row, while there is something to act on.
    fn render_update(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let updates = Updates::global(cx).read(cx);
        let view = cx.entity().downgrade();
        let button = match updates.state() {
            UpdateState::Idle | UpdateState::Checking => return None,
            UpdateState::Available(release) => Button::new("update")
                .outline()
                .icon(IconName::ArrowUp)
                .label(format!("Update to {}…", release.version))
                .on_click(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.open_update_dialog(window, cx));
                }),
            UpdateState::Installing(release) => Button::new("update")
                .outline()
                .loading(true)
                .disabled(true)
                .label(format!("Installing {}…", release.version)),
            UpdateState::Installed(version) => Button::new("update")
                .primary()
                .icon(IconName::RotateCw)
                .label("Restart to update")
                .tooltip(format!("Version {version} is installed"))
                .on_click(|_, _, cx| cx.restart()),
        };
        Some(
            div()
                .w_full()
                .px_2()
                .child(button.small().w_full())
                .into_any_element(),
        )
    }

    fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        Updates::global(cx)
            .clone()
            .update(cx, |u, cx| u.check(true, cx));
    }

    fn on_update_event(
        &mut self,
        event: &UpdateEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
                self.open_update_dialog(window, cx)
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

    fn open_update_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let updates = Updates::global(cx).clone();
        let model = updates.read(cx);
        let UpdateState::Available(release) = model.state() else {
            return;
        };
        let title: SharedString = format!("Hardcover Desktop {}", release.version).into();
        let notes: SharedString = crate::update::release_notes(&release.notes).into();
        let page_url = release.page_url.clone();
        let installable = model.can_install() && release.asset.is_some();
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
        window.open_alert_dialog(cx, move |alert, _, _| {
            let updates = updates.clone();
            let page_url = page_url.clone();
            let notes = notes.clone();
            let alert = alert
                .title(title.clone())
                .description(description.clone())
                .content(move |content, _, _| {
                    content.when(!notes.is_empty(), |this| {
                        this.child(
                            div()
                                .id("release-notes")
                                .max_h(rems(18.))
                                .overflow_y_scrollbar()
                                .child(TextView::markdown("release-notes-text", notes.clone())),
                        )
                    })
                })
                .confirm()
                .cancel_text("Later");
            if installable {
                alert.ok_text("Install").on_ok(move |_, _, cx| {
                    updates.update(cx, |u: &mut UpdateModel, cx| u.install(cx));
                    true
                })
            } else {
                alert.ok_text("Open release page").on_ok(move |_, _, cx| {
                    cx.open_url(&page_url);
                    true
                })
            }
        });
    }

    fn render_account(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let user = model.user().cloned();
        let muted = cx.theme().sidebar_foreground.opacity(0.7);
        let (status_icon, status_text): (Option<AnyElement>, SharedString) =
            match model.sync_status() {
                SyncStatus::Syncing => (
                    Some(Spinner::new().xsmall().into_any_element()),
                    match model.sync_fetched() {
                        0 => "Syncing…".into(),
                        n => format!("Syncing… {n} books").into(),
                    },
                ),
                SyncStatus::Failed { offline: true, .. } => (
                    Some(Icon::new(IconName::CircleAlert).xsmall().into_any_element()),
                    "Offline".into(),
                ),
                SyncStatus::Failed { .. } => (
                    Some(Icon::new(IconName::CircleAlert).xsmall().into_any_element()),
                    "Couldn't sync".into(),
                ),
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
        let username: SharedString = user
            .as_ref()
            .map(|u| u.username.clone().into())
            .unwrap_or_else(|| "Account".into());
        let avatar = user
            .as_ref()
            .and_then(|u| u.avatar.as_ref())
            .map(|a| a.url.clone())
            .and_then(|url| cover_file(&url, window, cx));
        let profile_url = user
            .as_ref()
            .map(|u| format!("https://hardcover.app/@{}", u.username));
        let view = cx.entity().downgrade();
        let focus = self.focus_handle.clone();
        // After "Later", the permissions notice is one menu item away.
        let reauthorize = self.reauthorize.clone();
        let offer_permissions = {
            let notice = self.reauthorize.read(cx);
            notice.is_dismissed() && {
                let services = Services::global(cx);
                services.session().is_some() && !services.missing_scopes().is_empty()
            }
        };

        // The dropdown wrapper sizes to its content; this block lets the
        // footer (and its hover surface) span the sidebar.
        div().w_full().child(
            SidebarFooter::new()
                .child(
                    h_flex()
                        .gap_2()
                        .flex_1()
                        .min_w_0()
                        .child(
                            Avatar::new()
                                .small()
                                .name(username.clone())
                                .when_some(avatar, |this, path| this.src(path)),
                        )
                        .child(
                            v_flex()
                                .min_w_0()
                                .flex_1()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .truncate()
                                        .child(username),
                                )
                                .child(
                                    h_flex()
                                        .id("sync-status")
                                        .min_w_0()
                                        .gap_1()
                                        .text_xs()
                                        .text_color(muted)
                                        .children(status_icon)
                                        .child(div().min_w_0().truncate().child(status_text))
                                        .when_some(sync_tooltip, |this, tip| {
                                            this.tooltip(move |window, cx| {
                                                gpui_kit::component::tooltip::Tooltip::new(
                                                    tip.clone(),
                                                )
                                                .build(window, cx)
                                            })
                                        }),
                                ),
                        ),
                )
                .child(
                    Icon::new(IconName::ChevronsUpDown)
                        .xsmall()
                        .text_color(muted),
                )
                .dropdown_menu(move |menu, _, _| {
                    let view = view.clone();
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
                    let menu =
                        if offer_permissions {
                            let reauthorize = reauthorize.clone();
                            menu.item(PopupMenuItem::new("Grant new permissions…").on_click(
                                move |_, _, cx| reauthorize.update(cx, |r, cx| r.show(cx)),
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
                                    updates_view.update(cx, |this, cx| this.check_for_updates(cx));
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
                                let _ =
                                    view.update(cx, |this, cx| this.confirm_sign_out(window, cx));
                            }),
                        )
                }),
        )
    }
}

fn shelf_icon(status: ReadingStatus) -> IconName {
    match status {
        ReadingStatus::CurrentlyReading => IconName::BookOpen,
        ReadingStatus::WantToRead => IconName::Inbox,
        ReadingStatus::Read => IconName::CircleCheck,
        ReadingStatus::Paused => IconName::Pause,
        ReadingStatus::DidNotFinish => IconName::Ban,
        ReadingStatus::Ignored => IconName::EyeOff,
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .size_full()
            .items_stretch()
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.show(Destination::Search, window, cx)
            }))
            .on_action(cx.listener(|this, _: &Refresh, _, cx| this.refresh(cx)))
            .on_action(cx.listener(|this, _: &GoBack, window, cx| this.go_back(window, cx)))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.open_settings(window, cx)),
            )
            .on_action(cx.listener(|_, _: &ShowAbout, window, cx| open_about(window, cx)))
            .on_action(cx.listener(|this, _: &ShowCurrentlyReading, window, cx| {
                this.show_shelf(ReadingStatus::CurrentlyReading, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowWantToRead, window, cx| {
                this.show_shelf(ReadingStatus::WantToRead, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowRead, window, cx| {
                this.show_shelf(ReadingStatus::Read, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowPaused, window, cx| {
                this.show_shelf(ReadingStatus::Paused, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowDidNotFinish, window, cx| {
                this.show_shelf(ReadingStatus::DidNotFinish, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowIgnored, window, cx| {
                this.show_shelf(ReadingStatus::Ignored, window, cx)
            }))
            .child(self.render_sidebar(window, cx))
            .child(div().flex_1().min_w_0().h_full().map(|this| {
                match (self.pages.current(), self.destination) {
                    (Some(page), _) => this.child(self.render_page(page, cx)),
                    (None, Destination::Search) => this.child(self.search.clone()),
                    (None, Destination::Shelf(_)) => this.child(self.library.clone()),
                }
            }))
    }
}

/// The sidebar's sections. `Sidebar` takes one item type, and Search sits
/// above the Library group without a group of its own. A view draws a section
/// that keeps its own state (the reading goals).
#[derive(Clone)]
enum NavSection {
    Menu(Box<SidebarMenu>),
    Group(SidebarGroup<SidebarMenu>),
    View(AnyView),
}

impl Collapsible for NavSection {
    fn collapsed(self, collapsed: bool) -> Self {
        match self {
            Self::Menu(m) => Self::Menu(Box::new(m.collapsed(collapsed))),
            Self::Group(g) => Self::Group(g.collapsed(collapsed)),
            Self::View(v) => Self::View(v),
        }
    }

    fn is_collapsed(&self) -> bool {
        match self {
            Self::Menu(m) => m.is_collapsed(),
            Self::Group(g) => g.is_collapsed(),
            Self::View(_) => false,
        }
    }
}

impl SidebarItem for NavSection {
    fn render(
        self,
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        match self {
            Self::Menu(m) => (*m).render(id, window, cx).into_any_element(),
            Self::Group(g) => g.render(id, window, cx).into_any_element(),
            Self::View(v) => v.into_any_element(),
        }
    }
}
