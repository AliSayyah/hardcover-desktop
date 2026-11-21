//! The named sizes (`hardcover-design`, Tokens → Frames and sizes), in rems
//! at the system's 16 px rem. Use a GPUI helper where one exists for the
//! step (`h_8`, `w_96`) and these where the size has a name of its own.
//! Cover sizes are [`crate::cover::CoverSize`].

use gpui_kit::{Pixels, Rems, px, rems};

/// A control: button, input, select, nav row, menu item (32; `h_8`).
pub const CONTROL: Rems = rems(2.);
/// A small control, inside a row, panel or rail (24; `h_6`).
pub const CONTROL_SMALL: Rems = rems(1.5);
/// A chip (20; `h_5`).
pub const CHIP: Rems = rems(1.25);
/// A tall row: the brand, the sidebar goal (40; `h_10`).
pub const ROW_TALL: Rems = rems(2.5);
/// The object bar over a book or page: its tools, Back (56).
pub const OBJECT_BAR: Rems = rems(3.5);

/// The sidebar (240).
pub const SIDEBAR: Rems = rems(15.);
/// The shelf list, by default; it is resizable (384; `w_96`).
pub const LIST: Rems = rems(24.);
/// The narrowest the shelf or search list gets (272): its header's filter
/// at [`FILTER_MIN`] beside a small select, 8 apart, inside the 16 inset.
/// With [`PANE_MIN`] and the sidebar it fills the 800 px minimum window.
pub const LIST_MIN: Rems = rems(17.);
/// The widest the shelf or search list gets (640).
pub const LIST_MAX: Rems = rems(40.);
/// The narrowest the book pane beside a list gets (288): Details' key
/// column beside a small select.
pub const PANE_MIN: Rems = rems(18.);
/// The least a filter field keeps when its row narrows (104): the sort
/// beside it gives way first, down to [`SELECT_SMALL`].
pub const FILTER_MIN: Rems = rems(6.5);
/// The rail on Home and the full book page (272).
pub const RAIL: Rems = rems(17.);
/// The key column of a property list (96; `w_24`).
pub const KEY: Rems = rems(6.);
/// The key column of a property list in a rail (72).
pub const KEY_IN_RAIL: Rems = rems(4.5);
/// A select (168).
pub const SELECT: Rems = rems(10.5);
/// A small select (128; `w_32`).
pub const SELECT_SMALL: Rems = rems(8.);
/// A list row's trailing lane, kept even when empty (48; `w_12`).
pub const TRAILING_LANE: Rems = rems(3.);
/// An upcoming row's action lane, kept either way (136).
pub const ACTION_LANE: Rems = rems(8.5);
/// A series row's next-step lane, kept on every row so the lanes line up
/// (240).
pub const NEXT_LANE: Rems = rems(15.);
/// One book in a series strip: an sm cover with its marks under it (40).
pub const STRIP_ITEM: Rems = rems(2.5);
/// The widest the Series destination's column gets (896).
pub const SERIES_COLUMN: Rems = rems(56.);

/// A meter with words beside it (6).
pub const METER: Rems = rems(0.375);
/// A meter inside a row (2).
pub const METER_THIN: Rems = rems(0.125);
/// The widest a meter gets (400).
pub const METER_MAX: Rems = rems(25.);
/// A meter in a row (160).
pub const METER_IN_ROW: Rems = rems(10.);
/// The goal meter's pace tick: 2 wide, 3 proud of the meter on each side.
pub const PACE_TICK_WIDTH: Pixels = px(2.);
pub const PACE_TICK_OVERHANG: Pixels = px(3.);

/// The widest a page's content gets (768): Coming soon, Discover, Stats (in
/// one column), list, series and author pages, and the book pane's content.
/// The page inset is outside it.
pub const PAGE_MAX: Rems = rems(48.);
/// A page's inset (32; `p_8`).
pub const PAGE_INSET: Rems = rems(2.);
/// A page's frame at its widest: [`PAGE_MAX`] and the inset on both sides.
pub const PAGE_FRAME_MAX: Rems = rems(PAGE_MAX.0 + 2. * PAGE_INSET.0);
/// The widest Home's content gets in two columns (1024), its inset
/// included: beyond it the rows' actions drift away from their books. A
/// wider region gives Home a third column instead ([`HOME_WIDE_MAX`]).
pub const HOME_MAX: Rems = rems(64.);
/// The narrowest a page's main column gets beside its rail (400): a lead
/// book's xl cover with its title and buttons, or a row's cover, text and
/// actions. Narrower, the rail folds under it.
pub const MAIN_MIN: Rems = rems(25.);
/// The narrowest a book header's info column gets beside the cover (192).
pub const HEADER_INFO_MIN: Rems = rems(12.);
/// The widest Back's label gets in the object bar (320).
pub const BACK_LABEL_MAX: Rems = rems(20.);
/// The book pane's inset (24; `px_6`).
pub const PANE_INSET: Rems = rems(1.5);
/// The book pane at its widest: [`PAGE_MAX`] and the pane's inset on both
/// sides. Its object bar keeps to it, so the tools stay over the content.
pub const PANE_FRAME_MAX: Rems = rems(PAGE_MAX.0 + 2. * PANE_INSET.0);
/// The least room a page keeps for its list under a header that scrolls
/// (224): three rows and the list's foot.
pub const PAGE_LIST_MIN: Rems = rems(14.);

/// A chart on a page (192).
pub const CHART: Rems = rems(12.);
/// A short chart on a page, such as a rating spread (160).
pub const CHART_SHORT: Rems = rems(10.);
/// A headline figure's column at its narrowest (160): Stats' figures, four
/// to a row at the page's width and two in a narrow window.
pub const FIGURE: Rems = rems(10.);

// ----- Wide windows: surplus width holds more content, never wider rows -----

/// The narrowest a content column gets beside another (448): two of them
/// and the [`RAIL`] need a 1536 window, so the 1200 default keeps two
/// columns. [`MAIN_MIN`] is too tight here: the lead book's buttons overrun
/// it, and an upcoming row's title has little room beside its date and lane.
pub const WIDE_COLUMN_MIN: Rems = rems(28.);
/// The widest a content column gets beside another (720): Home's two
/// content columns in its wide tier. Its rows' actions stay by their books.
pub const COLUMN_MAX: Rems = rems(45.);
/// The widest Home's content gets in three columns (1840), its inset
/// included: two [`COLUMN_MAX`] columns and the [`RAIL`], 32 apart. Beyond
/// it, paper.
pub const HOME_WIDE_MAX: Rems = rems(2. * COLUMN_MAX.0 + 2. * 2. + RAIL.0 + 2. * PAGE_INSET.0);

/// The widest prose and readers' words get (640).
pub const PROSE: Rems = rems(40.);
/// The widest an empty state's sentence gets (288; `w_72`).
pub const EMPTY_TEXT: Rems = rems(18.);

/// A menu's reason row (288): why its items are disabled ("… no
/// permission …"), wrapped at this width. The kit's menu labels don't wrap.
pub const MENU_REASON: Rems = rems(18.);

/// A short number field: a reading goal's target (112).
pub const NUMBER_FIELD: Rems = rems(7.);
/// The tallest a list inside a dialog gets: the edition picker's (384).
pub const DIALOG_LIST_MAX: Rems = rems(24.);
/// The tallest the goal dialog's finished books get (132): five and a half
/// rows of 24, so the half row says the list scrolls.
pub const DIALOG_PEEK_LIST_MAX: Rems = rems(8.25);
/// The tallest the release notes get in the update dialog (288).
pub const DIALOG_NOTES_MAX: Rems = rems(18.);

/// A dialog that confirms (24 rem).
pub const DIALOG_CONFIRM: Rems = rems(24.);
/// A dialog with a form (30 rem).
pub const DIALOG_FORM: Rems = rems(30.);

/// The 1 px hairline.
pub const HAIRLINE: Pixels = px(1.);
/// The focus ring drawn outside a focused control (3; `ring`).
pub const FOCUS_RING: Pixels = px(3.);
/// The 1 px accent edge of a focused control or row.
pub const FOCUS_EDGE: Pixels = px(1.);

/// An avatar beside a review or in the sidebar (32; `size_8`).
pub const AVATAR: Rems = rems(2.);
/// A person's avatar where a book has its cover (an author's page; 128).
pub const AVATAR_LARGE: Rems = rems(8.);
