---
name: hardcover-design
description: The design system of Hardcover Desktop, the app in this repository. Read before designing, building, changing or reviewing anything visible in crates/app - a screen, layout, spacing, colour, type, cover, chip, star, control state, overlay, empty state or interface copy - and when asked what the design rules are or whether a UI decision follows them. It replaces the generic gpui-kit Design Guides; the gpui-kit skill is still the reference for GPUI and component APIs.
---

# Hardcover Desktop design system

Hardcover Desktop is a personal reading app: a library of books that the user
reads, tracks and rates. Its design system is built for that, not for a
generic desktop tool. This skill is the only design authority for the app.
The `gpui-kit` skill tells you *how* to build something with GPUI and its
components; this skill tells you *what* to build and how it must look.

## Read

- [references/design-system.md](references/design-system.md): the system
  itself. It covers the thesis, tokens (colour, type, space, radius, frames,
  elevation), covers, components, states, screen layouts, right-to-left
  text, and copy. Read it in full before a new screen or a redesign. For a
  narrow change, read "Thesis", "Tokens" and the section for the change.
- [references/desktop-practice.md](references/desktop-practice.md): the
  desktop interaction practice the system builds on. It covers task first,
  hierarchy, alignment, command surfaces, Button vs Link, overlays, motion,
  interface language, accessibility, and the review checklist. Read it before
  adding a control, an overlay or any copy, and run its review checklist
  before you finish.
- [docs/design/mockup.html](../../../docs/design/mockup.html) (repository
  root): the reference mockup. Every screen and the token sheets are built
  from the same CSS variables as this system. Open it in a browser when you
  need to see a layout. When the mockup and the reference disagree, the
  reference wins; fix the mockup in the same change.

## The app follows this system

Every screen is built from the tokens and the shared components in
`crates/app/src/ui/`. What the kit doesn't let the app reach is listed under
"Kit limits" at the end of `design-system.md`.

- New UI follows this system. Compose the shared components; if a part is
  missing, add it to `ui/` (and the gallery) rather than beside a screen.
- A value comes from the token module (`design::colors`, `design::size`,
  `design::TypeRoles`), never from a literal.
- When a change meets a kit limit, add it to "Kit limits" and `ui::kit`'s
  module doc in the same commit; remove one in the commit that fixes it.

## Non-negotiables

A floor, not a substitute for the reference.

- **Serif speaks for the book, sans speaks for the app.**
  - Literata: book titles at title size and up, the Home greeting, large
    figures, and description prose.
  - Inter: everything else.
  - Fonts are static faces only: GPUI on Linux draws a variable font's
    default instance only.
- **Tokens, not values.** No raw colours, and no `px(...)` for ordinary
  layout. The exceptions are covered in the reference: hairlines, cover
  geometry, and the window size.
- **One accent, ink blue, spent scarcely.** It goes on progress, the default
  commit, focus, the selected sidebar icon, and links that open the browser.
- **Gold is the user's own rating, and nothing else.** Catalogue figures draw
  the same star in muted ink.
- **Chips are neutral pills.** Statuses differ by their icon, never by colour.
  The only colour on a chip is a content warning's warning icon.
- **Covers are books.** They are always 2:3 and come in six sizes. They have a
  hairline edge, a shadow only where the cover leads its row, and the cloth
  cover when there is no image.
- **One treatment per state.** Hover, pressed or selected, focus, disabled and
  loading each look the same everywhere.
- **Button for in-app actions, link for the browser.** Primary marks only the
  default commit of a decision area. A command that opens a dialog ends with
  `…`.
- **Copy:** sentence case. Name the object and the verb: `Remove "Dune" from
  your library?` with a `Remove` button.

Finish by running the review checklist in `desktop-practice.md`.
