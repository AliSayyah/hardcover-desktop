# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Hardcover Desktop is a Linux desktop client for [hardcover.app](https://hardcover.app). It is written in Rust (edition 2024) with gpui-kit 0.10. The licence is MIT, and the app id is `io.github.alisayyah.HardcoverDesktop`.

## Commands

```sh
cargo run -p hardcover-desktop                    # run the app (signs in with OAuth, or restores the keyring session)
cargo run -p hardcover-desktop --features demo    # a sample library: no account, keyring, settings or real cache; only covers are downloaded
cargo test --workspace                            # unit tests; live-API tests are #[ignore]d
cargo test -p hardcover-core <name>               # one test, e.g. `unauthorized_refreshes_once_and_retries`
cargo clippy --workspace --all-targets
cargo fmt --all

# Live API: needs network and a token
HARDCOVER_TOKEN=... cargo test -p hardcover-core --test live_read -- --ignored
HARDCOVER_TOKEN=... cargo run -p hardcover-core --example smoke   # read-only
cargo run -p hardcover-core --example oauth_login                 # CLI sign-in, stores the session in the keyring
cargo run -p hardcover-core --example oauth_check -- <book-id>    # scope + write cycle on a book NOT in the library
cargo run -p hardcover-core --example api_check                   # check the live API for changes; updates schema/
cargo run -p hardcover-core --example api_check -- --check        # same, without writing; exit 1 on any change
```

The `justfile` wraps these; `just` lists the recipes.
- `just check` runs fmt-check, clippy (`-D warnings`) and the tests.
- `just ci` adds `cargo deny`, the packaging validation and actionlint.
- `just package` builds the release tarball into `target/package/`.
- `just api-check` is `api_check --check`; `just schema-update` is the run that rewrites `schema/`.
- `just release patch` makes a whole release, from the PR to the published GitHub release (below, under CI and releases).
- Only the live-API recipes load `.env`; `just run` never does, so a token there can't bypass OAuth.
- The recipes that write to the real account, and `oauth-login`, ask for confirmation.

Keep the justfile in step when a command here changes.

`api_check` is local only and is not run in CI. It uses `HARDCOVER_TOKEN` if set, otherwise the app's keyring session; both see the same schema. It does three things:
1. Introspects the live schema and lists added, removed and changed types and fields.
2. Validates every document in `queries::ALL` against the live schema.
3. Checks the public scope map (`https://api.hardcover.app/capabilities.json`): every scope in `auth::DEFAULT_SCOPES` must still exist, and together they must grant every top-level field, gated column and gated argument the documents use. Some scopes gate only arguments: `write:reviews` grants no operation, just the `review*` fields of `insert_user_book`/`update_user_book`. Arguments inside a variable's value can't be seen statically, so an ungranted one there is a note, not a failure. The same check runs offline against `schema/` in the unit tests (`the_requested_scopes_cover_every_document`).

It then rewrites `schema/hardcover.graphql` and `schema/hardcover-capabilities.json`; review the change with `git diff schema/`. It exits non-zero if a document breaks or a scope no longer covers it. Never edit `schema/` by hand.

Environment variables (see `.env.example`; `.env` is gitignored and must never be committed):
- `HARDCOVER_TOKEN`: a personal access token. For development only: it bypasses OAuth in the app, and the examples and live tests use it.
- `HARDCOVER_CLIENT_ID`: overrides the compiled-in OAuth client id, `auth::DEFAULT_CLIENT_ID`.
- `HARDCOVER_THEME=light|dark`: forces a theme, over the choice in Settings, which then shows it as unavailable.
- `HARDCOVER_NO_UPDATE_CHECK=1`: turns off automatic update checks, whatever Settings says. Debug builds never check on their own; "Check for updates" (account menu, or Check now in Settings) always works.
- `live_writes` example: `HARDCOVER_LIVE_WRITES=yes`, `HARDCOVER_LIVE_PHASE`, `HARDCOVER_TEST_BOOK_ID`.

Live write tests run against a real account. Restore everything afterwards. Removing a book does not delete its `reading_journals` rows; clean those up separately, which needs a token with journal scope.

## Architecture

The workspace has two crates, in `crates/*`.

**`hardcover-core`** has no GPUI dependency, and every call in it is **synchronous and blocking**: there is no async runtime.
- `api`: `Client` over the GraphQL endpoint `https://api.hardcover.app/v1/graphql`.
  - `queries.rs` holds every GraphQL document as a const. A unit test validates each one against `schema/hardcover.graphql`, an introspected SDL loaded through apollo-compiler.
  - When you add or change a query, register it in `queries::ALL` so the test and `api_check` cover it. `api/sdl.rs` holds the validator; `api_check` includes the same file through `#[path]`, because apollo-compiler is a dev-dependency.
  - `wire.rs` holds the raw response shapes; `model.rs` holds the public types.
  - `UserBookFields` (the sync and every library write) includes the book's `book_series`, so an entry knows its series without a book page. Checked live: a full sync of about 800 entries took under 5 s with it.
- `http.rs`: a small transport seam. Production uses ureq. `http::fake::FakeTransport` scripts responses for the tests in `api/tests.rs`.
  - All fixtures must be synthetic: no real user data.
  - `HttpRequest` deliberately does not implement `Debug`, because it carries tokens.
- `ratelimit.rs`: a client-side token bucket with burst 8 and a refill of 1 per second. Hardcover's own limits are 60 requests per minute, burst 10, and at most 10 top-level fields per request.
  - On a 429 the client waits and retries once.
  - On a 401 it calls `TokenSource::on_unauthorized` once, which refreshes the token, then retries.
- `auth`: OAuth for a native public client.
  - Authorization code with PKCE S256, redirected to a loopback `http://127.0.0.1:<port>/callback`. `state` and `iss` are checked.
  - `Session` stores tokens in the system keyring (`KeyringTokenStore`, secret-service) and refreshes them proactively. Refresh tokens are single-use, so refreshes are serialized.
  - `StaticToken` is the `HARDCOVER_TOKEN` path.
  - Scopes: `DEFAULT_SCOPES` is `read:me read:catalog read:library write:library write:reviews read:goals write:goals read:lists write:lists read:journal read:social read:users read:notifications write:notifications`; the names are constants in `auth::scopes`. Adding one makes every existing session show the "sign in again" notice, so add new scopes in batches. `GrantedScopes` checks a grant, following the scope map's `implies` (`read:lists` covers `read:lists:public`; `all` covers everything); a test keeps its table in step with `schema/hardcover-capabilities.json`.
  - The granted scope is recorded with the tokens. A code exchange whose response has no `scope` records the requested scopes (RFC 6749 §5.1); a refresh without one keeps the recorded scope. Tokens with no recorded scope were saved by 0.1.x and count as `LEGACY_SCOPES` (the four it requested).
  - `Session::replace` swaps a new sign-in's tokens into the existing session (the "sign in again" path), so every `Client` on it moves to them. Never start a second `Session` next to a live one: its refresh would write the old grant back over the new one. The old grant is not revoked. `replace` refuses once the session is signed out, so a sign-in that finishes late can't sign the user back in; `Session::discard` revokes tokens nobody will use.
- `store`: `LibraryCache`, a SQLite database at `~/.local/share/hardcover-desktop/library.sqlite3`.
- `covers`: `CoverCache`, on disk at `~/.cache/hardcover-desktop/covers`.
- `demo` (the `demo` feature, and in tests): `demo::client()` answers every GraphQL operation from `demo/catalogue.json` through a fake transport, so queries, parsing and paging run as they do against Hardcover. Writes fail with a read-only error.
  - Series and author pages and similar books are derived from the catalogue, with the demo's own author and series ids. Books have no ratings distribution or alternative titles: the catalogue does not record them. `demo::cover_cache()` passes only the catalogue's cover URLs through to the real network and refuses every other URL.
  - The books are real popular ones; their ids, facts and cover URLs come from Hardcover. The descriptions are our own short text, not publisher blurbs. The reader, their shelves and their lists (an Owned list and three others) are invented: no real users.
  - Covers are downloaded from Hardcover at runtime, only for the catalogue's own URLs. Never commit the images.
  - When you add a query, give it an answer in `demo::answer`; the `every_document_has_an_answer` test fails until you do.
- `prefs`: `Preferences` (theme, automatic update checks) in `~/.config/hardcover-desktop/settings.json`. Loading never fails: a missing or garbled file gives the defaults, and every field has `#[serde(default)]`, because a self-updated app reads the file an older version wrote. Writes are atomic (temporary file, then rename). The file survives signing out.

**`app`** (the package is named `hardcover-desktop`) is the GPUI UI.
- `main.rs` sets up actions, keybindings and the window.
- `app_state.rs` holds the GPUI globals:
  - `Services` (client, session, cache and a snapshot of the granted scopes) exists only while signed in. Feature code asks `Services::global(cx).has_scope(scopes::READ_GOALS)`; it is cheap enough for `render`. Never ask the `Session` for its scope from the UI thread: its lock is held across a token refresh. A personal token (`HARDCOVER_TOKEN`) and the demo count as having every scope.
  - A feature whose scope is missing hides, or shows a disabled state with `format::needs_permission`. A session that lacks any of `DEFAULT_SCOPES` gets the sidebar notice (`sign_in/reauthorize.rs`): "Sign in again…" first finds out whose session it is (`me()` if the library doesn't know yet), runs the browser sign-in, checks that the same account approved (another account's tokens are revoked), and calls `Session::replace`; the cache and the library stay. Signing out cancels a sign-in still waiting on the browser. Turn a failed request into a message with `Services::error_message`, not `format::error_message`: on `insufficient_scope` it records the refused scope (`Services::scope_refused`, which counts it as missing) so the notice appears, and otherwise says to sign out and in again.
  - `CoverService` lives for the whole run.
  - `startup()` picks the auth path in this order: `HARDCOVER_TOKEN`, then the keyring session, then the sign-in screen.
  - With the `demo` feature, `startup()` always returns the demo client with an in-memory cache, before anything real is opened. Covers go to a temporary directory, and preferences are not saved.
- Every core call runs inside `cx.background_spawn`, and its result is applied on the foreground. Never call `hardcover-core` on the UI thread. The one exception: `main` reads the preferences before the event loop starts, because the first frame needs the theme.
- Covers are GPUI `Asset`s loaded with `window.use_asset`:
  - `CoverThumbnail` is 144 px and `CoverImage` is 480 px.
  - Decoding happens on one dedicated `cover-decoder` thread, not the background pool. Decoding on GPUI's 16 pool threads bloated glibc's per-thread malloc arenas: 62 MB versus 495 MB on a 442-cover benchmark. Keep large image decodes on that thread.
  - Scaling uses `fast_image_resize` (bilinear, the same Triangle filter `image` used). `cover_quality_tests.rs` checks the output against `image`'s resize; set `COVER_QUALITY_DIR` to run it over a real cover cache.
  - Each scaled cover is saved in `covers/scaled/v<N>/`: thumbnails as lossless QOI, opaque detail covers as JPEG q92. A relaunch reads these copies and decodes nothing. Bump `SCALED_VERSION` when the decode output changes; older versions are deleted at startup.
- `library/model.rs`: `LibraryModel` is the one entity every library view reads.
  - It loads the cache first, then syncs page by page.
  - Writes return the full entry, which replaces the local one and is upserted into the cache.
  - A sync that started before a write re-applies that write when the sync lands (`collection::merge_sync`).
  - `begin_write`/`end_write` track the writes in flight, per control.
  - Reviews are not part of the sync (review text can be long). The model fetches an entry's review when a book page shows it, keeps it for the run, and fetches it again after the next sync. A review write's response replaces it. Each fetch and write bumps the slot's generation, so a fetch that finishes after a later write is dropped. The app writes only `review_markdown`: a review written on the website (only `review_slate`, with Hardcover's plain `review_raw`) is shown read-only, with a link to edit it on Hardcover.
- `fonts.rs`: the interface font, Inter, bundled from `assets/fonts/inter/` (static faces: regular, italic, medium, semibold, bold) and set on the theme. Don't switch back to the system font: on Linux it is often a variable font (Adwaita Sans), and GPUI's Linux text system draws only a variable font's default instance, so every bold or semibold run renders regular. Ship the OFL licence with any new face; `package.sh` installs it.
- `bidi.rs`: `WrappingText`, for catalogue and user text that may wrap (titles, names, descriptions). GPUI's Linux layout draws wrapped right-to-left text (Persian, Arabic) mostly outside its box; a line that doesn't wrap is drawn correctly. So right-to-left text is wrapped word by word in a reversed flex row, keeping Latin runs ("Harry Potter") together; left-to-right text takes GPUI's own path. Use it instead of a plain string child wherever such text can wrap; single-line `truncate()` text doesn't need it. Markdown reviews and text fields still use GPUI's layout.
- `star.rs`: the rating star, a rounded solid SVG (`assets/icons/star.svg`) used instead of Lucide's. Gold (`star::color`, amber) marks only the user's own rating: the book page's `StarRating` control and the library rows. Catalogue figures (the Hardcover average, the ratings breakdown) draw it in the muted text colour. An empty star is the same shape in a faint solid (`star::empty_color`), so a half star splits cleanly.
- `chip.rs`: the app's chips, neutral rounded pills on the muted surface. `StatusChip` is a book's library status at the end of a row (search, Discover, series, author and list pages), with its shelf's sidebar icon (`chip::status_icon`), so statuses differ by shape, not colour. `TagChip` is one catalogue tag on the book page; content warnings carry the theme's warning icon, the only colour. Use these, not `Tag` directly, for any new status or tag, and don't colour-code them.
- `preferences.rs`: the `AppPreferences` global (`PreferencesModel`) and the theme. `follow_theme` re-applies the theme when the desktop's appearance or the preference changes. `UpdateModel` observes the model to start or stop automatic checks.
- `settings_view.rs` (Settings dialog, `Ctrl+,`) and `about.rs` (About dialog). Both open from the account menu.
- `goals/`: reading goals. `GoalsModel` loads the goals at start and on Refresh, and again 3 s after the library changes (outside a running sync), because Hardcover recalculates progress itself. `GoalsSection` is the sidebar's "Reading goal" section; `goal_view.rs` holds the goal dialog (target, delete) and the new-goal form. All of it hides without `read:goals`, and writes are disabled without `write:goals`. Goals are not cached.
- `lists/`: the user's lists. `ListsModel` is owned by the `Workspace`, created in `Workspace::new` before the views, and reached through `ListsModel::global`, a weak handle removed on sign-out; results that land after signing out are dropped. It loads the lists at start and on Refresh, and the membership index (which books are on which list, `Client::list_memberships`) the first time a book page is shown; Refresh reloads the index only once a book page has needed it. Writes update both from their own results, and a load that started before a write is run again. Past five pages of 1,000 rows the index is marked incomplete, and once it lands the book page's book is looked up alone (`book_list_memberships`).
  - `sidebar.rs` builds the sidebar's "Lists" section (the Owned list first, then by name); a list is a `Destination::List`, shown by `list_view.rs` a page of 100 books at a time (`ListDelegate::load_more`). Loaded books are kept for the session until Refresh. Delete on a row takes it off the list.
  - `book_lists.rs` holds the book page's "Lists" menu (every list but Owned, checked where the book is) and the Owned switch under "Your book". `list_dialog.rs` is the create/edit form.
  - Without `read:lists` it all hides and the book page offers only the one-way "Add to Owned list…"; list writes are disabled without `write:lists`. Turning Owned on needs `write:library`, off `write:lists`. Lists are not cached.
- `shell.rs`: `Shell` switches between Starting, SignIn and Workspace. `Workspace` contains the sidebar (shelves by status, the reading goal and lists) plus the current destination (Home, search, Discover, Stats, a shelf or a list) and any catalogue page over it. Home is the destination at launch (Ctrl+0); Ctrl+1 is the Currently Reading shelf.
  - Keyboard hints stay hidden until Ctrl is held for half a second; then each sidebar destination shows its shortcut in place of its count (`Kbd::global_binding_for_action`, so hints follow the bindings in `main.rs`). Give a new sidebar destination with a shortcut its action there too.
- `home/`: the Home destination. It asks Hardcover for nothing: `digest.rs` works out everything from the library and the date (pure, tested), and `HomeView` caches it per library revision and day, like Stats.
  - Continue reading: each Currently Reading book, the most recently updated first. Pace is pages per day since the open read started, counting the start day, and only from the third day; the finish date it implies is left out past a year. Audiobooks get no pace. A book whose entry hasn't changed in 7 days says "Last updated N days ago", and the rows are ordered by that time. Reads have no `updated_at` of their own, so this assumes a progress update changes `user_books.updated_at`: likely (live entries in progress had times matching their last reading) but not verified with a write. The latest Paused book is shown too; it has no Resume, because moving it to Currently Reading would start a new read.
  - "Update progress…" opens `progress_dialog.rs`, which checks the input with `format::parse_progress`, as the book page does. "Mark as read" (after a confirmation: the read ends today, and undoing it starts another read) and "Start reading" call `set_status`.
  - Rate what you finished: Read books finished in the last 60 days with no rating (`digest::to_rate`, at most four rows). A star click saves at once with `set_rating`, resending the status. A book rated on Home keeps its row until Home is next opened from the sidebar (`HomeView::opened`), so a slip can be undone and nothing moves under the pointer; coming back from a book page keeps it too. The stars stay enabled while a rating saves, so they keep the keyboard focus; a change made meanwhile is dropped. It hides without `write:library`. Every demo book is rated, so the demo never shows it.
  - The goal card shows one current goal (a books goal first), the pace tick, the books being read that should finish before its end (not idle ones), and books per month for a calendar-year goal. It hides without `read:goals`.
  - Up next: the next book in a series, then the newest Want to Read books. A book counts as next only if a finished book of its series sits at most one position before it, with nothing on Want to Read, Currently Reading or Paused in between, so a gap in the library suggests nothing. Series come from the library sync (`LibraryEntry::series`); caches written before it was synced have none until the next sync.
- `search/`: Search, with Books, Authors and Series tabs over Hardcover's search, debounced and paged (`results.rs`). Book results start with the library's own matches (`library_matches.rs`): every query word must begin a word of the title, authors or series, at least two letters in all, at most five books, Ignored ones left out. They are found on this computer as the query changes, so they show before Hardcover answers. The list then has two sections, "In your library" and "On Hardcover" (Hardcover's results without those books); with no library matches it has one. Look rows up through `ResultsDelegate::hit_at`/`index_of`, never by a flat index. A library change drops matches that left the library but adds none until the next query, so the selection doesn't move.
- `catalog/`: series and author pages, and the page history over the destination.
  - Any view opens a page with `navigation::open(Page::…)`; `Workspace` owns the `PageStack` and shows its top page with a Back bar. Back (`GoBack`: Escape on a page, Alt+Left anywhere) pops it; a sidebar destination clears it.
  - Each page is one request (`SeriesPage`, `AuthorPage`), cached per id for the session. Similar books on the book page are one more (`BooksByIds`), fetched after the details with a delay.
  - The book detail pane's catalogue parts (contributor and series links, ratings, tags, similar books) are in `book/detail_view/catalog_sections.rs`.

## Hardcover API quirks the code relies on

- Status ids: 1 Want to Read, 2 Currently Reading, 3 Read, 4 Paused, 5 Did Not Finish, 6 Ignored.
- Setting a rating without a status moves the book to Read. `set_rating` therefore takes a `keep_status` and resends the current status. `set_review` and `delete_review` do the same. Verified live: a review sent with the current status leaves the status alone and starts no read.
- Review arguments (`review_markdown`, `review_slate`, `review_has_spoilers`, `reviewed_at`) need `write:reviews`, which gates only those arguments, not an operation. `UPDATE_USER_BOOK_REVIEW` and `DELETE_USER_BOOK_REVIEW` write them out instead of passing them in `$object`, so the scope test sees them. `UserBookUpdateInput` has no `review_raw`. Hardcover ignores `null` or `""` for `review_markdown` and `null` or `{}` for `review_slate`: none of them clears a review. Writing an empty Slate document to `review_slate` does (`review_markdown` becomes null, `review_raw` empty, `has_review` false), so `delete_review` sends one. `reviewed_at` can't be cleared, and means nothing once `has_review` is false. Saving `review_markdown` fills `review_raw` and `review_slate` too. Reading other users' reviews needs `read:social`, which the app does not request.
- Reading an entry right after a write can return stale data. Use the entry from the mutation's own response, and refetch only when it is missing.
- Moving a book to Currently Reading starts a new read. A progress update with no open read moves the book to Currently Reading first.
- An entry can have many reads. The current one is the newest unfinished read, else the most recently finished: a past read added later gets the highest id.
- `edition_owned(id)` takes an edition id and only adds it to the user's Owned list (creating the list the first time, and posting to their feed). The Owned list is the user's `lists` row with slug `owned`, as the official SDK (github.com/hardcoverapp/hardcover-sdk) finds it. Taking an edition off is `delete_list_book(id)` (`write:lists`), which takes the `list_books` row id, not the book's; reading the list needs `read:lists`, and `user_books.owned` does not follow the list. The app counts a book as owned when any edition of it is on the list, and turning Owned off removes every one.
- Lists: `lists` returns at most 100 rows per request and `list_books` 1,000, so both are keyset-paged. Unranked lists usually have no `position`, so list books are ordered by position, then id. Verified live: adding a book that is already on a list returns the existing row, and an empty description clears it. Whether `update_list` keeps fields left out is unverified, so `Client::update_list` resends the rest of the list as read (`default_view`, `featured_profile`, `url`, and the raw `privacy_setting_id` unless a privacy was chosen) with the edited name, description and ranking. `insert_list`/`update_list` return `errors` (a list); `insert_list_book` and `delete_list_book` have no error field, so a missing id is the failure.
- Goals: `metric` is `book`, `page` or `hour`. For hour goals `goal` is in hours but `progress` in seconds; `Goal::progress` is converted to hours. `update_goal` takes a whole `GoalInput` (fields left out are not kept), so `Client::update_goal` resends the goal as read, `conditions` and the raw `privacy_setting_id` included, and refuses one whose conditions hold a field it doesn't know or that has no description (`description` is `String!`). The mutations return `errors` (a list), not `error`. `delete_goal` answers a successful delete with `id: null`, so only `errors` means it failed. A goal created without `privacy_setting_id` gets the account default. Progress updates by itself as the user reads (`update_goal_progress` exists but the app doesn't call it).
- Search has a 2 s server timeout. The `_like`, `_ilike` and regex filters are disabled.
- docs.hardcover.app blocks automated fetches. Read the docs source at github.com/hardcoverapp/hardcover-docs instead.

## Screenshots

`docs/screenshots/` holds the README's screenshots, in light and dark. Take them from the demo build (`--features demo`, with `HARDCOVER_THEME=light` or `dark`), never from a real account:
- Use a 1200×800 floating window and capture just the window, at its scale factor.
- The demo downloads its covers, so it needs the network. Wait until every visible cover has loaded before capturing.
- Resizing the window after launch squeezes the library's list toward its minimum width. Drag the divider back to its default of 24rem before capturing.
- Strip metadata from the PNGs (`magick … -strip`).

## gpui-kit

Use the bundled `gpui-kit` and `gpui-kit-design-guides` skills, in `.claude/skills`, before any UI or architecture work. When the skills and the crate disagree, the crate source wins: `~/.cargo/registry/src/index.crates.io-*/gpui-component-0.10.0`. For example, `ActiveTheme` is imported from `gpui_kit::component::ActiveTheme`.

## CI and releases

- `.github/workflows/ci.yml` runs on pull requests, on pushes to `master` and weekly:
  - fmt
  - clippy with `-D warnings`, and the tests, as two parallel jobs
  - `cargo deny` (`deny.toml`)
  - desktop-entry and metainfo validation
  - actionlint
- Clippy and the tests run in an `ubuntu:22.04` container, the same one the release builds in. `ci/install-build-deps.sh` holds the system packages; it is the single list for both.
- `.github/actions/setup-rust` sets up every build job. Its caches are saved only from `master`; other branches restore them.
  - apt: the Ubuntu mirrors sometimes stall for minutes, so the package lists and `.deb`s are cached under a monthly key. On an exact hit, CI jobs install without `apt-get update`. The release always updates (`fresh-packages: true`), so its packages are checked against freshly signed lists.
  - zstd is installed after the apt cache is saved, so that cache stays gzip; it is restored before zstd exists. The cargo caches (`Swatinem/rust-cache`, one per job) then use zstd.
- The toolchain is pinned in `rust-toolchain.toml`. Bump it on purpose, and fix any new clippy lints in the same commit.
- Actions are pinned to commit SHAs; Dependabot updates them.
- **To release:**
  1. Bump `version` in `crates/app/Cargo.toml`.
  2. Add a `<release>` to `packaging/linux/io.github.alisayyah.HardcoverDesktop.metainfo.xml`.
  3. Push a `vX.Y.Z` tag.

  `just release <patch|minor|major|X.Y.Z>` (`scripts/release.sh ship`) does all of it. It asks once, then:
  - bumps `origin/master` in a temporary worktree (never the current checkout): the version, `Cargo.lock`, and a `<release>` dated today;
  - pushes `release-X.Y.Z` and opens the "Release X.Y.Z" PR, assigned to the user, listing the commits since the last tag;
  - waits for its CI run, then squash-merges it as "Release X.Y.Z (#N)" and deletes the branch;
  - waits for CI on the merge commit, then pushes the annotated tag `vX.Y.Z` ("Hardcover Desktop X.Y.Z");
  - follows `release.yml` until the GitHub release exists, and prints its URL.

  It refuses a version that isn't newer than the current one and the latest tag, and a release with no commits since the last tag. After a failure (red CI, say), fix it and run `just release` again with no level: it resumes an open `release-*` PR, a bump on master that has no tag yet, or a tag that hasn't been published. `--yes` skips the question; without a terminal it is required.

  The steps also run on their own:
  - `just bump <level>` edits this checkout and commits nothing.
  - `just release-pr <level>` only opens the PR.
  - `just release-tag` only tags `origin/master` (CI there must already have passed; it asks first) and follows the release build.

  It merges only a PR from this repository (never a fork's), into master, whose diff is exactly the bump (`Cargo.lock` and `crates/app/Cargo.toml` +1 −1, the metainfo +1) and whose head holds the expected version; anything else stops it before CI is even awaited. Keep that check in step if the bump ever changes more files.

  `release.yml` checks that the three versions agree, builds with `--profile dist` (thin LTO, stripped), packs a tarball (`packaging/linux/package.sh`) and creates the GitHub release. Running it by hand builds the tarball as an artifact without publishing anything.

## Self-update

The core side is `hardcover_core::update`; the app side is `crates/app/src/update.rs` plus the sidebar row in `shell.rs`.
- **When it installs:** the app checks `releases/latest` on GitHub. It installs a release only when it runs from an unpacked release tarball (`<prefix>/bin/hardcover-desktop` with the desktop entry under `<prefix>/share`) in a writable prefix. Other installs only announce new releases.
- **Trust chain:**
  - The minisign signature must verify against `packaging/minisign.pub`.
  - The signature's `file:` comment must be the tarball name for the advertised version and architecture, which stops replays of older signed releases.
  - The version must be newer than the running one.
  - The archive's top-level directory must match the tarball name.
  - Only regular files under `bin/hardcover-desktop` and `share/` are unpacked. They are staged in the prefix and renamed into place, the binary last.
- **Restarting:** `main` resolves the executable path before anything else and passes it to `set_restart_path`. After an update, `/proc/self/exe` points to the deleted old binary.
- **Signing:**
  - `release.yml` signs in the publish job, with the `MINISIGN_SECRET_KEY` repository secret exposed to that one step only, then verifies with `packaging/minisign.pub`.
  - Never replace the key: installed copies trust only this one.
  - Never re-publish different bytes under an existing version. If a release goes wrong, release a new patch version.
- **Release notes:** generated changes come first, followed by `<!-- install -->` and `packaging/linux/release-notes.md`. The app's update dialog shows only the part before the marker.
