//! The design system's tokens in code: the one place that holds the app's
//! colours, type roles and named sizes. The system itself is the
//! `hardcover-design` skill (`.claude/skills/hardcover-design`); this module
//! follows its "Tokens" and "Implementing in GPUI" sections.
//!
//! - [`color`]: the paper-and-ink palette for both themes, including the
//!   app-only roles the kit's theme has no field for (gold, star-empty,
//!   fg-faint, track, surface, the overlays, the cloth colours, the cover's
//!   edge, hinge and shadow). Read it with [`colors`].
//! - [`theme`]: maps the palette onto gpui-component's theme, so the kit's
//!   own components (buttons, inputs, lists, menus, switches, tabs, dialogs)
//!   draw in it.
//! - [`type_roles`]: the type roles as style helpers (`.page_title()`,
//!   `.body_strong()`, `.display_24()`, `.tabular()`, ...).
//! - [`size`]: the named sizes: control heights, chip, rail, list and sidebar
//!   widths, meters, prose. Cover sizes are on [`crate::cover::CoverSize`].
//!
//! Nothing else in the app holds a raw colour. Data colours (a cover image,
//! a reviewer's Hardcover colour) are not UI colours and stay where they are
//! read.

pub mod color;
pub mod size;
pub mod theme;
pub mod type_roles;

pub use color::colors;
pub use type_roles::{TypeRole, TypeRoles};
