# Hardcover Desktop design system

The reference for every visible part of the app. The words **must** and
**never** mark rules. **Should** is the default; a concrete reason is needed
to depart from it, and the reason belongs in the code comment or the commit.
The mockup at `docs/design/mockup.html` shows every rule here applied. Where
the mockup and this file differ, this file wins.

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
| `border-strong` | `#918879` | `#77706A` | Control outlines: at least 3:1 on `raised`, paper, `surface` and the sidebar, since a field's outline is all that marks it |
| `track` | `#E0D9CC` | `#35312C` | The empty part of a meter; an off switch |
| `switch-thumb` | `#FFFFFF` (`raised`) | `#EEEAE3` (`fg`) | A switch's thumb, on `track` and on `accent`. In dark mode a `raised` thumb on `track` is 1.2:1, and gone once a disabled switch fades its track |
| `fg` | `#1F1C17` | `#EEEAE3` | Ink: text |
| `fg-muted` | `#5F574B` | `#ABA498` | Secondary text that carries information: authors, facts, counts, labels |
| `fg-faint` | `#7A7266` | `#8D877C` | Empty-state icons and other marks. Never text |
| `accent` | `#3B4A9C` | `#98A5F4` | Ink blue (details below the table) |
| `accent-fg` | `#FFFFFF` | `#12142A` | Text on accent |
| `accent-soft` | accent 10% | accent 14% | The avatar's fill |
| `ring` | accent 50% | accent 50% | The 2 px focus ring |
| `gold` | `#A16207` (ochre) | `#FBBF24` (amber 400) | The user's own rating, nothing else. The hover preview draws it as an outline |
| `star-empty` | ink 10% | ink 10% | An unfilled star |
| `danger` | `#B42318` | `#FB8178` | Destructive commands and errors |
| `warning` | `#B54708` | `#FDB022` | A content warning's icon; warnings |
| `cover-edge` | `rgba(30,20,10,.16)` | `rgba(255,255,255,.12)` | The cover's 1 px edge |
| `cover-hinge` | `rgba(30,20,10,.16)` | `rgba(0,0,0,.28)` | The hinge gradient's dark stop |
| `cloth-ink` | `#F4ECDD` | `#F4ECDD` | Type and stamp on a cloth cover |
| `scrim` | ink 16% | black 48% | The backdrop behind a dialog |

Accent is used on progress, the primary commit, the focus ring, the selected
sidebar icon, links that open the browser, the brand icon, a genre chip's
icon, and the ring and "Next" label on a series strip's next book. Nothing
else.

Contrast, checked over every surface alone, hovered and selected
(`design::contrast_audit` fails the build when a token breaks one):
- **Text that carries information is at least 4.5:1** (WCAG 1.4.3):
  `fg-muted` at least 4.7:1 in both themes, so it may carry information;
  accent on paper at least 5:1 and on a chip at least 4.8:1; danger on
  paper and in a dialog, pressed too.
- **Marks that carry meaning without words are at least 3:1** (WCAG
  1.4.11): `border-strong` on `raised`, paper, `surface` and the sidebar
  (3:1 to 3.9:1); `gold` against an empty star on paper, hovered or
  selected (3.1:1 light, 6.4:1 dark); the catalogue's `fg-muted` stars
  against an empty one; the warning icon on a chip.
- **`fg-faint` is 3.3:1 to 5.4:1**: enough for an icon, never enough for
  text. Placeholders are `fg-muted`.

A new colour role needs the same check: add its pairs to
`design::contrast_audit` (the mockup's console audit shows the same
numbers).

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
| `page-title` | Inter 600 | 20 / 28 | `.page_title()` | A destination's or page's title: "Currently reading" (always through `ui::PageHeader`; a catalogue name wraps, a destination's is cut) |
| `section` | Inter 600 | 16 / 24 | `.section()` | A section on a page: "Continue reading" |
| `byline` | Inter 500 | 16 / 24 | `.byline()` | A book's author and series under its title in the header |
| `heading` | Inter 600 | 14 / 20 | `.heading()` | A block in a pane: "About this book", "Details" |
| `body` | Inter 400 | 14 / 20 | `.body()` | Everything by default |
| `body-strong` | Inter 500 | 14 / 20 | `.body_strong()` | Row titles, values that lead |
| `label` | Inter 500, muted | 12 / 16 | `.label(cx)` | Sidebar and rail section labels: "Reading goal", "On Hardcover" |
| `caption` | Inter 400, muted | 12 / 16 | `.caption(cx)` | Counts, trailing figures, timestamps, helper text |
| `display-30` | Literata Display 600 | 30 / 36 | `.display_30()` | Book title on the full page; the Home greeting; figures ("9", "4.63") |
| `display-24` | Literata Display 600 | 24 / 30 | `.display_24()` | Book title in the pane and the Home lead; a series' name on the Series page |
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
| Next lane (a series row) | 240 wide, 24 after the row's body | `size::NEXT_LANE`, `NEXT_LANE_GAP` |
| Strip item (a series row) | 72 wide (an md cover in a 2 px inset and a 2 px ring), 8 apart; opened, lines 16 apart | `SeriesStrip::item_width`, `size::STRIP_GAP`, `STRIP_LINE_GAP`, `STRIP_RING_INSET`, `STRIP_RING` |
| Key column in a property list | 96; 72 in a rail | `w_24`, `w(rems(4.5))` |
| Select | 168; small 128 | `size::SELECT`, `size::SELECT_SMALL` |
| Number field (a page, a time) | 128 wide | `size::FIELD_NUMBER` |
| Date field | 224 wide max | `size::FIELD_DATE_MAX` |
| A book list's status chips | icon only below a 360 list | `size::LIST_COMPACT_BELOW` |
| App icon (About) | 64 | `size::APP_ICON` |
| Meter | 6 high beside words, 2 high inside a row; 400 wide max (160 in a row) | |
| Prose | 640 wide max | `size::PROSE` |
| Page content (Discover, Coming soon, Stats, Series, and tag, vibe, Trending, list, series and author pages) | fills the region inside the 32 inset; only reading text keeps the prose measure | `size::PAGE_INSET` |
| A catalogue page's list beside the book pane | 704: the prose measure and the inset; the pane takes the rest | `size::PAGE_BESIDE_PANE` |
| Book pane and full book page | fill their region | |
| Side column beside a book's text | at least 320 wide; the text keeps 640, 32 apart | `size::SIDE_MIN`, `size::PROSE` |
| Home | fills the region | |
| Content column beside another (Home, wide) | at least 448 wide; the two share the width beside the rail | `size::WIDE_COLUMN_MIN` |
| Stats, two columns | each column at least 544, filling its half | `size::CHART_COLUMN_MIN` |
| Chart | bars at most 48 wide; a year in Finished per year at least 64 | `size::CHART_BAR_MAX`, `CHART_YEAR_MIN` |
| Votes popover | 360 wide, padding 16; its mood names 96, its counts 48 | |
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

### Motion

The app moves only to explain a change: something left, or a value changed.
Nothing is decoration, and nothing waits for motion to finish. These are all
the motions there are; a new one is added here first (`design::motion`).

| Motion | Where | How |
| --- | --- | --- |
| Pulse | A skeleton while its content really loads | Opacity 100% to 50% and back over 2 s, every skeleton in step |
| Row leaving | A row taken out of the collection on screen: the selected book moved to another shelf or out of the library, a book taken off a list, a series unfollowed on the Series page, a book the reader said never to recommend (a vibe's page) | It keeps its place and fades out over 150 ms, ease-out, without its focus ring; then the rows below close the gap at once |
| Meter | A meter's value changing: progress saved, the goal after a sync | The fill moves on a critically damped spring (90% of the way in about 150 ms, settled in about 350 ms, no overshoot); a change mid-way carries on from where the fill is. The words beside it change at once |
| Kit overlays | Dialogs, menus, popovers, tooltips, notifications | The kit's own short fades and slides, unchanged |

- **Only what the reader is looking at moves.** A row fades only when the
  list is otherwise the same (same shelf, filter and sort; nothing added or
  moved) and at most three rows leave together; a leaving row can't be
  acted on. A meter moves only when the same thing's value changes: one shown
  for the first time, or for another book, starts at its value. Hardcover's
  figures (the muted ratings breakdown, the votes' meters) never move.
- **Instant everywhere else:** pages, Back, destinations, the pane following
  the selection, rows arriving or moving, a filter or sort, expanding text,
  covers landing, hover and focus.
- **Reduced motion** (the desktop's setting): every motion shows its end
  state at once. The skeleton holds still.
- **Not motion:** content that arrives later and pushes the page down is
  fixed by keeping its place, never by animating it in.

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
  the book; its title is a button too. A clickable cover is a link named
  for what it does (`Open “Dune”`), so a screen reader's pointer can use
  it; it is not a tab stop.

| Size | Width × height | GPUI | Use |
| --- | --- | --- | --- |
| xs | 20 × 30 | `w_5` × `h(rems(1.875))` | Compact rows, menus |
| sm | 32 × 48 | `w_8 h_12` | List rows (shelves, search, lists, and tag, vibe, Trending, series and author pages) |
| md | 64 × 96 | `w_16 h_24` | Home rows, Up next, a rail's shelf, Discover's shelves |
| lg | 80 × 120 | `w_20` × `h(rems(7.5))` | Similar books; Discover's lead shelf |
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
| Text button | Ink text, underline on hover only | In-app navigation inside content: an author, a series, a book title, a mood on a book's mood line |
| Link | Accent, underlined, a 12 px external-link icon | Only for things that open the browser: "Open on Hardcover" on a series, author or list page (a book's is in its "more" menu), and on a vibe's page once its list is over 7 days old |
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
  - Placeholder: `fg-muted` (the kit's `muted_foreground`).
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
  left out, and a caption under the field says why. The segments are
  `ui::kit::small_tab`s in a `.small_control()` bar: 24 high, 12 px labels.

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
  - A tag too long for its row is cut with an ellipsis, and then shows its
    whole text in a tooltip and to a screen reader.
  - The icon sits on the chip's leading (left) side, a right-to-left tag
    too: the interface isn't mirrored.
- **Wrapped chips** (`TagRow`): chips that wrap across a width sit 6 apart
  both ways (`s-6`), in the header and in the Tags block alike.

Most chips only say something. Two kinds are pressed:
- **Page chip:** a TagChip that opens a page: a genre in a book's header or
  Tags block (the genre's page), and Discover's Browse chips (a mood's or a
  genre's page, a featured vibe's page). The same pill, with the `hover`
  fill over its own on hover and the pointing hand; nothing else changes,
  so it still reads as a tag. Content warnings and other tags stay plain.
- **Filter chip** (a shelf's filter bar): a genre (with its icon) or a mood,
  then how many books a press would leave, tabular at weight 400 ("Fantasy
  5").
  - **Selected:** the `selected` fill over its own, the text in `fg`, and a
    12 px check, in `fg` too, in the icon's place (before a mood's name),
    so the state never rests on the fill alone. A press clears it.
  - **Disabled:** a chip whose press would leave no books: 50% opacity, no
    response, and the arrows skip it. A selected chip is never disabled.
  - It is a toggle to a screen reader, named with its count ("Fantasy, 5
    books", pressed or not).
- **Keys:** a row of pressed chips is one stop in the Tab order, as the
  sidebar's destinations are. Tab enters on the chip the row was left from
  (the first until then); Left and Right move along it (onto its "More"
  too), Home and End go to the ends, Enter and Space press, and Tab or
  Shift+Tab leaves. A chip with focus shows the ring outside the pill and
  the 1 px accent edge inside it.

Never colour-code chips, and never use a badge for ordinary metadata.

### Stars (`star.rs`)

- The app's own rounded solid star. Sizes: 20 for the rating control, 16
  in rows, 12 inline; gap 2.
- **Your rating:** `gold` fill. It reads at 3:1 against an empty star.
- **Hardcover's figures** (the average, the breakdown): the same star in
  `fg-muted`.
- **An empty star:** the same shape in `star-empty`, so a half star splits
  cleanly.
- An unrated rating control reads "Rate" beside it, in `fg-muted`.
- **Hover preview:** over the rating control, the stars a click would set
  show as `gold` outlines over the empty star (`star::outline`): they
  differ from a saved rating by shape, not by lightness alone. It is the
  pointer's preview, never a saved rating; the keyboard changes the rating
  itself.
- **Keys:** Left and Right move the stars half a star at a time, shown
  solid at once; the value is saved once the keys rest (half a second), or
  at once on Enter or when focus leaves. Up and Down stay the page's, so
  they scroll past the control. Backspace and Delete clear, at once.
- **Pointer targets:** each half star is a slice of the whole control, its
  full height, and the 2 px gaps belong to the halves beside them: ten
  equal slices, no dead space.
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
- A new value moves the fill there (Motion); the words change at once.
- Every meter has an accessible name, what it measures ("Reading
  progress", "Progress toward the goal", "5 stars"): `Meter::new` takes
  it.
- **Hardcover's figures** (the ratings breakdown, the votes in the Votes
  popover) are muted meters: the fill is `fg-muted` on `track`, never
  `accent`, which is the reader's own progress. They never move. Each is
  named with its figure ("Adventurous: 422 votes", "Medium pace: 58%").

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
- **Catalogue row** (search, and tag, vibe, Trending, series, author and
  list pages): the
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
  - the series name in `display-24`, a text button that opens the series
    page;
  - a muted line: the author, "#6 not out yet" when a later book is still
    to come, and the activity ("Reading now", "Last finished Sep 28",
    "Caught up since Feb 2", "Paused since Jun 2, 2024", "Finished Jan 12,
    2025");
  - 8 below, the series' meter (read of what is out, at most 400 wide) with
    "6 of 31 read" beside it in `body-strong`;
  - 4 below, the key, a caption line: each mark the strip shows with how
    many books carry it, 16 apart, in the key's order ("✓ 6 read", "📖 1
    currently reading", "⏸ 1 paused", "📥 9 want to read", "1 did not
    finish", "1 ignored", a small faded cover with "11 not in your library",
    the clock with "1 not out yet"), so the marks explain themselves;
  - the strip, 24 below (room for "Next" over the next book);
  - the next lane, 240 wide and kept on every row so the lanes line up: an
    md cover, a caption ("Reading · #1", "Next · #3", "Paused · #3"), the
    title as a text button, a line ("Page 418 of 1,007 · 42%", "Paused at
    page 140 of 412", or the date in `body-strong` with "· In 6 days"
    muted), then one small command: "Update progress…", "Start reading", a
    ghost "Want to read" with a plus, or the StatusChip. A finished series
    leaves the lane empty.
- **Strip** (`ui::SeriesStrip`): the books of a series in order, 8 apart,
  each 72 wide: the md cover inside a 2 px inset and a 2 px ring, clear but
  for the series' next book, whose ring is `accent` with "Next" (caption,
  600, `accent`) over it; a 2 px line under it (a meter for the book being
  read, else empty, so the positions sit on one line); the shelf's 12 px
  icon and the position ("#3") in a caption, the next book's in ink at
  weight 600; then the title, two caption lines at most, centred, in ink
  (muted for a book not in the library). A book that is out but not in the
  library shows its cover faded to 40% (`series_strip::MISSING_OPACITY`),
  with no shelf icon; one not out yet is a **slot** (`ui::CoverSlot`: a
  dashed `border-strong` outline in the cover's shape, no fill) with a 12 px
  clock. Every book opens on a click, the slot too, and shows its whole
  title in a tooltip. An Ignored book keeps its cover and the Ignored icon,
  and is never the next book.
  - **One line until opened:** a strip that fits shows every book. One that
    doesn't shows as many books as fit beside a small ghost "Show the other
    15 books" (the room of three books), starting at the book before the
    next one, so the line shows what came last and what comes next.
    "Show the other N books" opens the strip to every book, wrapping onto as
    many lines as it needs, 16 apart, and "Show fewer" (the room of two)
    ends it. The series opened are kept for the session.
  State never relies on colour: the shelf icon (a faded cover has none), the
  slot, the clock and the "Next" label carry it.
  Each book is named for a screen reader with what the eye reads from it:
  "#2, Dune Messiah: Currently reading, 40%, next", "#3, Children of Dune:
  not in your library".
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
- **Reasons are never hover-only.** A row's tooltip (why it is disabled,
  why a load failed) is also its accessible description, and while the row
  has keyboard focus it shows as a caption under the label. The account
  row does the same with why a sync failed, under the sync line. A
  disabled row can't take focus; a missing permission is also the
  sidebar's "Sign in again" notice, which can.
- **Keyboard:** the destinations, from Home to "New list…", are one stop in
  the Tab order however many lists the reader has, so the page is never
  dozens of presses away. Tab enters on the row the sidebar was left from
  (Home until then); Up and Down move between rows, stopping at the ends,
  Home and End go to the first and the last, Enter and Space press the
  row, and Tab or Shift+Tab leaves. The account row at the foot is a stop
  of its own.

### Popovers, menus and dialogs

- **Popover and menu:** `raised`, `r-box`, `shadow-popover`, padding 4.
  - Menu items are 32 high, padding 8, `r-control`, with a 16 px icon in a
    fixed slot. They use the same hover and selected fills as rows.
  - The app's menus are the kit's, which keep their own radius, shadow,
    item height and icon size (Kit limits).
  - A popover that holds content, not commands (the Votes popover, the
    only one), has a 16 inset and a fixed width. It opens 4 under its
    menu button, on the button's leading edge, and scrolls inside when
    the window is too short for it. The button stays pressed while it is
    open; Escape or a press outside closes it and gives focus back to the
    button.
  - A row's context menu (a vibe's rows, the only one) repeats commands
    that have another path, never the only one: the keyboard reaches them
    through the book's own menus.
- **Dialogs:**
  - **Width:** confirmations 24 rem; forms 30 rem.
  - **Title:** the decision ("Delete “Book club picks”?").
  - **Body:** only new information.
  - **Buttons:** Cancel, then the result ("Delete" in danger; "Save" in
    primary). A confirmation's result that destroys nothing ("Add",
    "Install", "Never recommend") is primary; Cancel has the focus when it
    opens.

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
- optionally one button, or one link when what helps is in the browser
  ("Open on Hardcover").

Use the app's own words: "No matches" / "No books in Did not finish match
“dune”." and "Select a book to see its details". An empty state is never an
alert.

## States

Each state has one treatment, on every control. Hover is never the only cue.

| State | Treatment |
| --- | --- |
| Rest | As specified for the component |
| Hover | The `hover` fill. A text button gains its underline. The rating control previews the stars a click would set as `gold` outlines |
| Pressed, selected, open | The `selected` fill. A selected nav row also takes weight 500 and the accent icon. A menu button stays pressed while its menu is open |
| Focus-visible | A 2 px `ring` outside, plus a 1 px accent edge: the border for controls, an inset edge for rows, and just outside the text for bare text (a text button, a byline link), which has no border. Never clipped by a scrolling parent |
| Disabled | 50% opacity, with no hover or pressed response. When the reason isn't obvious, say it beside the control or in its tooltip (`format::needs_permission`) |
| Loading | `skeleton` shapes where the content will be, pulsing between 100% and 50% opacity. Covers show a skeleton until the image lands. A button running its action keeps its label, shows its loading state and refuses repeat clicks. No spinner in a list |
| Error | Say what happened and how to recover, next to the thing that failed (`session::error_message`): one muted sentence with a small "Try again" (`ui::LoadFailed`); where the whole region has nothing else to show (the library never loaded), the same in the empty state's place, titled ("Couldn't load your library"), and counts that would read 0 read "—" |

Switching between content that is already loaded is immediate: no skeleton,
no transition. The few things that move are listed under Motion.

## Screens

Every screen starts from its task:

| Screen | Primary task | Focal point |
| --- | --- | --- |
| Home | Pick up where you left off | The lead book |
| Shelf | Find a book on it | The list |
| Book | Know the book and update it | Cover, title, status, progress |
| Coming soon | See what's next in your series, and want it | The dates |
| Series | See where you are in each series and what's next | The next step in each series |
| Discover | Find a book you don't know | The first shelf: "For you" |
| A tag's page | See the books readers file under a mood or a genre | The list |
| A vibe's page | Go through one of Hardcover's lists for you | The list |

### Window shell

- The sidebar (240) sits beside the destination.
- Book, series, author, tag, vibe and Trending pages open over the
  destination with a Back bar.
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
  less its insets. Never from the raw window width. One helper gives that
  width beside the sidebar (`ui::region_width`); a screen never subtracts
  the sidebar itself. The choice is a pure function with unit tests
  (`ui::columns::tier`, like `folds`), pinned at 800 and 1200 (today's
  layout), on both sides of each threshold, and at 1600 and 2560. The tier
  is what `Columns` is built from, so a second column can't sit beside the
  main one except in the three-column tier.
- **Tiers by screen:**
  - **Home:** fills its region in every tier. Two content columns of at
    least 448 beside the rail (a 1536 window) give it three columns:
    Continue reading; Coming soon, Rate what you finished, then Newest on
    Want to read; the rail. The two content columns share the width beside
    the rail equally. With nothing for the second column, Home keeps two
    columns.
  - **Series:** the column fills the region. The strip takes the width,
    showing as many books on its line as fit (25 at 2560) before it is cut
    to one line with "Show the other N books"; the next lanes stay 240 and
    lined up on the trailing edge.
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
  - **Tag, vibe, Trending, list, series and author pages:** the list fills
    the region, and the page's intro line and any description keep the 640
    measure. When the region also fits a book pane of 392 beside a 704
    list (a 1337 window), the selected book shows in the pane, which takes
    the rest.
  - **Discover:** fills the region. Each cover shelf is one row with as
    many covers as fit, never a second row; the Browse chips wrap across
    the width.
  - **Coming soon** fills the region; its date and action lanes keep the
    trailing edge. **Search** is a shelf: its list keeps its resizable
    width and the pane takes the rest.

### Shelf: list plus book pane

- **List:** 384 wide by default, with a trailing hairline.
  - **Header:** padding 12/16, a hairline under it. The page title has its
    count beside it in `fg-muted`; under that, the filter input ("Filter",
    filling the width) and the sort select (168), 8 apart. As the list
    narrows, the select gives way first, down to the small 128; the filter
    keeps at least 104. Under them, the filter bar (below).
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
  - the mood line (below), when the book has one;
  - the genres: genre TagChips (with their icons), wrapped 6 apart. Each
    is a page chip: it opens the genre's page;
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
- **Tags:** the genres past the header's first six, then content
  warnings and other tags, each a group, 16 apart: the group's name
  ("Content warnings") in `label`, and 6 below it its chips, wrapped 6 apart
  across the block's width. Each group shows six until the block's small
  ghost "Show N more" (then "Show fewer"). Moods are never chips here: they
  are the header's mood line and its votes. A block left with one group
  takes the group's name as its `heading` ("Content warnings") and drops
  the label.

### The mood line (a book's header)

What Hardcover's readers say the book is like, in one line of the header
and no block: "Hopeful · Funny · Adventurous", then "Votes".
- **Where:** in the info column of every book header (the pane and the full
  page), after the facts and Hardcover's average, before the genres. It
  comes with the book's details, as the genres do, with no skeleton of its
  own.
- **The line:** `body`. Up to three moods, each a text button that opens
  the mood's page, with " · " in `fg-muted` between them; then, for a slow
  or fast book, " · " and the pace in plain ink ("Slow pace", "Fast pace");
  then, 4 after, "Votes": a small ghost menu button. The moods wrap; the
  pace and "Votes" stay together.
- **Which moods.** From Hardcover's exact votes for its 14 moods on the
  book (`taggable_counts`); a mood's share is its votes over the book's
  votes for all 14, and its usual share is the same across Hardcover.
  - Under 50 mood votes: **no line and no "Votes"**.
  - A mood stands out with at least 10% of the book's mood votes and at
    least 1.5 times its usual share. The line shows those, the furthest
    above usual first, at most three.
  - When none stands out, the line shows the three moods with the most
    votes.
- **Pace** is named only when Slow or Fast leads, with at least 20 pace
  votes and at least half of them. Medium is never named: nearly every
  book is.
- The numbers above are the rule; they are kept in one place in code, with
  a test over real books.
- **Keys:** each mood and "Votes" is a Tab stop. To a screen reader the
  line is a group named "Moods from Hardcover readers".

### The Votes popover

Every figure behind the line, on demand: a content popover (Popovers) 360
wide, opened by "Votes".
- **Top:** "Votes from Hardcover readers" in `caption`, muted, 12 above the
  groups. Then groups 16 apart, each under a `label`, 6 below it.
- **Moods:** the moods with at least 5 votes, the most votes first (ties in
  Hardcover's order). Rows 4 apart: the mood (a text button that opens its
  page, in a 96 lane), a 6 px muted meter filling the row, its length
  against the first mood's, and the votes in a 48 lane (`caption`, tabular,
  muted, right-aligned). "Votes" in `caption`, muted, names that lane on
  the label's line.
- **Pace** (Slow, Medium, Fast) and **Driven by** (Plot, A mix,
  Character), each shown from 20 votes: three equal cells 12 apart, in
  that order. A cell has the answer, its share on the trailing edge
  (`caption`, tabular) and a 6 px muted meter under them; all muted, but
  the leading answer's name is ink at weight 500. The three shares add up
  to 100%.
- **Characters:** one muted line of the traits readers agree on: of
  Loveable or Unloveable, Strong development or Weak development, Diverse
  or Not diverse, the side with at least two thirds of the pair's votes,
  when the pair has at least 20. The trait in ink, its share after it
  ("Loveable 92% · Strong development 68%"). A pair nearer even says
  nothing and is left out; with none, no group.
- **Rarely:** under a hairline (12 above and below), one `caption` line,
  muted: of the ten most used moods on Hardcover (Adventurous to Hopeful),
  those with at most a fifth of their usual share (no votes included), the
  most used first, at most three, and only when the book has at least 100
  mood votes: "Rarely dark.", "Rarely dark, mysterious or sad." The four
  least used moods are rare on most books, so their absence says nothing.
  With none, no hairline.
- The meters are Hardcover's figures: muted, and they never move (Meters).
- **Keys:** Enter or Space on "Votes" opens it and moves focus to the first
  mood; Tab goes through the moods; Escape closes it. Choosing a mood
  closes it and opens the mood's page.

### The filter bar (a shelf's list)

Narrowing the reader's own shelf by genre and mood: for choosing the next
book, not for discovery. Each of the six status shelves has it (not
Search, and not a list's page), in the list's header, 12 under the filter
and the sort.
- **Two rows,** 6 apart, each 24 high: genres, then moods. Each row holds
  filter chips (Chips), 6 apart, for the genres or moods the shelf's books
  have: the most books first (ties by name for genres, in Hardcover's
  order for moods), as many as fit on one line, then a small ghost menu
  button "More" with the rest, named "More genres" or "More moods" for a
  screen reader. A row never wraps. A row with nothing to offer hides, and
  so does "More" with nothing left.
  - **"More":** each item is the name with its count on the trailing edge;
    the selected one is checked, and one that would leave no books is
    disabled. A choice made there takes the first place in its row for as
    long as it is selected, so it can always be seen and cleared.
  - **Narrow list:** fewer chips fit; a row keeps its selected chip, else
    its first, and a chip too long for the row is cut with an ellipsis.
- **What a book has.** Its genres are Hardcover's, with duplicates merged
  under one name ("Sci-fi", "Science Fiction" and "Science-Fiction" are
  one). It has a mood when the mood is on its mood line, so the bar and
  the pane always agree.
- **One genre and one mood at most,** and they combine: a book must have
  both. Pressing another chip of the same row replaces the choice;
  pressing the selected one clears it. The choices are kept per shelf for
  the session.
- **Counts** say what a press would leave: with a mood selected, each
  genre's count is among that mood's books, and the other way round. They
  don't follow the text filter.
- **Title row:** while a chip or the text filter is set, the count reads
  "2 of 8" and a small ghost "Clear filters" sits on the trailing edge; it
  clears both chips and the text.
- **Foot:** while a chip is selected and some of the shelf's books have no
  genre or mood votes, one `caption` line, muted, under the rows (inset
  8/16): "1 book on this shelf has no genre or mood votes yet, so no chip
  finds it." ("3 books on this shelf have…").
- **No matches:** the empty state in the list's place: "No matches", one
  sentence, and a default "Clear filters". The sentence is one of: "No
  books in Want to read are both Mystery and Adventurous.", "No books in
  Want to read match Mystery." (one chip), "No books in Want to read match
  “dune” with these filters." (the text filter as well). A disabled chip
  can't be pressed, so this shows only when the shelf changes under the
  chips, or with the text filter. The selected chips stay pressable.
- **States:** the genres and moods come from one cached fetch of the
  library's tags. Until it first lands, skeleton chips keep the two rows'
  place; if it fails, one muted line with a small "Try again" takes it.
- **Keys:** each row is one Tab stop (Chips); a press keeps focus on the
  chip.

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
    at most two lines, broken between words; a word too long for the
    line ends in "…", never split. Eight, wrapping.
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
  - **Up next:** the next book of a series the reader follows (Series): an
    md cover with shadow; "Next in Dune · #2" as a caption; the title in
    `title-18`; the author; a small "Start reading".
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
  soon", then a muted line ("5 books still to come in the series you
  follow").
- **Groups:** a `label` per month for the coming 12 months ("November",
  "February 2027"), then per year ("Later in 2027", "2028"), with 24 above
  and 4 below. Under each, upcoming rows.
- **States:** a failed load says so in one muted sentence with "Try again";
  an empty list uses the empty state.

### Series (a destination)

- **Which series:** only those the reader follows on Hardcover. Nothing
  joins by itself: a series is followed from Add series, a series page or
  Undo, and leaves only when unfollowed. Home's Up next and Coming soon
  look at the same set. A series shows its whole-numbered books, and a
  fractional one (a novella) only when it is in the library or not out yet.
- **Sidebar:** "Series" after Discover, with the `library` icon and the
  count of followed series not finished. It hides, with the page, without
  `read:catalog` or `read:social` (the follows).
- **Content:** inset 32, filling the region (see "Wide windows").
- **Header:** the `page-title` "Series" with the count shown beside it,
  then the muted line "The series you follow, and what's next in each."
  On the trailing edge, 8 apart: the filter input (168, "Filter series":
  every word must begin a word of the name or the author), the filter
  select (168): Not finished (the default), Finished and All series, each
  with its count, the sort select (168): Status (the default: the groups
  below), Recently active, Name (a leading article left out), Author (by
  last name), Most read (the meter's share) and Fewest left to read (books
  out and still to read; a series with none left last), and "Add series…"
  (a button with `plus`), which opens Add series. Every sort but Status is
  one list without group labels, its first row with no hairline over it.
  Both choices are kept for the session. In a narrow window the controls
  wrap under the title, and onto a second line. The header stays put while
  the series scroll under it, so the filters are always at hand.
- **Nudge:** under the header, once the follows are known, one muted line
  names the started series not followed that the reader touched in the
  last 30 days ("You've been reading 2 series you don't follow: “Elfhame”
  and “The Folk of the Air”."), with a small ghost "Add series…". It never
  follows anything itself, and an old library doesn't nag.
- **A row's menu:** each row's name line ends in a small "more" icon button
  ("Series actions"): "Unfollow series" (`bookmark-minus`), and "Add N
  books to Want to read…" (`plus`) while whole-numbered books that are out
  aren't in the library; it asks first (below). Unfollowing leaves the list
  as a row leaving does (Motion), and a notification says so with Undo
  ("You no longer follow “Mistborn”. It's left Home too."). Without
  `write:social` or `write:library` the command is disabled with its reason
  under it.
- **Groups:** a `label` with its count, 24 above and 4 below. In progress (a
  book is being read, or the next is out and the series was touched within a
  year), Waiting for the next book (by release date), Not started (followed,
  nothing read, being read, paused or given up on; its next step is the
  first book out), Not touched in a year, and Finished. Within a group, the
  latest activity first (a series not started: when it was followed).
  Under each, series rows.
- **Foot:** with Not finished, one muted line ("1 finished series isn't
  shown.") and a small ghost "Show finished", which switches the select to
  All series.
- **States:** skeleton rows while loading; a failed load says so in one
  muted sentence with "Try again"; following nothing, the empty state (the
  `library` icon, "You don't follow any series", "Follow a series to see
  where you are in it and what comes next.", and "Add series…"), with the
  filter and sort left out until there is something to filter. Not loaded
  is never "follows nothing": the empty state waits for the follows. A
  filter with no match says so in one muted line under the header ("Every
  series you follow is finished.", "No series match “dune”.").

### Add series (over Series)

- **Object bar:** 56 high with a hairline; Back ("Series") on the leading
  edge.
- **Content:** inset 32, at most 704 wide. The `page-title` "Add series"
  and the muted line "The series of the books on your shelves that you
  don't follow yet.", with the filter input (168, "Filter series", as on
  Series) on the header's trailing edge.
- **Groups:** "Started" (a book read, being read, paused or given up on;
  the most recently active first, so the nudge's series lead), then "Only
  on Want to read" (the latest added first), each a `label` with its count
  on the trailing edge, 24 above and 4 below, over a sequence of rows.
  Ignored books count for nothing.
- **Row:** the sm cover of the series' first book on the shelves (opens
  the book), the name (a text button that opens the series page), the
  muted line "author · how far" ("Holly Black · Reading now", "2 read ·
  Last finished Oct 30", "1 on Want to read"), and a small "Follow"
  (`bookmark-plus`). A series followed here stays where it is as a pressed
  "Following" (`bookmark-check`), which unfollows again, until the page is
  opened again, so nothing moves under the pointer and a slip is one press
  to undo.
- **Without `write:social`:** every Follow is disabled, and one muted line
  under the header says why (`needs_permission`).
- **States:** skeleton rows until the library and the follows are known; a
  failed load says so with "Try again"; nothing left to follow, or nothing
  matching the filter, is the empty state.

### Discover (a destination)

All discovery lives here: books the reader doesn't know. Nothing of
Hardcover's recommendations shows on a shelf or on Home.
- **Content:** inset 32, filling the region. The `page-title` "Discover",
  then sections: the first 24 below it, the others 32 apart. A section has
  its `section` title, with a small ghost "See all" on the trailing edge
  where there is more, and its content 12 below.
- **Cover shelf:** one row of covers 16 apart, as many as fit; under each,
  its title at 12/500 in at most two lines, as for Similar books. The
  cover and the title open the book page.
- **Sections, in order:**
  1. **"For you":** Hardcover's Recommendations vibe, in its order. The
     muted line "Hardcover's recommendations from your library." sits
     between the title and the shelf. "See all 250" opens the vibe's page.
  2. **"Browse":** three groups, 16 apart, each a `label` and 6 below it a
     row of page chips (Chips), wrapped 6 apart: "By mood" (the 14 moods,
     in Hardcover's order), "By genre" (Fantasy, Science Fiction, Young
     Adult, Romance, Mystery, Thriller, Horror, History, Classics,
     Biography, Adventure, War, with their icons) and "Featured on
     Hardcover" (the vibes Hardcover features, by name). A mood or genre
     opens its tag page; a vibe opens its vibe page.
  3. **"Top picks":** Hardcover's Top Picks vibe. "See all 35" opens its
     page.
  4. **"Because you loved <title>":** the similar books of the reader's
     latest 5-star read, less the books in the library and those Top
     picks shows. No "See all": the shelf is all there is. A title too
     long for the line is cut with an ellipsis.
  5. **"Trending this month":** one book per series. "See all" opens
     Trending.
- **One lead.** The first shelf on the page is the lead: lg covers with
  the cover shadow. Every other shelf has md covers, flat. "Browse" always
  comes second, after the lead.
- **Without `read:vibes`:** "For you", "Top picks" and "Featured on
  Hardcover" hide, and "Because you loved" leads, as far as "Sign in
  again…". With no 5-star read, or no similar books for it, that section
  hides too and "Trending this month" leads.
- **A vibe not built yet:** "For you" (or "Top picks") keeps its title and
  its place, and shows, where the shelf would be, one muted sentence and
  the link: "Hardcover builds this list when you open it on
  hardcover.app." and "Open on Hardcover". The lead is then the first
  section that has covers.
- A book the reader said never to recommend (a vibe's page) is left out of
  "For you" and "Top picks" at once.
- **States:** a shelf loading is a row of skeleton covers; one that failed
  is one muted sentence with "Try again" in its place, and the other
  sections stay. A section with nothing to show hides without leaving a
  gap.
- **Trending** (over Discover, from "See all"): the `page-title`
  "Trending", the period segments (Week, Month, 3 months, Year, All time)
  and the list of catalogue rows, with the pane beside it in a wide window.

### A tag's page (a mood or a genre)

One page for both, over the page it was opened from (a book's mood line,
its Votes popover, a genre chip, Discover's Browse), with Back.
- **Header:** inset 32. The `page-title` is the tag ("Relaxing", "Science
  Fiction"). Under it a muted intro, at the prose measure:
  - a mood: "Books Hardcover readers call relaxing, the most votes first.";
  - a mood and a genre: "Mystery books Hardcover readers call relaxing,
    the most votes first.";
  - a genre: "Books Hardcover readers file under Science Fiction, the most
    votes first."
- **One select** (168) on the title's line, at the trailing edge, for the
  other kind: on a mood's page "Any genre" and Browse's genres, each with
  its icon; on a genre's page "Any mood" and the 14 moods. The choice is
  kept with the page, so Back finds it.
- **Order:** always by votes: for the mood, or for the genre while no mood
  is chosen. There is no sort control.
- **Rows:** catalogue rows, 16 below the header: an sm cover, the title,
  a muted tabular line with the author, the year and the votes the order
  goes by ("Travis Baldree · 2022 · 1,204 votes", "1 vote"), and the
  StatusChip of a book in the library. Enter or a double click opens the
  book page; in a wide window the selected book shows in the pane beside
  the list (Wide windows).
- **States:** skeleton rows while loading; a failed load is one muted
  sentence with "Try again"; a choice with no books is the empty state
  ("No matches", "Hardcover has no Mystery books readers call relaxing.").

### A series page

- **Header:** the series' name as the `page-title` (wrapping), "by" its
  author (a text button), the facts ("4 books · Ongoing", and "· Following"
  when followed), "You've read 1 of 4", the description in `prose` (three
  lines) and "Open on Hardcover". The books follow as catalogue rows.
- **Commands,** on the header's trailing edge, 8 apart: "Follow"
  (`bookmark-plus`), or a pressed "Following" (`bookmark-check`) that
  unfollows, with Undo; and "Add N books to Want to read…" (`plus`) while
  whole-numbered books that are out aren't in the library. It asks first
  ("Add 3 books to Want to read?", naming up to five books and counting the
  rest, and saying when books not out yet are left out); the result
  ("Add 3 books") is primary and Cancel has the focus. Without
  `write:social` Follow is disabled with its reason as a tooltip; without
  `write:library`, so is Add.

### A vibe's page

One of Hardcover's lists for the reader ("For you", "Top picks") or one it
features ("Hidden Gem Fantasy"), over Discover, with Back.
- **Header:** inset 32. The `page-title` is the vibe's name; "For you" for
  Hardcover's Recommendations. Under it a muted intro, at the prose
  measure: "Hardcover's recommendations from your library." for For you,
  "Popular books Hardcover picked for your taste." for Top picks, and a
  featured vibe's own description. Then one muted, tabular line: the count
  and the list's age, "250 books · Updated today".
- **Age.** Hardcover rebuilds a vibe only when its reader opens its page
  on hardcover.app, at most once a day; the app can't ask for it. The age
  is in whole local days (`dates`): "Updated today", "Updated yesterday",
  "Updated 3 days ago".
  - **Up to 7 days:** only the age.
  - **From the eighth day:** the line goes on, "250 books · Updated 9 days
    ago. Hardcover updates this list when you open it on hardcover.app.",
    and "Open on Hardcover" (an `ExternalLink` to the vibe's page) shows
    on the title's line, at the trailing edge. Never sooner: a fresh list
    has nothing to gain from the website.
- **Rows:** catalogue rows in Hardcover's order, 16 below the header: an
  sm cover, the title, the author muted, and the StatusChip of a book
  already in the library; nothing else marks it. Enter or a double click
  opens the book page; in a wide window the selected book shows in the
  pane (Wide windows).
- **"Never recommend…"** on For you and Top picks: in a row's context menu
  ("Open", "Want to read" for a book not in the library, a separator, then
  "Never recommend…" with the thumbs-down icon), and in the book's "more"
  menu while the book is open from a vibe, which is the keyboard's path.
  The book only, not its author. Without `write:vibes` it is disabled,
  with the reason (`format::needs_permission`).
  - It asks first, since it can't be undone from the app: "Never recommend
    “The Lady of the Lake”?" / "Hardcover will leave it out of your
    recommendations. You can't undo this from this app." / Cancel (with
    the focus), then "Never recommend" (primary: nothing is destroyed).
  - Once Hardcover has accepted it, the row leaves (Motion), and the app
    hides the book from every vibe's page and from Discover's shelves at
    once: Hardcover's own list drops it only at its next rebuild. The app
    remembers the book until a rebuilt list comes without it. There is no
    Undo. A failure is a notification, titled "Couldn't update your
    recommendations".
- **Not built yet:** the empty state under the title: the sparkles icon,
  "Your recommendations aren't ready" (For you; "This list isn't ready"
  for any other), "Hardcover builds this list when you open it on
  hardcover.app. It shows here after that.", and the link "Open on
  Hardcover".
- **States:** skeleton rows while loading; a failed load is one muted
  sentence with "Try again". Without `read:vibes` no vibe's page can be
  reached: Discover hides what opens them.

### Screens not in the mockup

Search, Stats, list pages, series and author pages, Settings,
About, sign-in and the dialogs use the same parts:
- a page title;
- catalogue rows with sm covers and StatusChips;
- rails and hairlines rather than boxes;
- display type only for a book's or person's name and for headline figures.

An author page's header puts the photo where a book has its cover (a large
avatar), with the name in `display-24`. Under the header, where a section
title would be, a small segmented "Books" | "Series" switches the list (an
author without series gets the "Books" title); the order select sits at its
trailing edge on Books only. Series lists every series, the most read first,
as sequence rows in the list's place, their covers on the book rows' spine:
an sm cover (their most-read book in it), the name as a text button that
opens the series page, and a muted line ("7 books · Complete · 3 read").
Stats charts use `accent` for the
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
  wrong place. A single truncated line keeps its place, but GPUI still lays
  it out as if it read left to right, so a bracket or a mark at the edge of
  a Persian run lands on the wrong side: a text button (a title in a row)
  sets a right-to-left label through `WrappingText`, clamped to one line.
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
| `scrollbar_thumb`, `scrollbar_thumb_hover` | `border-strong`, `fg-muted` |
| `tab_bar_segmented`, `tab_bar` | `chip` blended into `bg` (`tab_bar` paints a disabled segment) |
| `info`, `success` | `fg-muted`: a notification's icon and a success check stay ink |
| `overlay` | `scrim` |
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
controls at 12 px (`SmallControl`, `SmallIconButton`, `small_tab`),
dialogs on `raised` with the system's widths and keys (`open_dialog`,
`open_alert_dialog`, `bind_keys`; a confirmation's own footer focuses its
button by handle, with the ring only after keyboard input), and quiet
buttons on the text spine (`OnTextSpine`).

What follows is off the system and can't be fixed without patching the
kit. Keep it in step with `ui::kit`'s module doc; remove an item in the
commit that fixes it.
- Menus (`PopupMenu`) and the Select dropdown round at 6, not 8, and use
  the kit's own shadow, not `shadow-popover`.
- Menu items are 26 high (one every 28), not 32, with 12 px icons, not 16:
  the menu sets both itself.
- A menu's separator is a 2 px rule, not a hairline, and runs to the
  menu's edges, so menus keep to as few groups as they can.
- Menu items hover at 10% in dark mode: the kit's `accent` serves both menu
  items and ghost buttons, and ghost buttons take half of it there.
- The segmented tab bar's sliding indicator is always paper, and casts the
  kit's small shadow: a shadow on a control.
- A small segment pads its label 10, not 8.
- A disabled switch fades its track, not its thumb.
- A radio's circle takes no style from outside: unchecked and disabled, it
  is faint, most in dark mode. Settings shows a fixed theme as text rather
  than disabled radios; a form's radios are disabled only while it saves.
- The `DatePicker`'s field stays paper.
- A dialog keeps the kit's shadow.
- A notification's action button ("Undo" after a book comes off a list)
  is the kit's small size, so its label is 14 px, not 12.
- The kit's controls (buttons, inputs, selects, switches) draw a 3 px focus
  ring, not 2: the width is a private constant of the kit. The app's own
  rows, text buttons and components draw 2.
- Some of the kit's own measures are px, so they don't follow the
  interface scale (`ui_scale`) and look a little small at 1.25: a chart's
  labels (Stats, the goal's months) are a fixed 10 px; a segmented tab
  bar's height and padding (Search's tabs, Trending's periods, the progress
  dialog's units); a dialog's margins.
- The kit's buttons and switches show their ring whenever they hold focus,
  not only after keyboard input. A press never focuses one, so this shows
  only where focus is handed to it after a pointer action: a menu closing
  onto its trigger, a dialog closing onto its opener, Back. Buttons whose
  focus the app keeps (`ui::kit::KeyboardRing`: the alert footers, and the
  book view's buttons focus is handed to after an in-place change) follow
  the rule; a switch can't be given a handle.
- Empty-state icons are Lucide's, at stroke 2, not 1.5.
