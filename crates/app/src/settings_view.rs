//! The Settings dialog: theme, update checks and the signed-in account.
//!
//! Every change applies at once and is saved; there is nothing to confirm, so
//! the dialog only has its close button.

use gpui_kit::component::button::Button;
use gpui_kit::component::radio::{Radio, RadioGroup};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use hardcover_core::prefs::ThemePreference;

use crate::design::TypeRoles as _;
use crate::library::model::LibraryModel;
use crate::preferences::{AppPreferences, PreferencesModel, theme_override};
use crate::ui::Avatar;
use crate::update::{UpdateModel, UpdateState, Updates, automatic_checks_unavailable};

const THEMES: [(ThemePreference, &str); 3] = [
    (ThemePreference::System, "System"),
    (ThemePreference::Light, "Light"),
    (ThemePreference::Dark, "Dark"),
];

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
        let forced = theme_override();
        let current = forced.unwrap_or_else(|| self.preferences.read(cx).theme());
        let selected = THEMES.iter().position(|(theme, _)| *theme == current);
        let preferences = self.preferences.clone();
        let choices = RadioGroup::horizontal("theme")
            .children(THEMES.map(|(_, label)| Radio::new(SharedString::from(label)).label(label)))
            .selected_index(selected)
            .disabled(forced.is_some())
            .on_change(move |ix, _, cx| {
                if let Some((theme, _)) = THEMES.get(*ix) {
                    preferences.update(cx, |p, cx| p.set_theme(*theme, cx));
                }
            });
        section(
            "Appearance",
            setting_row(
                "Theme",
                forced.map(|_| "Set by HARDCOVER_THEME.".into()),
                choices,
                cx,
            ),
        )
    }

    fn render_updates(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let unavailable = automatic_checks_unavailable();
        let wanted = self.preferences.read(cx).automatic_update_checks();
        let preferences = self.preferences.clone();
        let switch = Switch::new("automatic-update-checks")
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
            .small()
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
            .map(|u| u.username.clone().into())
            .unwrap_or_else(|| "Signed in".into());
        let avatar = Avatar::new(
            user.as_ref()
                .and_then(|u| u.name.clone())
                .filter(|name| !name.trim().is_empty())
                .map_or_else(|| username.clone(), SharedString::from),
        )
        .image(
            user.as_ref()
                .and_then(|u| u.avatar.as_ref())
                .map(|a| a.url.clone()),
        );
        let view = cx.entity().downgrade();
        let sign_out =
            Button::new("sign-out")
                .small()
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
