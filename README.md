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

<img src="docs/screenshots/home.png" alt="Home in the light and dark themes, split diagonally: the books being read with their progress and pace, the reading goal and whether it is on track, the next book in a series, the newest books on Want to read, and the books coming soon">

## Features

- **Home:** the books you're reading, with your pace and when each should be done at it, your reading goal and whether you're on track, and what to read next, such as the next book in a series you've started.
- **Your library by shelf:** Currently Reading, Want to Read, Read, Paused and Did Not Finish, with counts in the sidebar, plus the books you ignored.
- **Series:** every series you've started, with the books you've read, the one you're on and the next one to read, grouped into in progress, waiting for the next book, not touched in a year and finished.
- **Coming soon:** the unreleased books in the series on your shelves, by release date, with one click to put them on Want to Read.
- **Search:** find any book, author or series on Hardcover, and add books to a shelf.
- **Discover:** the books the most readers finished on Hardcover this week, month, three months, year or of all time, each with its shelf.
- **Book details:** the authors and other contributors, series, page count, average rating, description, ratings breakdown, genres, moods and content warnings, and similar books.
- **Series and authors:** open a book's series to see its books in reading order and how many you've read, or an author to see their most-read books, each with its shelf.
- **Your reading:** set the status and rating, update your progress in pages (or time, for an audiobook), set the dates you started and finished, or remove a book.
- **Your copy:** choose the edition you read, keep private notes, choose who can see the book, and mark it owned or not (your Owned list on Hardcover).
- **Every read:** see each time you read a book, add a re-read or an earlier read, change its dates, or delete it.
- **Reviews:** read what other readers wrote about a book, with their ratings; spoilers stay hidden until you ask.
- **Your review:** write a review of a book in your library, in Markdown, mark it as containing spoilers and date it; change or delete it later. A review written on the website is shown here and edited there.
- **Reading goals:** see this year's goal and your pace in the sidebar, and set, change or delete a goal of books, pages or hours.
- **Lists:** open your lists from the sidebar to see their books in order, create, edit or delete a list, and put a book on your lists or take it off from its page.
- **Stats:** for all time or one year: books and pages finished, re-reads included, your pace, genres and moods, formats, book lengths, how you rate against other readers, your authors and series, how long books wait on your shelves, and when they were published, worked out from your library (only the genres and moods ask Hardcover).
- **Offline access:** your library and covers are cached on your computer, so they open instantly and stay readable without a connection.
- **Secure sign-in:** you sign in through Hardcover in your browser (OAuth with PKCE). The app never sees your password, and the session is stored in your system keyring.
- **Self-updating:** the app tells you about each new release and installs it in one click, after verifying its signature, whether you installed it with the installer or run the AppImage.
- **Settings:** choose a theme (Paper, warm; Graphite, neutral; or Hardcover, the website's own colours), each with a light and a dark side, light or dark or follow the system, and turn automatic update checks on or off.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/library-dark.png">
  <img src="docs/screenshots/library-light.png" alt="The Currently reading shelf, with a book's page beside it: its status, rating, progress, notes, edition, owned switch and privacy">
</picture>

- **Keyboard first:**

  | Shortcut | Action |
  | --- | --- |
  | <kbd>Ctrl</kbd>+<kbd>0</kbd> | Home |
  | <kbd>Ctrl</kbd>+<kbd>F</kbd> | Search |
  | <kbd>Ctrl</kbd>+<kbd>1</kbd>…<kbd>6</kbd> | Go to a shelf |
  | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>D</kbd> | Discover |
  | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>R</kbd> | Series |
  | <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>S</kbd> | Stats |
  | <kbd>Ctrl</kbd>+<kbd>R</kbd> | Refresh the library |
  | <kbd>Esc</kbd> or <kbd>Alt</kbd>+<kbd>←</kbd> | Back from a series, author or book page |
  | <kbd>Ctrl</kbd>+<kbd>,</kbd> | Settings |
  | <kbd>Ctrl</kbd>+<kbd>Q</kbd> | Quit |

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/series-dark.png">
  <img src="docs/screenshots/series-light.png" alt="Series: the series the reader has started, grouped by where they are, each with its books as a row of covers and the next step in it">
</picture>

## Install

Every release on [GitHub](https://github.com/AliSayyah/hardcover-desktop/releases/latest) comes in two forms for x86_64 Linux, the same app in each: the [installer](#installer), which puts it in `~/.local`, and an [AppImage](#appimage), a single file to run from anywhere. Either way the app updates itself, in one click, after verifying the new release's signature.

Both need x86_64 Linux with glibc 2.35 or newer (Ubuntu 22.04, Debian 12, Fedora 36 and later), Wayland or X11, Vulkan or OpenGL graphics drivers, and a Secret Service keyring, such as GNOME Keyring or KWallet, to store your sign-in.

### Installer

```sh
curl -fsSL https://github.com/AliSayyah/hardcover-desktop/releases/latest/download/install.sh | bash
```

That runs the installer published with the latest release. It downloads the release, checks its signature, and installs `~/.local/bin/hardcover-desktop` with its desktop entry and icon, so **Hardcover Desktop** appears in your app launcher. It needs no sudo. Installed this way, the app keeps itself up to date. To check the signature it uses your `minisign`, or downloads minisign's own release and checks its checksum.

The installer is signed too. To check it before running it, with [minisign](https://jedisct1.github.io/minisign/):

```sh
curl -fsSLO https://github.com/AliSayyah/hardcover-desktop/releases/latest/download/install.sh
curl -fsSLO https://github.com/AliSayyah/hardcover-desktop/releases/latest/download/install.sh.minisig
minisign -Vm install.sh -P RWQmH1HvOAVP+8piXBk3ZC+nqzBVFZFcynEg8NUsgQDCSYYAkrSkPRXW && bash install.sh
```

To install a tarball you already have, download the release's `install.sh`, put the tarball's `.minisig` next to it and run `bash install.sh hardcover-desktop-<version>-x86_64-linux.tar.gz`. Or unpack it yourself, to the same place:

```sh
mkdir -p ~/.local
tar -xzf hardcover-desktop-*-x86_64-linux.tar.gz -C ~/.local --strip-components=1
```

To uninstall, delete these:
- `~/.local/bin/hardcover-desktop`
- `~/.local/share/applications/io.github.alisayyah.HardcoverDesktop.desktop`
- `~/.local/share/icons/hicolor/scalable/apps/io.github.alisayyah.HardcoverDesktop.svg`
- `~/.local/share/metainfo/io.github.alisayyah.HardcoverDesktop.metainfo.xml`
- `~/.local/share/licenses/hardcover-desktop/`

### AppImage

Download the `.AppImage`, make it executable and run it:

```sh
chmod +x hardcover-desktop-*-x86_64-linux.AppImage
./hardcover-desktop-*-x86_64-linux.AppImage
```

Without FUSE, add `--appimage-extract-and-run`. To uninstall, delete the file.

The app updates the AppImage in place: the new release replaces the file and keeps its name, so shortcuts to it keep working (the version in the name stays the one you downloaded; **About** shows the one you run). It needs to be able to write to the AppImage's folder; otherwise it only tells you about new releases. The AppImage also carries the standard update information, so tools such as [AppImageUpdate](https://github.com/AppImageCommunity/AppImageUpdate), AppImageLauncher and Gear Lever can update it too.

### Checking a download

Every download, the installer included, has a `.minisig` signature next to it on the release (the AppImage's `.zsync` is for update tools, not a download). To check a file yourself with [minisign](https://jedisct1.github.io/minisign/), download its `.minisig` into the same directory and run:

```sh
minisign -Vm <file> -P RWQmH1HvOAVP+8piXBk3ZC+nqzBVFZFcynEg8NUsgQDCSYYAkrSkPRXW
```

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

With [just](https://github.com/casey/just) installed, `just check` runs all three (formatting is checked, not applied), and `just` lists the other tasks: the demo, the release tarball, `cargo deny`, packaging validation.

## Licence

[MIT](LICENSE). Hardcover is a trademark of its owners; this project only uses its public API.

The bundled fonts are under the SIL Open Font License: [Inter](https://rsms.me/inter/) ([licence](assets/fonts/inter/LICENSE.txt)), [Literata](https://github.com/googlefonts/literata) ([licence](assets/fonts/literata/LICENSE.txt)) and [Vazirmatn](https://github.com/rastikerdar/vazirmatn) ([licence](assets/fonts/vazirmatn/LICENSE.txt)).
