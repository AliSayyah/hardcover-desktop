<p align="center">
  <img src="assets/icon.svg" width="96" height="96" alt="">
</p>

<h1 align="center">Hardcover Desktop</h1>

<p align="center">
  An unofficial Linux desktop client for <a href="https://hardcover.app">Hardcover</a>, the social reading tracker.
</p>

<p align="center">
  <a href="https://github.com/AliSayyah/hardcover-desktop/actions/workflows/ci.yml"><img src="https://github.com/AliSayyah/hardcover-desktop/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/AliSayyah/hardcover-desktop/releases/latest"><img src="https://img.shields.io/github/v/release/AliSayyah/hardcover-desktop" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/AliSayyah/hardcover-desktop" alt="MIT licence"></a>
</p>

Hardcover Desktop is a native app built with Rust and [GPUI](https://gpui-kit.com). It keeps your library close at hand. You can browse your shelves, find and add books, rate them, and log your progress without opening a browser.

> [!NOTE]
> This is a community project. It is not made by, affiliated with, or endorsed by Hardcover.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/library-dark.png">
  <img src="docs/screenshots/library-light.png" alt="Hardcover Desktop showing the Currently reading shelf, with a book's details and reading progress beside it">
</picture>

## Features

- **Your library by shelf:** Currently Reading, Want to Read, Read, Paused and Did Not Finish, with counts in the sidebar, plus the books you ignored.
- **Search:** find any book on Hardcover and add it to a shelf.
- **Book details:** the authors and other contributors, series, page count, average rating, description, ratings breakdown, genres, moods and content warnings, and similar books.
- **Series and authors:** open a book's series to see its books in reading order and how many you've read, or an author to see their most-read books, each with its shelf.
- **Your reading:** set the status and rating, update your progress in pages (or time, for an audiobook), set the dates you started and finished, or remove a book.
- **Your copy:** choose the edition you read, keep private notes, choose who can see the book, and add the edition to your Owned list.
- **Every read:** see each time you read a book, add a re-read or an earlier read, change its dates, or delete it.
- **Reading goals:** see this year's goal and your pace in the sidebar, and set, change or delete a goal of books, pages or hours.
- **Offline access:** your library and covers are cached on your computer, so they open instantly and stay readable without a connection.
- **Secure sign-in:** you sign in through Hardcover in your browser (OAuth with PKCE). The app never sees your password, and the session is stored in your system keyring.
- **Self-updating:** the app offers each new release and installs it in one click, after verifying its signature.
- **Settings:** choose a light or dark theme or follow the system, and turn automatic update checks on or off.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/search-dark.png">
  <img src="docs/screenshots/search-light.png" alt="Searching for an author, with one of the results open beside the list">
</picture>

- **Keyboard first:**

  | Shortcut | Action |
  | --- | --- |
  | <kbd>Ctrl</kbd>+<kbd>F</kbd> | Search |
  | <kbd>Ctrl</kbd>+<kbd>1</kbd>…<kbd>6</kbd> | Go to a shelf |
  | <kbd>Ctrl</kbd>+<kbd>R</kbd> | Refresh the library |
  | <kbd>Esc</kbd> or <kbd>Alt</kbd>+<kbd>←</kbd> | Back from a series, author or book page |
| <kbd>Ctrl</kbd>+<kbd>,</kbd> | Settings |
  | <kbd>Ctrl</kbd>+<kbd>Q</kbd> | Quit |

## Install

Download `hardcover-desktop-<version>-x86_64-linux.tar.gz` from the [latest release](https://github.com/AliSayyah/hardcover-desktop/releases/latest) and unpack it into `~/.local`:

```sh
tar -xzf hardcover-desktop-*-x86_64-linux.tar.gz -C ~/.local --strip-components=1
```

This installs `~/.local/bin/hardcover-desktop` with its desktop entry and icon, so **Hardcover Desktop** appears in your app launcher. Installed this way, the app keeps itself up to date.

To check the download first, verify its signature with [minisign](https://jedisct1.github.io/minisign/):

```sh
minisign -Vm hardcover-desktop-*-x86_64-linux.tar.gz -P RWQmH1HvOAVP+8piXBk3ZC+nqzBVFZFcynEg8NUsgQDCSYYAkrSkPRXW
```

**Requirements:**
- x86_64 Linux with glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and later)
- Wayland or X11
- Vulkan or OpenGL graphics drivers
- a Secret Service keyring, such as GNOME Keyring or KWallet

To uninstall, delete these:
- `~/.local/bin/hardcover-desktop`
- `~/.local/share/applications/io.github.alisayyah.HardcoverDesktop.desktop`
- `~/.local/share/icons/hicolor/scalable/apps/io.github.alisayyah.HardcoverDesktop.svg`
- `~/.local/share/metainfo/io.github.alisayyah.HardcoverDesktop.metainfo.xml`
- `~/.local/share/licenses/hardcover-desktop/`

## Your data

The app talks to Hardcover's API, and to GitHub to check for updates. It stores the following on your computer:

| What | Where |
| --- | --- |
| Sign-in session | System keyring, service `hardcover-desktop` |
| Library cache | `~/.local/share/hardcover-desktop/` |
| Cover images | `~/.cache/hardcover-desktop/covers/` |
| Settings | `~/.config/hardcover-desktop/settings.json` |

Signing out (account menu → **Sign out…**, or in **Settings**) revokes the session and deletes the library cache; your settings stay. To turn off automatic update checks, switch them off in **Settings**, or set `HARDCOVER_NO_UPDATE_CHECK=1`. **Check for updates** in the account menu still works.

## Build from source

You need Rust (the toolchain is pinned in `rust-toolchain.toml` and installs automatically with [rustup](https://rustup.rs)), plus a few system libraries.

On Debian or Ubuntu, install them with:

```sh
sudo apt install build-essential pkg-config libdbus-1-dev libfontconfig-dev libxcb1-dev libxkbcommon-x11-dev
```

On other distributions, install the equivalents: D-Bus, Fontconfig, xcb and xkbcommon-x11 development packages.

Then:

```sh
cargo run -p hardcover-desktop                # run the app
cargo test --workspace                        # run the tests
cargo build --profile dist -p hardcover-desktop  # optimized, stripped build
```

To try the app without an account, build it with the `demo` feature:

```sh
cargo run -p hardcover-desktop --features demo
```

It shows a sample library of popular books for a reader called `demo_reader`. The books come with their real covers, downloaded from Hardcover; that is the only thing the demo fetches. It never reads your keyring, cache or settings, and edits are refused. The screenshots above come from it.

The workspace has two crates:
- `crates/hardcover-core`: the Hardcover GraphQL client, OAuth sign-in, the library and cover caches, and self-update. It has no GUI code.
- `crates/app`: the GPUI interface.

[CLAUDE.md](CLAUDE.md) has more on the architecture, API quirks and release process.

## Contributing

Issues and pull requests are welcome. CI runs rustfmt, clippy (`-D warnings`), the tests and `cargo deny`, so it saves a round trip to run them locally first:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Licence

[MIT](LICENSE). Hardcover is a trademark of its owners; this project only uses its public API.
