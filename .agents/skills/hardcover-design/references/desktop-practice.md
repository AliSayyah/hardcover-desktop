# Desktop practice

The interaction rules the design system stands on. They are adapted from the
GPUI Kit Design Guides, keeping what holds for this app. Where this file and
`design-system.md` overlap, `design-system.md` has the app's values; this
file has the reasoning and the rules for situations the system doesn't
cover.

## Before you build

Look at the nearest existing feature, the theme tokens and the component's
source in the `gpui-component` crate. State the primary task, who owns the
state, how the parts are composed and the keyboard path before writing code.
Never infer a GPUI API from React or web examples, and never invent a method
because its name seems plausible. A screenshot that looks right isn't proof:
keyboard, focus, dynamic content, both themes, resizing and failure states
are part of the design.

## Start from the task

Before drawing or changing a screen, write down:
- the reader's primary task;
- the object being viewed or changed (a book, a shelf, a list, a goal);
- the actions that must stay immediately available;
- the information needed to decide;
- the empty, loading, error, offline, read-only and missing-permission
  states;
- the keyboard path through it.

Organise the window around those answers, and around the reader's objects:
books, shelves, lists, reads, goals. Don't organise it around Hardcover's API
or the app's modules.

- **Weight follows importance.** The primary task gets the most space and
  the strongest type. A core result must not shrink to a count in a corner
  while secondary content fills the page.
- **Name every action's parts.** For each proposed action, name its object,
  its current state, its scope and its result. If the screen doesn't show
  them, the action is premature.
- **Don't expose a capability just because the API has one.** First design
  how its object enters the reader's mental model.

## Hierarchy

- **Few levels:** page title, section, heading, body, muted metadata, labels.
  Size, weight, spacing and hairlines come before colour or containers.
- **Emphasis is a budget.** A surface has one focal point. If everything is
  bold, boxed, coloured or badged, nothing is.
- **Judge the whole feature, not each control.** Hide colour mentally: the
  primary task, the current selection and the next action must still be
  obvious from structure alone.

## Alignment

Alignment is structure, not polish.

- **Spines:** every surface has a few. They are shared leading and trailing
  edges, text baselines, and fixed lanes for counts, trailing figures and
  actions. Peers attach to the same spine, even when they are different
  components.
- **Lanes survive missing content.** Rows reserve the same lanes for cover,
  title, figures and actions, so a row with a missing item doesn't move its
  label. Keep icons in fixed slots so labels don't shift.
- **Indent only for real hierarchy.** When a nested level ends, return
  exactly to the parent's spine.
- **Align text and numbers properly.** Mixed sizes in one row align by
  baseline. Numbers that compare are right-aligned and tabular. Prose and
  names are left-aligned.
- **One owner per edge.** A hairline is drawn by one of the two regions it
  separates. A scrollbar sits against the edge of the region that scrolls,
  not inside its padding.
- **Check equality with a measurement, not by eye.** A one-pixel drift
  between two edges or gaps that are meant to be equal is a defect. Fix it
  at the shared owner (the token, the shared component inset), not with an
  offset.

## Layout

- **The shell:** the sidebar workspace with a master–detail shelf. Global
  navigation stays put while content changes.
- **Overflow:** the work area takes the remaining space with `flex_1()` and
  `min_w_0()`/`min_h_0()` wherever children must shrink. Use `Scrollable`
  or a virtual list; don't rebuild scrolling from nested divs.
- **Three sizes per region:** every major region has a minimum at which its
  task still works, a comfortable default, and a rule for taking surplus
  space. The collection stays scannable, and the detail takes the surplus.
  Persist a split the user drags, and clamp it to the window when restored.
- **As the window narrows:**
  1. keep the primary task;
  2. let resizable regions reach their minimum;
  3. collapse secondary labels, or wrap the header's cover above its text;
  4. move rare actions into a menu;
  5. scroll only the region that overflows.

  Never hide an action without another path to it.
- **A clipped region clips its focus ring.** Never trade away keyboard
  visibility to simplify overflow.

## Components and composition

- **Compose before you invent.** Start from gpui-component's components and
  the app's own (cover, chips, stars, rows, `WrappingText`). Build a new
  primitive only when its behaviour is genuinely new.
- **Keep repeated patterns in one place.** When a pattern carries the app's
  language or policy, wrap it in an app component. Examples: the cover, a
  StatusChip, the rating control.
- **Each component has its role.** A menu, a popover, a select and a dialog
  are not interchangeable boxes. Each owns its own focus, keyboard,
  selection and dismissal behaviour.
- **Keep a family's geometry.** Menu rows share height, padding, icon and
  check slots, separators and state treatment. Never imitate a menu with a
  custom popup whose spacing only approximates it.

## Interaction states

A control predicts its result. Its label names the action and the object,
its state shows whether it is available, and its feedback confirms the same
outcome.

- **Respond at once.** Prevent a second submission while work runs. Show the
  result next to the object that changed. Add a message only when the result
  itself isn't visible.
- **Every interactive control is designed for every state:**
  - rest, hover and pressed;
  - open, while its popup is showing;
  - focus-visible, selected, disabled, loading and error.

  Apply the treatments in `design-system.md` → States.
- **Selection is information.** Tabs, segmented controls, selectable rows,
  filters and destinations show a persistent selected state through the
  item's own fill and weight. Never use a leading bar or a one-sided border
  as the marker.
- **A segmented control is one shape.** Its end segments follow the
  container's corners, and each boundary is one divider of even weight.
- **Destructive actions:**
  - prefer undo for reversible changes;
  - confirm irreversible ones in an alert dialog that names the object and
    the consequence.

### Pointer

- The default arrow cursor goes on buttons, rows, tabs and menu items. The
  pointing hand is only for links and content that behaves like a link.
  Text, resize and grab cursors appear only during that manipulation.
- Hover effects stay modest. Never reveal the only copy of an essential or
  destructive action on hover.

### Command surfaces

- **Frequent and primary actions stay visible,** as labelled buttons.
- **Secondary actions for a region** go behind a visible menu trigger
  ("Lists", "more").
- **Commands on the object under the pointer** go in a context menu. A
  context menu is never the only path to a command.
- **Important commands get an Action and a key binding** when they have a
  natural keyboard form, and the binding shows in menus and tooltips.
- **Hover-revealed icons** are only shortcuts to commands that are reachable
  elsewhere.
- **Same command, same everything:** the same verb, icon, shortcut, enabled
  state and result wherever it appears. Dangerous items are labelled and set
  apart.

### Button means in-app action; link means the browser

- Use a button for anything that changes the app's state, confirms,
  submits, runs a command, or navigates inside the app. Pick the variant by
  local hierarchy: default, ghost, text or icon.
- Primary marks only the default commit of a decision area (the Enter
  action). It is never "the only button" or "the button we want clicked".
- An underlined link is only for an external target: a URL or an email
  address. Never style an app command as a link to make it look quiet.
- "View" doesn't make an in-app destination a link. A book, an author or a
  series opens through a button, row or text button.

## Feedback and overlays

Choose the smallest surface that fits:
- **Tooltip:** a short explanation, or a shortcut, for an unfamiliar symbol
  or an ambiguous scope. Don't restate an obvious label.
- **Popover:** contextual controls, or detail on demand, that don't
  interrupt.
- **Menu:** a compact list of actions.
- **Notification:** asynchronous status that needs no decision.
- **Dialog:** a focused decision or a short form.
- **Alert dialog:** confirming a consequential, irreversible action.

Rules for overlays:
- Don't stack overlays. Escape dismisses the topmost one, and focus returns
  to its trigger or the next logical target.
- An overlay's action refers to something the overlay actually shows. Hide
  an action that doesn't apply, or disable it with a reason.
- A footer holds only actions that apply to the whole surface.
- An alert interrupts the hierarchy. Use it for exceptional information the
  task needs now, never for tips, descriptions or empty space.

## Motion

- **Purpose:** motion explains a change in appearance, dismissal, expansion
  or place. It is never ambient decoration, and nothing depends on it to be
  understood.
- **Keep it cheap:** prefer opacity and transforms to animating layout.
  Respect reduced motion.
- **Interruptions:** an interrupted transition reverses from where it is.
  It doesn't restart.
- **Continuity:** while the pointer moves among related targets, keep a
  preview surface steady and update it in place. Don't fade it out and in on
  every move.
- **Feedback reflects reality:** a skeleton only while content is genuinely
  loading. Moving between content that is already loaded is instant.
- **The app's motions** are listed, with their timings, under Motion in
  `design-system.md`. Add a new one there before building it.

## Lists and large collections

- Dense doesn't mean cramped. Keep row identity (cover, title) stable, and
  keep comparable values aligned and tabular.
- Distinguish focus, hover, the selected row and multi-selection.
- Sorting and filtering are visible and reversible.
- Selection follows the item's identity (the book), not its index, across
  filtering, reordering and syncs.
- Virtualise large collections without changing keyboard behaviour.
- Every collection has a useful empty state.

## Interface language

Words are part of the structure. Write a feature's vocabulary as a system:
destinations, objects, commands, states and outcomes. Use the app's lexicon
(`design-system.md` → Copy).

- **Let context carry context.**
  - Don't repeat what the surface already says: "Lists", not "List
    management".
  - Add words only when they change the decision: the scope, an
    irreversible consequence, a prerequisite, a way to recover.
  - Avoid wrapper words such as Management, Module, Page, Function,
    Operation and System.
- **Buttons** are short and name the result: "Save", "Remove", "Mark as
  read".
  - "Cancel" leaves without committing.
  - "OK" only acknowledges pure information.
  - A longer label is right when it separates choices people could confuse:
    "Remove from this list" versus "Remove from library".
- **Confirmation dialogs:**
  - **Title:** the decision, such as `Delete “Book club picks”?`.
  - **Body:** only new scope, consequence or recovery.
  - **Buttons:** Cancel and the result; the destructive result in danger.
  - Avoid "Notice", "Warning", "Are you sure…", "Please note", and
    "successfully".
- **Punctuation:**
  - Labels, buttons, menu items, tabs, headings and placeholders take no
    final period. Full sentences do.
  - Never use an exclamation mark in routine messages.
  - Use the single `…` character, on every command that opens a dialog or
    needs more input before it completes. An immediate command takes none.
- **Errors** say what happened and what to do next ("Couldn't save. Check
  your connection and try again."). Skip success messages when the result is
  visible.
- **Long text:** copy must survive longer strings and larger text. Never
  build a sentence from fragments, and truncate only where there is a way
  to see the whole text (a tooltip, the book page).

## Accessibility checklist

Before a screen is done, check that:
- every action is reachable and operable by keyboard, and the focus order
  follows the visual and task order;
- focus stays visible, is never clipped, and returns after overlays;
- controls have accessible names, and icon-only controls have tooltips when
  their meaning, scope or shortcut needs them;
- text that carries information meets AA contrast (`fg` or `fg-muted`,
  never `fg-faint`);
- status never relies on colour alone, and disabled and read-only states
  are distinguishable;
- labels, errors and help sit next to their controls;
- the screen works with longer titles, right-to-left titles and larger text;
- pointer targets are comfortable even in dense rows.

## Review checklist

A review judges decisions, not a component inventory. Ask, in order:

1. **Is the task clear?** Can a new reader see the purpose, the primary
   action and the next step without guessing?
2. **Does every action keep its promise?** Do the label, control, state,
   scope, feedback and result describe one outcome?
3. **Is the hierarchy decisive and restrained?** Does the focal point get
   the space it deserves, while accent, bold, chips and primary buttons stay
   scarce?
4. **Is the book the book?** Is the serif only on book content? Are the
   covers 2:3 at a system size? Does gold appear only on the reader's own
   rating, and are chips neutral?
5. **Could it do less, better?** Can an entry point, option or state be
   removed, combined or deferred without weakening the task?
6. **Is the structure exact?** Do peers share spines and equal gaps (to the
   rendered pixel), and are the hairlines and scrollbars where they belong?
7. **Does it follow the tokens?** No raw colours or sizes? Is every state
   the system's one treatment, and does it look right in both themes?
8. **Does it hold in every state?**
   - keyboard and focus;
   - empty, loading, error and missing-permission states;
   - long and right-to-left titles;
   - the minimum window size;
   - reduced motion.
9. **Was it tested in a real window?** Run it with representative content:
   the demo build (`cargo run -p hardcover-desktop --features demo`), in
   light and dark.
