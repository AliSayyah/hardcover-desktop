# Hardcover Desktop design system

The reference for every visible part of the app. The words **must** and
**never** mark rules. **Should** is the default; a concrete reason is needed
to depart from it, and the reason belongs in the code comment or the commit.
The mockup at `docs/design/mockup.html` shows every rule here applied.

## Thesis

Hardcover Desktop is a quiet room for someone's books. Content leads, and the
interface stays out of the way until it is needed.

1. **Serif speaks for the book, sans speaks for the app.** A book's title, a
   description, the greeting and big figures are set in Literata. Every
   control, label, count and message is set in Inter. You can tell at a
   glance what is the book and what is the tool.
2. **Paper and ink.**
   - The surfaces are warm off-white, or warm near-black in dark mode, and
     the text is warm ink.
   - Colour is scarce, so it means something. Ink blue is the one accent.
     Gold appears only on your own rating.
   - The covers bring the rest of the colour.
3. **Covers are books.** A cover is the most recognisable thing about a book.
   Covers are always 2:3, never cropped to a square, and drawn like a small
   object. A book without an image gets a cloth cover, not an icon.
4. **One focal point per surface.** On Home it is the book you are reading.
   On a shelf it is the list. On a book it is the cover, the title and your
   progress. If you squint at a surface, its focal point and next action
   must still read.
5. **Structure before decoration.** Spacing, alignment, hairlines and type
   carry the hierarchy. Boxes, shadows and colour come last, and only where
   they mean something.
6. **Desktop first.** Persistent navigation, keyboard paths, resizable panes,
   and real menus.

## Tokens

Tokens are the only source of values. In code:
- the gpui-component theme carries the colours the kit's components read;
- app-only roles (gold, cloth, the cover's edge, hinge and shadow, star-empty)
  live in one app module;
- sizes use GPUI's rem helpers, at a 16 px rem. The helpers exist for the
  Tailwind steps (`_0p5` to `_12`, then `_16`, `_20`, `_24`, `_32`, `_40`,
  `_48`, `_56`, `_64`, `_72`, `_80`, `_96`); any other size is `rems(..)`.
- **Every size in this system is in design px.** On a large monitor the app
  scales its rem (`ui_scale`): it draws the interface up to 1.5 times the
  design, in eighths, so a logical pixel looks no smaller than on a 24"
  1080p monitor at scale 1 (a 27" 4K monitor at 1.5 gets 1.25). Whatever is
  built from rems follows; raw `px(...)` doesn't, so it is kept for what
  must stay one device-independent pixel (see "Implementing in GPUI"). The
  user can't set the factor; `HARDCOVER_UI_SCALE` forces one.

The mapping to theme fields is under "Implementing in GPUI".

### Colour

The fills `hover`, `selected` and `chip` are **ink overlays**: translucent ink
over whatever surface they sit on. Because of that they step the same way on
paper, the sidebar, a panel or a control. A chip stays visible on a hovered or
selected row.

| Role | Light | Dark | Use |
| --- | --- | --- | --- |
| `bg` | `#FAF8F4` | `#171614` | Paper: the work area |
| `sidebar` | `#F1EEE7` | `#100F0E` | The sidebar |
| `surface` | `#F3F0E9` | `#1E1C1A` | The one inset panel in a pane (progress, Not started) |
| `raised` | `#FFFFFF` | `#262320` | Controls, inputs, popovers, dialogs |
| `hover` | ink 5% `rgba(31,28,23,.05)` | ink 5% `rgba(238,234,227,.05)` | Pointer over a row, nav item, menu item or ghost button |
| `selected` | ink 10% | ink 10% | The current row or destination; a pressed control |
| `chip` | ink 7% | ink 8% | Chip fill |
| `skeleton` | `#E9E4DA` | `#2B2825` | Something still loading (about 1.2:1 on paper in both themes) |
| `border` | `#E4DED3` | `#292623` | Hairlines between regions and sections |
| `border-strong` | `#D0C8BA` | `#3D3934` | Control outlines |
| `track` | `#E0D9CC` | `#35312C` | The empty part of a meter; an off switch |
| `switch-thumb` | `#FFFFFF` (`raised`) | `#EEEAE3` (`fg`) | A switch's thumb, on `track` and on `accent`. In dark mode a `raised` thumb on `track` is 1.2:1, and gone once a disabled switch fades its track |
| `fg` | `#1F1C17` | `#EEEAE3` | Ink: text |
| `fg-muted` | `#5F574B` | `#ABA498` | Secondary text that carries information: authors, facts, counts, labels |
| `fg-faint` | `#857D70` | `#807A6F` | Placeholders, disabled hints, empty-state icons. Nothing a reader must read |
| `accent` | `#3B4A9C` | `#98A5F4` | Ink blue (details below the table) |
| `accent-fg` | `#FFFFFF` | `#12142A` | Text on accent |
| `accent-soft` | accent 10% | accent 14% | The avatar's fill |
| `ring` | accent 50% | accent 50% | The 2 px focus ring |
| `gold` | `#F59E0B` (amber 500) | `#FBBF24` (amber 400) | The user's own rating, nothing else |
| `gold-preview` | gold 55% | gold 55% | The rating control's hover preview: the stars a click would set |
| `star-empty` | ink 14% | ink 14% | An unfilled star |
| `danger` | `#B42318` | `#F97066` | Destructive commands and errors |
| `warning` | `#B54708` | `#FDB022` | A content warning's icon; warnings |
| `cover-edge` | `rgba(30,20,10,.16)` | `rgba(255,255,255,.12)` | The cover's 1 px edge |
| `cover-hinge` | `rgba(30,20,10,.16)` | `rgba(0,0,0,.28)` | The hinge gradient's dark stop |
| `cloth-ink` | `#F4ECDD` | `#F4ECDD` | Type and stamp on a cloth cover |

Accent is used on progress, the primary commit, the focus ring, the selected
sidebar icon, links that open the browser, the brand icon, and a genre
chip's icon. Nothing else.

Contrast, checked over every surface alone, hovered and selected:
- `fg-muted` is at least 4.7:1 in both themes (WCAG AA), so it may carry
  information;
- `fg-faint` is about 3.7:1, so it must never carry anything a reader needs;
- accent on paper is at least 5:1, and on a chip at least 5.3:1.

A new colour role needs the same check. The mockup's console audit does it.

**Cloth colours** are eight book-cloth tones, the same in both themes.
cloth-ink on each is at least 5:1.

`#6A4A39` `#3A574B` `#414C6B` `#743838` `#7A5D30` `#39596A` `#594567` `#4E5356`

Rules:
- No raw colour outside the theme and the app's token module. Covers, cloth
  and a reviewer's avatar colour from Hardcover (`cached_image.color`) are
  data, not UI colour.
- Never use a status colour as decoration, and never encode meaning by colour
  alone.
- Check every new surface in both themes.

### Type

Two families with fixed roles, and seven sizes: 12, 14, 16, 18, 20, 24, 30.
Every size has a fixed line height.

| Role | Family, weight | Size / line | In code | Use |
| --- | --- | --- | --- | --- |
| `page-title` | Inter 600 | 20 / 28 | `.page_title()` | A destination's title: "Currently reading" |
| `section` | Inter 600 | 16 / 24 | `.section()` | A section on a page: "Continue reading" |
| `byline` | Inter 500 | 16 / 24 | `.byline()` | A book's author and series under its title in the header |
| `heading` | Inter 600 | 14 / 20 | `.heading()` | A block in a pane: "About this book", "Details" |
| `body` | Inter 400 | 14 / 20 | `.body()` | Everything by default |
| `body-strong` | Inter 500 | 14 / 20 | `.body_strong()` | Row titles, values that lead |
| `label` | Inter 500, muted | 12 / 16 | `.label(cx)` | Sidebar and rail section labels: "Reading goal", "On Hardcover" |
| `caption` | Inter 400, muted | 12 / 16 | `.caption(cx)` | Counts, trailing figures, timestamps, helper text |
| `display-30` | Literata Display 600 | 30 / 36 | `.display_30()` | Book title on the full page; the Home greeting; figures ("9", "4.63") |
| `display-24` | Literata Display 600 | 24 / 30 | `.display_24()` | Book title in the pane and the Home lead |
| `title-18` | Literata Text 600 | 18 / 24 | `.title_18()` | Book titles in Home rows and Up next |
| `prose` | Literata Text 400 | 16 / 26 | `.prose()` | Descriptions; at most 640 px wide |
| `reader` | Inter 400 | 14 / 22 | `.reader()` | Readers' words: your review, other readers' reviews, notes that run long; at most 640 px wide, 8 between paragraphs |

The role helpers are `design::TypeRoles`; each sets the family, its
Persian fallback, the weight, the size and the line height. Never use
GPUI's own size helpers (`text_sm`, `text_xl` and the rest): they set
GPUI's line heights and no family. `.tabular()` adds tabular figures.

- Literata comes in two optical cuts, registered as two families: **Text**
  (the 12 pt cut) up to 18 px, and **Display** (the 36 pt cut) from 24 px.
  Both are static faces.
- No letter-spacing anywhere: GPUI has none, and the Display cut does the
  tightening.
- Text of different sizes in one row shares a baseline: a count beside a
  page title, "of 12 books" beside a display figure, a sidebar count beside
  its label. In GPUI, centre the row and put each smaller run on the lead's
  baseline with `baseline_on(TypeRole::…, lead)`; `items_baseline()` aligns
  the bottoms of the line boxes instead, because GPUI's text reports no
  baseline to the layout.
- No all-caps labels. Hierarchy comes from size, weight and colour.
- Use tabular figures wherever numbers stack or change in place:
  percentages, page counts, counts in the sidebar, dates in lists. Use lining
  figures in display sizes.
- Don't put the serif on controls, labels or messages, and don't put Inter on
  a book title of 18 px and up.
- Book titles in list rows are `body-strong` Inter. The serif starts at
  title size.
- The serif is the book's voice: its title and its description. Reviews and
  notes are readers' voices, so they are set in Inter (`reader`), never in
  the serif.
- Persian and Arabic have no serif. Literata has no Arabic script, so
  right-to-left text is set in Vazirmatn wherever it appears, at the role's
  size and weight. A book title or description keeps its serif role in
  every other respect (size, line height, position).
- Inter is bundled with its static faces (`fonts.rs`). Never fall back to
  the system UI font: on Linux it is often variable, and GPUI draws a
  variable font's default instance only.

### Space

One scale, named by the relationship it expresses.

| Token | px | GPUI | Relationship |
| --- | --- | --- | --- |
| `s-2` | 2 | `gap_0p5` | Optical; gap between list rows; stars |
| `s-4` | 4 | `gap_1` | Parts of one control; title to author |
| `s-6` | 6 | `gap_1p5` | Chips in a wrapped group, both ways; a tag group's label to its chips |
| `s-8` | 8 | `gap_2` | Closely related controls; icon to label; sidebar inset |
| `s-12` | 12 | `gap_3` | One group: cover to text in a row; list header |
| `s-16` | 16 | `gap_4` | Groups in a section; panel and list inset |
| `s-24` | 24 | `gap_6` | Sections; the pane's inset; between blocks |
| `s-32` | 32 | `gap_8` | Regions; the page and Home inset; Home's column gap |

- **Insets:** sidebar 8, list 16, book pane 24, page and Home 32.
- **Gaps:** row gap 2; block gap 24.
- A component's padding belongs to it, and the gap between components belongs
  to their parent. Don't double padding.
- Equal relationships use equal gaps, and a related pair sits closer than
  separate groups. A one-rendered-pixel difference between two gaps that are
  meant to be equal is a defect.

### Radius

| Token | px | Use |
| --- | --- | --- |
| `r-cover-s` | 2 | Covers xs and sm |
| `r-cover` | 3 | Covers md and up (books are nearly square-cornered) |
| `r-control` | 6 (`theme.radius`) | Buttons, inputs, selects, nav rows, menu items |
| `r-box` | 8 (`theme.radius_lg`) | List rows, panels, popovers, dialogs |
| `r-pill` | full | Chips, meters, switches, the avatar |

A rounded parent does not clip its children in GPUI. Give an image, a
selection fill or a nested surface its own radius.

### Frames and sizes

| What | Size | GPUI |
| --- | --- | --- |
| Control (button, input, select, nav row, menu item) | 32 high | `h_8` |
| Small control (inside a row, panel or rail) | 24 high | `h_6` |
| Chip | 20 high | `h_5` |
| Tall row (brand, sidebar goal) | 40 high | `h_10` |
| Object bar (book tools, Back) | 56 high | `h(rems(3.5))` |
| Icons | 16; 12 in small controls and chips; 24 in empty states | `size_4`, `size_3`, `size_6` |
| Sidebar | 240 wide | `w(rems(15.))` |
| Shelf list | 384 wide by default, resizable from 272 (its filter, a small select and the inset) to 640 | `size::LIST`, `LIST_MIN`, `LIST_MAX` |
| Rail (Home, full book page) | 272 wide | `w(rems(17.))` |
| Next lane (a series row) | 240 wide | `size::NEXT_LANE` |
| Strip item (a series row) | 40 wide | `size::STRIP_ITEM` |
| Key column in a property list | 96; 72 in a rail | `w_24`, `w(rems(4.5))` |
| Select | 168; small 128 | `size::SELECT`, `size::SELECT_SMALL` |
| Meter | 6 high beside words, 2 high inside a row; 400 wide max (160 in a row) | |
| Prose | 640 wide max | `size::PROSE` |
| Page content (Discover, Coming soon, Stats, Series, list, series and author pages) | fills the region inside the 32 inset; only reading text keeps the prose measure | `size::PAGE_INSET` |
| A catalogue page's list beside the book pane | 704: the prose measure and the inset; the pane takes the rest | `size::PAGE_BESIDE_PANE` |
| Book pane and full book page | fill their region | |
| Side column beside a book's text | at least 320 wide; the text keeps 640, 32 apart | `size::SIDE_MIN`, `size::PROSE` |
| Home | fills the region | |
| Content column beside another (Home, wide) | at least 448 wide; the two share the width beside the rail | `size::WIDE_COLUMN_MIN` |
| Stats, two columns | each column at least 544, filling its half | `size::CHART_COLUMN_MIN` |
| Chart | bars at most 48 wide; a year in Finished per year at least 64 | `size::CHART_BAR_MAX`, `CHART_YEAR_MIN` |
| Empty-state text | 288 wide max | `w_72` |
| Window | 1200 × 800 by default, 800 × 560 minimum | |

### Elevation

There are three levels and no others:
1. **Flat.** Every region, panel, row and card. Use a hairline or a fill,
   never a shadow.
2. **Cover.** `shadow-cover` on a cover that leads its row or block.
   - Light: `0 1px 2px rgba(40,28,16,.18), 0 8px 20px rgba(40,28,16,.14)`
   - Dark: `0 1px 2px rgba(0,0,0,.55), 0 10px 24px rgba(0,0,0,.45)`
3. **Popover.** Menus, popovers and dialogs: `raised` with `shadow-popover`.
   - Light: `0 0 0 1px rgba(30,20,10,.10), 0 4px 12px rgba(40,28,16,.12)`
   - Dark: `0 0 0 1px rgba(255,255,255,.08), 0 4px 12px rgba(0,0,0,.5)`

## Covers

There is one cover component. Never draw a cover box by hand.

Rules:
- **Shape:** always 2:3. The image fills it with `object-fit: cover`.
- **Corners:** `r-cover-s` for xs and sm, `r-cover` for md and up. The image
  is clipped to them.
- **Edge:** a 1 px inside border in `cover-edge` over the image, at every
  size, so a white cover doesn't melt into paper.
- **Hinge**, from md up: a strip 6% of the width on the spine (leading)
  side, a linear gradient from `cover-hinge` to clear.
- **Shadow**, from md up: `shadow-cover` where the cover leads its row or
  block. That means Home's reading rows, the book header, Up next and
  Similar books. A row of small thumbnails in a rail doesn't get it, and xs
  and sm never do.
- **Loading:** a `skeleton` fill with the edge, until the image lands.
- **No image:** the cloth cover.
- **Click:** opens the book. The cover is never the only keyboard path to
  the book; its title is a button too.

| Size | Width × height | GPUI | Use |
| --- | --- | --- | --- |
| xs | 20 × 30 | `w_5` × `h(rems(1.875))` | Compact rows, menus |
| sm | 32 × 48 | `w_8 h_12` | List rows (shelves, search, Discover, lists, series and author pages) |
| md | 64 × 96 | `w_16 h_24` | Home rows, Up next, a rail's shelf |
| lg | 80 × 120 | `w_20` × `h(rems(7.5))` | Similar books |
| xl | 128 × 192 | `w_32 h_48` | The book pane's header; the Home lead |
| xxl | 160 × 240 | `w_40` × `h(rems(15.))` | The full book page's header |

**The cloth cover** stands in for a missing image, like a bound book with no
dust jacket.
- **Colour:** one of the eight cloth colours, picked by a stable hash of the
  title (`h = 7; for each code point c: h = h * 31 + c (mod 2^32)`, index
  `h % 8`). The same book always gets the same cloth.
- **Type:** cloth-ink, in Literata Text 600 for the title. Its size scales
  with the cover: xs 10, sm 16, md 9, lg 11, xl 18, xxl 22 design px. The
  padding, the frame's inset and the hinge are design px too, so the cloth
  scales with the interface; the frame and the rules stay 1 px. It stands in
  for an image, so it is the one place off the type scale.
- **Content by size:**
  - xs: the title's first letter, skipping a leading "The " or "Untitled "
    (Hardcover's placeholder titles for unannounced books, such as "Untitled
    Empyrean #4");
  - sm: that letter and a short rule under it;
  - md: a blind-stamped frame (inset 5%, 1 px cloth-ink at 28%), the title
    (at most four lines), and a rule (24% wide, 60%);
  - lg and up: all of md, plus the author at the foot, at 0.8× the title
    size and 85% opacity.
- **Padding:** 16% top, 13% sides and bottom.
- **Right-to-left titles:** set right to left in Vazirmatn 600, at a 1.5
  line height.

## Components

### Buttons and links

| Kind | Look | Use |
| --- | --- | --- |
| Default | `raised`, 1 px `border-strong`, ink, 14/500 | Ordinary visible commands: "Update progress…", "Choose…", "Start reading" |
| Ghost | No fill or border | Tools and quiet secondary commands: "Lists", "Mark as read…", "Edit", Back |
| Primary | `accent` fill, `accent-fg` | The default commit of a dialog or form (the Enter action): "Save", "Create". Never "the important one" on a page |
| Danger | Danger text, no fill | Destructive commands; the confirm button of a destructive dialog |
| Icon | 32 square, ghost | Familiar symbols only ("more"), with an accessible name and a tooltip |
| Text button | Ink text, underline on hover only | In-app navigation inside content: an author, a series, a book title |
| Link | Accent, underlined, a 12 px external-link icon | Only for things that open the browser: "Open on Hardcover" on a series, author or list page (a book's is in its "more" menu) |
| Byline link | Ink, 14/500, underline on hover, pointing hand | A reviewer's name, which opens their Hardcover profile in the browser. It repeats on every review, so it stays quiet; a right-to-left name goes without the underline, which GPUI on Linux draws beside it |

- **Sizes:** default 32 (padding 12, icon 16, gap 8); small 24 (padding 8,
  12 px text, icon 12, gap 4). Use small inside rows, panels and rails. In
  code a small button or select is `.small_control()` (`ui::kit`), an
  icon-only one `.small_icon_button()`; never the kit's `.small()`, which
  sets 14 px text.
- **Menu buttons:** a button that owns a menu ends with a 12 px chevron, and
  stays pressed while the menu is open. The status button shows the status's
  icon.
- **Ellipsis:** a command that opens a dialog, or needs more input, ends
  with `…`.

### Inputs

- Input and select are 32 high, `raised`, with a 1 px `border-strong` and
  `r-control`.
  - Placeholder: `fg-faint`.
  - A leading icon: 16 px, `fg-muted`.
  - A small select is 24 high.
- A switch is 28 × 16 (the kit's `.small()`): `track` when off, `accent`
  when on, with a `switch-thumb` thumb (`raised` in light mode, `fg` in
  dark).
- A setting fixed from outside the app (an environment variable) shows its
  value as text with the reason under it, not as a control the user can't
  operate.
- A field's label is visible and its help sits next to it. In property lists
  the key column holds the label.
- A field whose unit the reader chooses ("Update progress…": Page or Time,
  and %) has a small segmented control before it, the book's own unit
  first. What the input means goes after the field in `fg-muted`, tabular
  ("Page 200 of 476", "6h 25m of 16h 10m"). A unit that can't be offered is
  left out rather than disabled (Kit limits), and a caption under the
  field says why.

### Chips (`chip.rs`)

A chip is a neutral pill:
- 20 high, `chip` fill, `fg-muted` text at 12/500;
- padding 8, gap 4, an optional 12 px icon.

The two kinds:
- **StatusChip:** a book's library status at the end of a catalogue row, with
  its shelf's sidebar icon. The icons are book-open, inbox, circle-check,
  pause, ban and eye-off.
- **TagChip:** a catalogue tag.
  - A genre adds an icon chosen from its name, in `accent`: rocket for
    science fiction, wand-sparkles for fantasy, heart for romance, and so on
    (`chip::genre`, about 40 genres, matched on normalised words with the
    common variants: "Sci-Fi", "SF", "YA", "Children's", "Non-Fiction"). A
    genre that matches none has no icon; an icon is never guessed.
  - A content warning adds the warning icon in `warning`.
  - Moods and other tags have no icon.
  - Those two icon colours are the only colours a chip ever carries; the
    pill and its text stay neutral.
  - The icon sits on the chip's leading (left) side, a right-to-left tag
    too: the interface isn't mirrored.
- **Wrapped chips** (`TagRow`): chips that wrap across a width sit 6 apart
  both ways (`s-6`), in the header and in the Tags block alike.

Never colour-code chips, and never use a badge for ordinary metadata.

### Stars (`star.rs`)

- The app's own rounded solid star. Sizes: 20 for the rating control, 16
  in rows, 12 inline; gap 2.
- **Your rating:** `gold` fill.
- **Hardcover's figures** (the average, the breakdown): the same star in
  `fg-muted`.
- **An empty star:** the same shape in `star-empty`, so a half star splits
  cleanly.
- An unrated rating control reads "Rate" beside it, in `fg-muted`.
- **Hover preview:** over the rating control, the stars a click would set
  show in `gold-preview` (gold 55%). It is the pointer's preview, never a
  saved rating; the keyboard changes the rating itself.
- In code: the rating control is `book::star_rating::StarRating` (20;
  `.small()` 16 in a row); a rating only shown is `ui::Stars` (16;
  `.small()` 12 inline).

### Meters

- A meter is an `accent` fill on `track`, with pill ends. It is 6 high when
  words sit beside it and 2 high inside a row.
- A goal meter adds a pace tick, where steady progress would be today: 2 px
  wide, `fg`, 3 px proud of the meter on each side.
- Words come with every meter ("Page 312 of 476", "66%"). The bar is never
  the only carrier.

### Panels, rails and hairlines

- **Panel:** `surface`, `r-box`, padding 16, no border, no shadow. A pane
  has at most one: the progress block for a book you have started, or the
  Not started block for a Want to read book.
- **Rail:** a column with a 1 px leading hairline and a 24 inset. Sections
  inside it start with a `label` and are separated by hairlines with 24
  above and below (to the label's line box). A section's small action
  ("Add a past read…") is centred on the label's line and hangs over it,
  so the label sits in the same place with or without one. The rail runs
  the page's height, to its foot on a short page. Never boxes.
- **Hairline:** 1 px `border`, drawn by exactly one of the two regions it
  separates.
- Never put a card inside a card.

### Rows

- **List row:**
  - padding 8, `r-box`, gap 12; rows sit 2 apart;
  - an sm cover, then a text column: the title in `body-strong` and the
    author or facts in `body` muted, both one line and truncated;
  - a trailing lane 48 wide, right-aligned: tabular caption figures, or a
    2 px meter under a percentage.
  - The lane is kept even when a row has nothing to put in it, so columns
    stay aligned.
- **Catalogue row** (search, Discover, series, author and list pages): the
  same, with the StatusChip trailing. In a list narrower than 360 the chip
  shrinks to its icon (`StatusIconChip`, the status in its tooltip and
  accessible name), so the title keeps its room.
- **Selectable or not.** A collection you select from (a shelf, search
  results, a list) uses list rows, with hover and selected fills. A read-only
  sequence on Home or a page (Continue reading, Coming soon, reviews) uses
  **sequence rows**: no fill, a hairline between rows (none above the
  first), 12 above and below (16 for reviews). Their titles are text buttons.
- **Upcoming row** (`upcoming/row.rs`, shared by Home and the Coming soon
  list), a sequence row:
  - an sm cover (usually a cloth cover; unreleased books rarely have one);
  - the title in `body-strong` and the series with its position, muted;
  - a date lane, right-aligned and tabular: the date in `body-strong` ("Nov
    26", "Feb 3, 2027", or the year when it is further off), with a caption
    under it when it is within two weeks ("Tomorrow", "In 6 days");
  - an action lane 136 wide: a small ghost "Want to read" with a plus icon,
    or the StatusChip when the book is already on the shelf. The lane is
    kept either way, so the dates line up.
- **Series row** (`series/row.rs`), a sequence row with 16 above and below,
  top-aligned:
  - the series name in `title-18`, a text button that opens the series page;
  - a muted line: the author, "2 of 6 read", "#6 not out yet" when a later
    book is still to come, and the activity ("Reading now", "Last finished
    Sep 28", "Caught up since Feb 2", "Paused since Jun 2, 2024", "Finished
    Jan 12, 2025");
  - the strip, 12 below;
  - the next lane, 240 wide and kept on every row so the lanes line up: an
    sm cover, a caption ("Reading · #1", "Next · #3", "Paused · #3"), the
    title as a text button, a line ("Page 418 of 1,007 · 42%", "Paused at
    page 140 of 412", or the date in `body-strong` with "· In 6 days"
    muted), then one small command: "Update progress…", "Start reading", a
    ghost "Want to read" with a plus, or the StatusChip. A finished series
    leaves the lane empty.
- **Strip** (`ui::SeriesStrip`): the books of a series in order, 4 apart,
  each 40 wide: the sm cover, a 2 px line under it (a meter for the book
  being read, else empty, so the positions sit on one line), then the
  shelf's 12 px icon and the position in a caption. The next book's position
  is ink at weight 600. A book not in the library is an empty **slot**
  (`ui::CoverSlot`: a dashed `border-strong` outline in the cover's shape,
  no fill); one not out yet is a slot with a 12 px clock. An Ignored book
  keeps its cover and the Ignored icon, and is never the next book. A strip
  that can't show every book keeps the next one in view and ends with a
  "+N" caption.
  State never relies on colour: the icon, the slot and the clock carry it.
- **Property list** (Details): rows at least 24 high, 8 apart. The key
  column (96, or 72 in a rail) is `fg-muted`; the value is ink, with small
  controls inline.

### Sidebar

- The sidebar has an 8 inset and a 1 px trailing hairline. Rows are aligned
  on three spines: icons at x 16, labels at x 40, counts on the trailing
  lane.
- **Nav row:** 32 high, padding 8, `r-control`.
  - The icon is 16, `fg-muted`.
  - The count is caption, tabular, `fg-muted`.
- **Selected row:**
  - fill: `selected`;
  - label: weight 500;
  - icon: `accent`;
  - count: `fg`.
- **Section label:** `label`, with 16 above and 4 below.
- **Brand row:** 40 high: the book-open icon in `accent` and "Hardcover" at
  16/600.
- **Goal row:** 40 high. "2026" at the label spine and "9 of 12 books" on the
  count lane, with a 2 px meter under them from the label spine to the
  trailing edge.
- **Account:** at the foot: a 32 avatar (`accent-soft` circle, accent
  initials 12/600), the username in `body-strong`, the sync time in caption,
  and a chevrons-up-down icon.
- While Ctrl is held, each destination shows its shortcut in place of its
  count (existing behaviour).

### Popovers, menus and dialogs

- **Popover and menu:** `raised`, `r-box`, `shadow-popover`, padding 4.
  - Menu items are 32 high, padding 8, `r-control`, with a 16 px icon in a
    fixed slot. They use the same hover and selected fills as rows.
  - The app has no popover of its own; its menus are the kit's, which keep
    their own radius, shadow, item height and icon size (Kit limits).
- **Dialogs:**
  - **Width:** confirmations 24 rem; forms 30 rem.
  - **Title:** the decision ("Delete “Book club picks”?").
  - **Body:** only new information.
  - **Buttons:** Cancel, then the result ("Delete" in danger; "Save" in
    primary).

### Reviews (other readers')

The book's "Reviews" block, above Similar books. It hides without
`read:social`.
- **Header:** the `heading` "Reviews" with the count as a caption on the
  trailing edge ("12 reviews").
- **Each review** is a sequence row (16 above and below), top-aligned:
  - a 32 avatar: the reader's picture through the cover pipeline, else
    their initials (12/600, white) on their Hardcover colour, else the
    kit's initials on `accent-soft`;
  - the byline link (or `@username` when the name is empty) and their
    rating in 12 px muted stars, 8 apart;
  - a caption: "Mar 14, 2026 · 48 likes";
  - the text in `reader`, collapsed to about six lines, with a small ghost
    "Read more" / "Show less";
  - a spoiler review shows only "Contains spoilers" (muted) and a small
    "Show".
- **Loading:** two skeleton reviews (an avatar circle and three bars).
- **More:** a small default "Show more reviews" after the last one.
- A right-to-left review sits on the leading edge, and its lines run right
  to left (`WrappingText`).

### Empty states

An empty state is centred in the region it explains:
- a 24 px icon in `fg-faint` (stroke 1.5);
- a `body-strong` title;
- one `fg-muted` sentence, at most 288 wide;
- optionally one button.

Use the app's own words: "No matches" / "No books in Did not finish match
“dune”." and "Select a book to see its details". An empty state is never an
alert.

## States

Each state has one treatment, on every control. Hover is never the only cue.

| State | Treatment |
| --- | --- |
| Rest | As specified for the component |
| Hover | The `hover` fill. A text button gains its underline. The rating control previews the stars a click would set in `gold-preview` |
| Pressed, selected, open | The `selected` fill. A selected nav row also takes weight 500 and the accent icon. A menu button stays pressed while its menu is open |
| Focus-visible | A 2 px `ring` outside, plus a 1 px accent edge: the border for controls, an inset edge for rows, and just outside the text for bare text (a text button, a byline link), which has no border. Never clipped by a scrolling parent |
| Disabled | 50% opacity, with no hover or pressed response. When the reason isn't obvious, say it beside the control or in its tooltip (`format::needs_permission`) |
| Loading | `skeleton` shapes where the content will be, pulsing between 100% and 50% opacity. Covers show a skeleton until the image lands. A button running its action keeps its label, shows its loading state and refuses repeat clicks. No spinner in a list |
| Error | Say what happened and how to recover, next to the thing that failed (`Services::error_message`) |

Switching between content that is already loaded is immediate: no skeleton,
no transition.

## Screens

Every screen starts from its task:

| Screen | Primary task | Focal point |
| --- | --- | --- |
| Home | Pick up where you left off | The lead book |
| Shelf | Find a book on it | The list |
| Book | Know the book and update it | Cover, title, status, progress |
| Coming soon | See what's next in your series, and want it | The dates |
| Series | See where you are in each series and what's next | The next step in each series |

### Window shell

- The sidebar (240) sits beside the destination.
- Book, series and author pages open over the destination with a Back bar.
  A sidebar destination clears them.
- **Wide window:** see "Wide windows" below. A book pane's tools sit at its
  trailing edge, over its content.
- **Short window:** when the sidebar's destinations don't all fit, a
  hairline over the account says the list scrolls under it.

### Wide windows

Rows and panels fill the width they are given. Only reading text keeps its
measure: `size::PROSE`, 640, for book descriptions and review text. The
width left beside the text holds secondary blocks, and cover shelves add
covers as they widen. A 4K monitor at 1.5 gives about 2560 logical px; a
layout that stops short of it leaves a blank strip that reads as a bug. The
1200 × 800 default and the 800 × 560 minimum are where the README's
screenshots are taken, and every screen lays out there exactly as it did
before this rule. They are design px: at an interface scale of 1.25 the
window opens at 1500 × 1000 and can't be made smaller than 1000 × 700, so
every width below (the tiers' thresholds included) is in design px too.

- **Leading edge.** Content starts on the leading edge, on the same spine as
  Back. It is never centred in the window.
- **Fill, then add.** A region's rows, panels and columns take its whole
  width. Where the width grows past what a column needs, it gains a column
  or shows more items: a cover shelf adds covers, a side column takes the
  secondary blocks beside the text.
- **Reading text keeps its measure.** A description, review text, notes and
  other running prose stop at 640, in every layout; the rows around them
  may be wider.
- **Tiers come from the region.** Each layout picks its tier from the width
  its own region gets: the window less the sidebar, less any list beside it,
  less its insets. Never from the raw window width. The choice is a pure
  function with unit tests (`ui::columns::tier`, like `folds`), pinned at
  800 and 1200 (today's layout), on both sides of each threshold, and at
  1600 and 2560.
- **Tiers by screen:**
  - **Home:** fills its region in every tier. Two content columns of at
    least 448 beside the rail (a 1536 window) give it three columns:
    Continue reading; Coming soon, Rate what you finished, then Newest on
    Want to read; the rail. The two content columns share the width beside
    the rail equally. With nothing for the second column, Home keeps two
    columns.
  - **Series:** the column fills the region. The strip takes the width,
    showing as many books as fit (45 at 2560) before it windows; the next
    lanes stay 240 and lined up on the trailing edge.
  - **Stats:** one column filling the region; two columns of at least 544,
    32 apart (a 1424 window), each filling its half: Finished per year;
    Your ratings, then Shelves. The figures fill one row across both, 32
    apart, so the third starts on the second column's spine. A chart fills
    its column, its bars widening up to 48; Finished per year gives a year
    at least 64, so a wider chart reaches further back when the library has
    the years (never fewer than 12). Shelves keeps the shelf list's 384,
    so its counts stay by their names.
  - **The book pane and the full book page:** fill their region. When the
    main column holds the 640 text and a side column of at least 320, 32
    apart (`book::detail_view::side_width`; the full page in a 1592
    window), the blocks under the panel split: About this book, Your
    review and Reviews at 640; tags, other titles and Similar books in the
    side column, filling the rest, in whole rows.
  - **Discover, list, series and author pages:** the list fills the region,
    and the page's intro line and any description keep the 640 measure.
    When the region also fits a book pane of 392 beside a 704 list (a 1337
    window), the selected book shows in the pane, which takes the rest.
  - **Coming soon** fills the region; its date and action lanes keep the
    trailing edge. **Search** is a shelf: its list keeps its resizable
    width and the pane takes the rest.

### Shelf: list plus book pane

- **List:** 384 wide by default, with a trailing hairline.
  - **Header:** padding 12/16, a hairline under it. The page title has its
    count beside it in `fg-muted`; under that, the filter input ("Filter",
    filling the width) and the sort select (168), 8 apart. As the list
    narrows, the select gives way first, down to the small 128; the filter
    keeps at least 104.
  - **Rows:** list rows, inset 8.
- **Book pane:**
  - **Object bar:** 56 high, no hairline. "Lists" (a ghost menu button with
    the gallery-vertical-end icon, the sidebar's list icon) and "more" (an
    icon button: "Open on Hardcover" for any book with a page there, then,
    for a book in the library, its library commands) sit on the trailing
    edge, over the content, which fills the pane. The book's tools live
    here, never over the title.
  - **Content:** inset 24, with 0 above. One column; when the pane fits a
    main column of 400 beside the rail, the full page's columns with the
    pane's xl header and inset.
- **Header:** an xl cover beside the info column, 24 apart. The info column
  has, top to bottom:
  - the title in `display-24`;
  - the author: a text button at 16;
  - the series ("The Stormlight Archive #1"): a text button;
  - the facts: muted and tabular ("2021 · 476 pages");
  - Hardcover's average: a 12 px muted star and "4.51 average · 9,223
    ratings";
  - the genres: genre TagChips (with their icons), wrapped 6 apart;
  - at the foot of the cover, the action row: the status menu button, then
    the rating control (20 px stars and "Rate"), 16 apart.
- **Then one panel, 24 below:**
  - **Progress** (Currently reading, Paused): "Page 312 of 476" in
    `body-strong` with "66%" muted at the trailing edge; a 6 px meter; then a
    muted line ("Started Nov 1 · 16 pages a day · done around Dec 1") and a
    small "Update progress…".
  - **Not started** (Want to read): "Not started" / "Starting moves it to
    Currently reading." and "Start reading".
- **Then blocks, 24 apart,** each with a `heading` and an optional small
  action on the trailing edge: Notes, About this book (`prose`), and Details
  (a property list: Edition, Owned, Privacy, Added). The other existing
  blocks (your review, reads, lists, series, other readers' reviews,
  similar books) follow the same pattern.
- **Tags:** the genres past the header's first six, then moods, content
  warnings and other tags, each a group, 16 apart: the group's name
  ("Content warnings") in `label`, and 6 below it its chips, wrapped 6 apart
  across the block's width. Each group shows six until the block's small
  ghost "Show N more" (then "Show fewer").

### Full book page (over Discover, search, a series or an author)

- **Object bar:** 56 high, with a hairline under it. Back sits on the
  leading edge (a ghost button with arrow-left and the destination's name);
  the book's tools sit on the trailing edge.
- **Layout:** inset 32. A main column and a 272 rail, 24 apart.
- **Main column:**
  - the header with an xxl cover and the title in `display-30`;
  - the progress or Not started panel;
  - About this book;
  - Your review;
  - Reviews (other readers');
  - Similar books: lg covers with shadow, 16 apart, with titles at 12/500 in
    at most two lines. Eight, wrapping.
- **Wide main column:** when it holds the 640 text and a 320 side column,
  32 apart, the blocks under the panel split. The text column, exactly 640:
  About this book, Your review, Reviews. The side column, filling the rest:
  Tags, other titles, then Similar books as a grid of lg covers (rows 24
  apart), two rows of as many as fit (at least eight, at most the sixteen
  fetched).
- **Rail:**
  - On Hardcover: the average as a `display-30` figure with muted 16 px
    stars; "4,280 ratings · 10,144 readers"; the ratings breakdown
    ("Open on Hardcover" is in the "more" menu);
  - Details (key column 72);
  - Reads: a label, plus a small ghost "Add a past read…" on the trailing
    edge;
  - Notes.
- The rail starts at the top, beside the header, so the page has no empty
  band.

### Home

- **Header:** inset 32. The date in `fg-muted` ("Friday, November 20"), then
  the greeting in `display-30` ("Good morning", "Good afternoon" or "Good
  evening", from the local time).
- **Columns:** a main column and a 272 rail, 32 apart, 24 below the greeting.
- **Main column:** "Continue reading" (`section`), then:
  - **The lead:** the most recently updated Currently reading book.
    - An xl cover with shadow, the title in `display-24`, and the author
      muted.
    - A progress block at most 400 wide: page and percentage, a 6 px meter,
      the pace line.
    - Then "Update progress…" (default) and "Mark as read…" (ghost).
  - **The other books being read**, as rows separated by hairlines (16
    above and below):
    - an md cover with shadow;
    - the title in `title-18`; the author muted;
    - a 160-wide 2 px meter with the percentage;
    - a caption ("Page 418 of 1,007 · 13 pages a day · done around Jan 4",
      or "Last updated 8 days ago");
    - small "Update progress…" and ghost "Mark as read…" stacked on the
      trailing edge.
  - **The latest Paused book:** an sm row with a Paused StatusChip.
  - **"Rate what you finished"** follows, when there is anything to rate.
    It isn't in the mockup. Use the same rows with an sm cover, and put the
    16 px rating control in the trailing lane.
  - **"Coming soon"** closes the column, 24 below: the `section` title with
    a small ghost "See all 5" on the trailing edge, then up to five upcoming
    rows due in the coming 12 months. With none due but some further off,
    one muted sentence says so ("Nothing is due in the next 12 months. 2
    books are further off.") and "See all" stays. It hides when nothing is
    coming.
- **Rail:**
  - **Reading goal:** a `label` ("2026 reading goal"); the figure ("9" in
    `display-30` with "of 12 books" muted); a goal meter with its pace tick;
    one sentence ("2 behind pace, 3 to go in 41 days. *Project Hail Mary*
    should finish in time.", with the title in ink 500).
  - A hairline.
  - **Up next:** an md cover with shadow; "Next in Dune · #2" as a caption;
    the title in `title-18`; the author; a small "Start reading".
  - A hairline.
  - **Newest on Want to read:** three md covers without shadow, 16 apart,
    with titles at 12 px in two lines; the section's `label` has a small
    ghost "See all 7" on the trailing edge.
- Sections that don't apply (no goal, missing scope) disappear without
  leaving a gap.
- **Narrow window:** when the main column would get less than 400 beside
  the rail, the rail's sections follow the main column, full width, under
  a hairline (`ui::Columns`).
- **Width:** Home fills its region; the rows fill their columns.
- **Wide window:** when two content columns of at least 448 fit beside the
  rail, "Coming soon", "Rate what you finished" and "Newest on Want to read"
  move to a second column between the main column and the rail, 24 apart;
  the two content columns share the width (`Columns::second`; see "Wide
  windows"). There Newest on Want to read is a `section` with "See all N"
  and one row of md covers with their titles, as many as the column holds,
  never a second row.

### Coming soon (the full list, over Home)

- **Object bar:** 56 high with a hairline; Back ("Home") on the leading edge.
- **Content:** inset 32, filling the region. The `page-title` "Coming
  soon", then a muted line ("5 books still to come in the series on your
  shelves").
- **Groups:** a `label` per month for the coming 12 months ("November",
  "February 2027"), then per year ("Later in 2027", "2028"), with 24 above
  and 4 below. Under each, upcoming rows.
- **States:** a failed load says so in one muted sentence with "Try again";
  an empty list uses the empty state.

### Series (a destination)

- **Sidebar:** "Series" after Discover, with the `library` icon and the
  count of series not finished. It hides, with the page, without
  `read:catalog`.
- **Content:** inset 32, filling the region (see "Wide windows").
- **Header:** the `page-title` "Series" with the count shown beside it,
  then the muted line "The series you've started, and what's next in each."
  On the trailing edge, 8 apart: the filter input (168, "Filter series":
  every word must begin a word of the name or the author) and the select
  (168): Not finished (the default), Finished and All series, each with its
  count. The choice is kept for the session. In a narrow window the
  controls wrap under the title.
- **Which series:** those in which the reader has read, is reading, paused
  or gave up on a book. A series shows its whole-numbered books, and a
  fractional one (a novella) only when it is in the library or not out yet.
- **Groups:** a `label` with its count, 24 above and 4 below. In progress (a
  book is being read, or the next is out and the series was touched within a
  year), Waiting for the next book (by release date), Not touched in a year,
  and Finished. Within a group, the latest activity first. Under each,
  series rows.
- **Foot:** with Not finished, one muted line ("1 finished series isn't
  shown.") and a small ghost "Show finished", which switches the select to
  All series.
- **States:** skeleton rows while loading; a failed load says so in one
  muted sentence with "Try again"; with nothing started, the empty state
  (the `library` icon, "No series yet", "When you read a book that belongs
  to a series, the series shows here with what comes next."). A filter with
  no match says so in one muted line under the header.

### Screens not in the mockup

Search, Discover, Stats, list pages, series and author pages, Settings,
About, sign-in and the dialogs use the same parts:
- a page title;
- catalogue rows with sm covers and StatusChips;
- rails and hairlines rather than boxes;
- display type only for a book's or person's name and for headline figures.

An author page's header puts the photo where a book has its cover (a large
avatar), with the name in `display-24`. Stats charts use `accent` for the
series, `track` for the grid and muted labels. Stats is one column, and two
on a wide region, filling it (see "Wide windows").

## Right-to-left text

Persian and Arabic titles, names and descriptions appear in an English
interface.

- **Don't mirror the interface.**
- **Alignment:** a right-to-left block sits on the interface's leading
  (left) edge like any other block. Only the line runs right to left.
- **Wrapping:** a title that wraps fills its column, and its lines pack to
  the right.
- **Implementation:** wrapping right-to-left text goes through `WrappingText`
  (`bidi.rs`), because GPUI on Linux draws wrapped right-to-left text in the
  wrong place. A single truncated line (rows) can use GPUI's own layout.
- **Fonts:** Persian and Arabic are set in Vazirmatn (static faces)
  everywhere, under the serif roles as well as the sans ones, so the script
  reads the same across the app. Display sizes use a 1.5 line height. The
  fallback is named, not left to GPUI: each type role names it, and
  `WrappingText` names it for the family it inherits. (A family without
  Latin, such as Noto Naskh Arabic, can't be used: GPUI drops a family that
  has no "m".)
- **Covers:** a cloth cover with a right-to-left title sets it right to left.

## Copy

- **Case:** sentence case everywhere. Proper nouns and Hardcover's own names
  keep their case.
- **Words:** nouns for destinations ("Home", "Discover"), verbs for commands
  ("Update progress…", "Start reading"), and short phrases for states
  ("Not started", "Last updated 8 days ago").
- **Lexicon:** use these words, the same way everywhere.
  - Statuses: Want to read, Currently reading, Read, Paused, Did not finish,
    Ignored.
  - The rest: library, shelf, list, Owned list, reading goal, read (one time
    through a book: "Current read", "Add a past read…"), progress, edition,
    notes, review, Hardcover.
- **Numbers:** "Page 312 of 476", "16 pages a day", "9 of 12 books",
  "4.51 average", with thousands separators.
- **Dates:** "Nov 30", with the year when it isn't this year's ("Nov 30,
  2025").
- **Confirmations:** name the object and the result. "Mark “Dune” as read?"
  with "Mark as read"; never "Are you sure?" with "OK".
- **Errors:** what happened, then how to recover. No apologies, no codes, no
  "successfully".

## Where this system departs from generic desktop guidance (deliberately)

- A serif and a bundled UI font, instead of the platform UI font. The serif
  separates the book from the tool, and the platform font is unreliable
  under GPUI on Linux.
- Display sizes of 24 and 30, above a generic 20 px ceiling, for book titles
  and headline figures.
- Shadows on covers, which are objects, not cards. Panels stay flat.
- Cover radii of 2 and 3, outside the control tiers, because books are
  nearly square.
- Cloth-cover type scales with the cover, because it stands in for an image.

## Implementing in GPUI

**Theme mapping.** The gpui-component theme carries these roles for the
kit's own components. Verify each field against the component that reads
it when you wire it.

| Theme field | Token |
| --- | --- |
| `background` | `bg` |
| `foreground` | `fg` |
| `sidebar` | `sidebar` |
| `sidebar_foreground` | `fg` |
| `sidebar_border` | `border` |
| `sidebar_primary` | `accent` |
| `sidebar_accent` | `selected` |
| `secondary`, `group_box`, `muted` | `surface` |
| `popover`, `button` | `raised` |
| `border` | `border` |
| `input` (outline) | `border-strong` |
| `muted_foreground` | `fg-muted` |
| `primary`, `progress_bar`, `ring`, `link`, `caret`, `slider_bar` | `accent` |
| `primary_foreground` | `accent-fg` |
| `list_hover` | `hover` |
| `list_active` | `selected` |
| `accent` (the kit's hover for menu items and ghost buttons) | `hover`; ink 10% in dark mode, where the kit halves it for ghost buttons |
| `button_hover`, `button_active` | `hover`, `selected` blended into `raised` |
| `secondary_hover`, `secondary_active` | `hover`, `selected` blended into `surface` |
| `primary_hover`, `primary_active` | `hover`, `selected` blended into `accent` |
| `button_danger` | clear: `danger` text, with `hover` and `selected` on hover and press |
| `switch` (off), `switch_thumb` | `track`, `switch-thumb` |
| `scrollbar_thumb`, `scrollbar_thumb_hover` | `border-strong`, `fg-faint` |
| `tab_bar_segmented` | `chip` blended into `bg` |
| `chart_1` to `chart_5`, `chart_grid` | `accent`, `track` |
| `skeleton` | `skeleton` |
| `danger` | `danger` |
| `warning` | `warning` |
| `radius` | 6, times the interface scale, rounded |
| `radius_lg` | 8, times the interface scale, rounded |
| `font_family` | Inter |
| `font_size` | 16, times the interface scale (the rem) |

The app builds this as two theme configs (`design::theme`), so every mode
change applies it. `ring` is the solid accent: the kit draws its ring at 50%
of it, plus a 1 px accent edge. What the kit doesn't let the theme
reach is listed under "Kit limits".

**Notes:**
- **App-owned roles** live in one app module and nowhere else: gold,
  star-empty, the cloth colours, cloth-ink, cover-edge, cover-hinge and
  the cover shadow.
- **Overlay fills on controls:** a control that paints its own background
  (a default button on `raised`) needs the overlay blended into a solid
  colour. GPUI replaces a background; it doesn't layer one.
- **Line heights:** set them explicitly with `line_height(rems(..))`. GPUI's
  default line height is 1.618.
- **Tabular figures:** `font_features` with `tnum`.
- **Gradients and shadows:** the hinge is a `linear_gradient`; shadows are
  `BoxShadow`s.
- **Raw `px(...)`** doesn't follow the interface scale. It is allowed only
  for hairlines (the cover's edge, the cloth's frame and rules), focus
  rings, the scrollbar thumb, the pace tick and shadows. Everything else
  uses the rem helpers and tokens, cover geometry included (`CoverSize`'s
  sizes and radii are rems). A width handed to text layout
  (`WrappingText::width`, `TextButton::wrap_width`) is a rem size converted
  with `window.rem_size()`, never `px(design)`. The window's sizes come from
  `ui_scale::policy`, in logical px at the interface scale.
- **Fonts** are bundled with their OFL licences, which `package.sh`
  installs.
  - Literata Text (12 pt) at 400, 400 italic and 600, and Display (36 pt) at
    600.
  - Vazirmatn at 400, 500 and 600, for Persian and Arabic everywhere.
  - If a release lacks a static instance, cut one from the variable font
    with `fonttools varLib.instancer` and commit the static output.
  - The test in `fonts.rs` that rejects variable faces covers every new face.

## Kit limits

The app follows this system everywhere: the palette is on the kit's theme
(`design::theme`), every value comes from the token module
(`design::colors`, `design::size`, `design::TypeRoles`), the fonts are
bundled (`fonts.rs`), covers have one component (`cover.rs`), and every
screen is built from the shared components in `crates/app/src/ui/` (the
demo build shows them all with `HARDCOVER_GALLERY=1`).

`ui::kit` fixes what the theme can't reach and the kit lets the app
refine: inputs, selects and text areas on `raised` (`RaisedField`), small
controls at 12 px (`SmallControl`, `SmallIconButton`), dialogs on `raised`
with the system's widths and keys (`open_dialog`, `open_alert_dialog`,
`bind_keys`), and quiet buttons on the text spine (`OnTextSpine`).

What follows is off the system and can't be fixed without patching the
kit. Keep it in step with `ui::kit`'s module doc; remove an item in the
commit that fixes it.
- Menus (`PopupMenu`) and the Select dropdown round at 6, not 8, and use
  the kit's own shadow, not `shadow-popover`.
- Menu items are 26 high (one every 28), not 32, with 12 px icons, not 16:
  the menu sets both itself.
- Menu items hover at 10% in dark mode: the kit's `accent` serves both menu
  items and ghost buttons, and ghost buttons take half of it there.
- The sidebar's menu hover is 8%, not 5%.
- The segmented tab bar's sliding indicator is always paper.
- A disabled segment of a segmented tab bar is painted paper, like the
  selected one, so it reads as chosen. The app leaves out a segment it
  can't offer instead.
- A disabled switch fades its track, not its thumb.
- A radio's circle takes no style from outside: unchecked and disabled, it
  is faint, most in dark mode. Settings shows a fixed theme as text rather
  than disabled radios; a form's radios are disabled only while it saves.
- The `DatePicker`'s field stays paper.
- A dialog keeps the kit's shadow.
- The kit's controls (buttons, inputs, selects, switches) draw a 3 px focus
  ring, not 2: the width is a private constant of the kit. The app's own
  rows, text buttons and components draw 2.
- Some of the kit's own measures are px, so they don't follow the
  interface scale (`ui_scale`) and look a little small at 1.25: a chart's
  labels (Stats, the goal's months) are a fixed 10 px; a segmented tab
  bar's height and padding (Search's tabs, Discover's periods, the progress
  dialog's units); a dialog's margins.
