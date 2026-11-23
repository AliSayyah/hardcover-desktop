//! The Settings dialog: the theme and its mode, update checks and the
//! signed-in account.
//!
//! Every change applies at once and is saved; there is nothing to confirm, so
//! the dialog only has its close button.

use gpui_kit::component::button::Button;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tab::TabBar;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::prefs::ThemePreference;

use crate::design::TypeRoles as _;
use crate::library::model::LibraryModel;
use crate::preferences::{AppPreferences, PreferencesModel, theme_family_override, theme_override};
use crate::ui::Avatar;
use crate::ui::ThemeTiles;
use crate::ui::kit::SmallControl as _;
use crate::ui::kit::small_tab;
use crate::ui::theme_tiles::family_name;
use crate::update::{UpdateModel, UpdateState, Updates, automatic_checks_unavailable};

/// The modes, in the segmented control's order.
const MODES: [ThemePreference; 3] = [
    ThemePreference::System,
    ThemePreference::Light,
    ThemePreference::Dark,
];

fn mode_name(mode: ThemePreference) -> &'static str {
    match mode {
        ThemePreference::System => "System",
        ThemePreference::Light => "Light",
        ThemePreference::Dark => "Dark",
    }
}

/// What a mode does, under its row's name.
fn mode_description(mode: ThemePreference) -> &'static str {
    match mode {
        ThemePreference::System => "Light or dark as your desktop is.",
        ThemePreference::Light => "Always light.",
        ThemePreference::Dark => "Always dark.",
    }
}

const FORCED: &str = "Set by HARDCOVER_THEME.";

/// Asks the workspace to sign out (after its own confirmation).
pub struct SignOutRequested;

pub struct SettingsView {
    preferences: Entity<PreferencesModel>,
    updates: Entity<UpdateModel>,
    library: Entity<LibraryModel>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SignOutRequested> for SettingsView {}

impl SettingsView {
    pub fn new(library: Entity<LibraryModel>, cx: &mut Context<Self>) -> Self {
        let preferences = AppPreferences::global(cx).clone();
        let updates = Updates::global(cx).clone();
        let subscriptions = vec![
            cx.observe(&preferences, |_, _, cx| cx.notify()),
            cx.observe(&updates, |_, _, cx| cx.notify()),
            cx.observe(&library, |_, _, cx| cx.notify()),
        ];
        Self {
            preferences,
            updates,
            library,
            _subscriptions: subscriptions,
        }
    }

    fn render_appearance(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let preferences = self.preferences.read(cx);
        let family = preferences.theme_family();
        let mode = preferences.theme();
        let model = self.preferences.clone();
        // The theme: tiles, or the forced one's name with why.
        let theme = match theme_family_override() {
            Some(forced) => setting_row(
                "Theme",
                Some(FORCED.into()),
                div().body().child(family_name(forced)),
                cx,
            ),
            None => v_flex()
                .gap_2()
                .child(div().body().child("Theme"))
                .child(ThemeTiles::new("theme", family, move |family, _, cx| {
                    model.update(cx, |p, cx| p.set_theme_family(family, cx));
                }))
                .into_any_element(),
        };
        let model = self.preferences.clone();
        let mode = match theme_override() {
            Some(forced) => setting_row(
                "Mode",
                Some(FORCED.into()),
                div().body().child(mode_name(forced)),
                cx,
            ),
            None => setting_row(
                "Mode",
                Some(mode_description(mode).into()),
                TabBar::new("theme-mode")
                    .segmented()
                    .small_control()
                    .selected_index(MODES.iter().position(|m| *m == mode).unwrap_or(0))
                    .children(MODES.map(|m| small_tab(mode_name(m))))
                    .on_click(move |ix, _, cx| {
                        let mode = MODES[*ix];
                        model.update(cx, |p, cx| p.set_theme(mode, cx));
                    }),
                cx,
            ),
        };
        section("Appearance", v_flex().gap_4().child(theme).child(mode))
    }

    fn render_updates(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let unavailable = automatic_checks_unavailable();
        let wanted = self.preferences.read(cx).automatic_update_checks();
        let preferences = self.preferences.clone();
        let switch = Switch::new("automatic-update-checks")
            .small()
            .checked(wanted && unavailable.is_none())
            .disabled(unavailable.is_some())
            .accessibility_label("Check for updates automatically")
            .on_change(move |checked, _, cx| {
                preferences.update(cx, |p, cx| p.set_automatic_update_checks(*checked, cx));
            });
        let automatic = setting_row(
            "Check for updates automatically",
            Some(
                unavailable
                    .unwrap_or("Looks for a new release on GitHub at startup and every 12 hours.")
                    .into(),
            ),
            switch,
            cx,
        );

        let model = self.updates.read(cx);
        let status: Option<SharedString> = match model.state() {
            UpdateState::Available(release) => {
                Some(format!("Version {} is available.", release.version).into())
            }
            UpdateState::Installing(release) => {
                Some(format!("Installing version {}…", release.version).into())
            }
            UpdateState::Installed(version) => {
                Some(format!("Version {version} is installed. Restart the app to use it.").into())
            }
            UpdateState::Idle | UpdateState::Checking => None,
        };
        let busy = matches!(
            model.state(),
            UpdateState::Installing(_) | UpdateState::Installed(_)
        );
        let updates = self.updates.clone();
        let check = Button::new("check-for-updates")
            .small_control()
            .label("Check now")
            .loading(matches!(model.state(), UpdateState::Checking))
            .disabled(busy)
            .on_click(move |_, _, cx| {
                updates.update(cx, |u, cx| u.check(true, cx));
            });
        let version = setting_row(
            format!("Hardcover Desktop {}", model.current()),
            status,
            check,
            cx,
        );
        section("Updates", v_flex().gap_4().child(automatic).child(version))
    }

    fn render_account(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let user = self.library.read(cx).user().cloned();
        let username: SharedString = user
            .as_ref()
            .map_or_else(|| "Signed in".into(), |u| u.username.clone().into());
        let avatar = Avatar::for_user(user.as_ref(), username.clone());
        let view = cx.entity().downgrade();
        let sign_out = Button::new("sign-out")
            .small_control()
            .label("Sign out…")
            .on_click(move |_, _, cx| {
                let _ = view.update(cx, |_, cx| cx.emit(SignOutRequested));
            });
        section(
            "Account",
            h_flex()
                .gap_3()
                .items_center()
                .child(avatar)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .body_strong()
                        .truncate()
                        .child(username),
                )
                .child(sign_out),
        )
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The top padding sets the first section apart from the dialog title.
        // Dialogs don't inherit the shell's family, so the roles set it.
        v_flex()
            .pt_2()
            .gap_6()
            .body()
            .child(self.render_appearance(cx))
            .child(self.render_updates(cx))
            .child(self.render_account(cx))
    }
}

/// A titled group of settings.
fn section(title: &'static str, content: impl IntoElement) -> AnyElement {
    v_flex()
        .gap_3()
        .child(div().heading().child(title))
        .child(content)
        .into_any_element()
}

/// A label with optional help text, and its control on the trailing edge.
fn setting_row(
    label: impl Into<SharedString>,
    description: Option<SharedString>,
    control: impl IntoElement,
    cx: &Context<SettingsView>,
) -> AnyElement {
    h_flex()
        .gap_4()
        .items_center()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().body().child(label.into()))
                .when_some(description, |this, description| {
                    this.child(div().caption(cx).child(description))
                }),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}
