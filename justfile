# Tasks for Hardcover Desktop. Run `just` to list them.
# They wrap the cargo commands in CLAUDE.md, which still work on their own.

set shell := ["bash", "-euo", "pipefail", "-c"]
set positional-arguments

app := "hardcover-desktop"
core := "hardcover-core"

# Exports the variables in .env. Only the live-API recipes use it: `just run`
# must not pick up a HARDCOVER_TOKEN and skip OAuth without being asked to.
load_env := "set -a; if [[ -f .env ]]; then source ./.env; fi; set +a"

# List the recipes
default:
    @just --list --unsorted

# Run the app. Extra arguments go to cargo, e.g. `just run --release`
[group('run')]
run *args:
    cargo run -p {{ app }} "$@"

# Demo build: a sample library, no account. Theme: light or dark (default: Settings)
[group('run')]
demo theme="":
    if [[ -n "$1" ]]; then export HARDCOVER_THEME="$1"; fi; cargo run -p {{ app }} --features demo

# Demo build with injected faults (HARDCOVER_DEMO_FAULTS), e.g. `just demo-faults latency=800ms,500,error-rate=0.2`
[group('run')]
demo-faults faults theme="":
    if [[ -n "$2" ]]; then export HARDCOVER_THEME="$2"; fi; HARDCOVER_DEMO_FAULTS="$1" cargo run -p {{ app }} --features demo

# The shared components' gallery (demo build). Theme: light or dark (default: Settings)
[group('run')]
gallery theme="":
    if [[ -n "$1" ]]; then export HARDCOVER_THEME="$1"; fi; HARDCOVER_GALLERY=1 cargo run -p {{ app }} --features demo

# Debug build of the app
[group('build')]
build:
    cargo build -p {{ app }}

# Optimised, stripped build, as the release builds it (target/dist/hardcover-desktop)
[group('build')]
dist:
    cargo build --profile dist --locked -p {{ app }}

# Release tarball and its checksum, from the dist build, in target/package/
[group('build')]
package: dist
    packaging/linux/package.sh "$({{ just_executable() }} version)" target/dist/{{ app }} target/package

# The AppImage and its .zsync, from the release tarball, in target/package/. Bundles this system's libraries: release builds use Ubuntu 22.04
[group('build')]
appimage: package
    packaging/linux/package-appimage.sh "target/package/{{ app }}-$({{ just_executable() }} version)-x86_64-linux.tar.gz"

# Print the app's version, from crates/app/Cargo.toml
[group('build')]
version:
    @sed -n 's/^version = "\(.*\)"$/\1/p' crates/app/Cargo.toml | head -n1

# Format the code
[group('check')]
fmt:
    cargo fmt --all

# Check the formatting without changing anything
[group('check')]
fmt-check:
    cargo fmt --all --check

# Clippy over every target, warnings as errors, as CI runs it, then the demo build too
[group('check')]
clippy:
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo clippy -p {{ app }} --all-targets --features demo --locked -- -D warnings

# Apply clippy's suggestions, then format
[group('check')]
fix:
    cargo clippy --workspace --all-targets --fix --allow-dirty --allow-staged
    cargo fmt --all

# Unit tests (live-API tests stay ignored). Extra arguments go to cargo: `just test <name>`
[group('check')]
test *args:
    cargo test --workspace --locked "$@"

# cargo deny: advisories, bans, licenses and sources (or the checks given)
[group('check')]
deny *checks:
    @command -v cargo-deny >/dev/null || { echo "cargo-deny is not installed: cargo install --locked cargo-deny" >&2; exit 1; }
    cargo deny --locked check "$@"

# Validate the desktop entry and metainfo; check the Cargo and metainfo versions agree
[group('check')]
packaging:
    desktop-file-validate packaging/linux/*.desktop
    appstreamcli validate --no-net packaging/linux/*.metainfo.xml
    ci/check-version.sh

# ShellCheck the installer and the packaging, CI and release scripts
[group('check')]
shellcheck:
    shellcheck install.sh packaging/linux/*.sh ci/*.sh scripts/*.sh

# Test the release script's handling of a flaky network, offline (a fake gh)
[group('check')]
release-test:
    scripts/release-test.sh

# Test CI's apt-get wrapper, offline (a fake apt-get)
[group('check')]
apt-test:
    ci/apt-get-test.sh

# Lint the GitHub workflows
[group('check')]
actionlint:
    @command -v actionlint >/dev/null || { echo "actionlint is not installed: https://github.com/rhysd/actionlint" >&2; exit 1; }
    actionlint -color

# Formatting, clippy and tests: run this before pushing
[group('check')]
check: fmt-check clippy test

# Everything CI runs offline: check, plus cargo deny, packaging, shellcheck, the release script's and apt-get wrapper's tests and actionlint
[group('check')]
ci: check deny packaging shellcheck release-test apt-test actionlint

# Live read tests (needs HARDCOVER_TOKEN, from the environment or .env)
[group('live')]
live-test:
    {{ load_env }}; cargo test -p {{ core }} --test live_read -- --ignored

# Read-only smoke run against the API (needs HARDCOVER_TOKEN)
[group('live')]
smoke:
    {{ load_env }}; cargo run -p {{ core }} --example smoke

# Compare the live API with schema/ and the scopes, writing nothing; fails on any change
[group('live')]
api-check:
    {{ load_env }}; cargo run -p {{ core }} --example api_check -- --check

# Like api-check, then rewrite schema/ to match the live API
[group('live')]
schema-update:
    {{ load_env }}; cargo run -p {{ core }} --example api_check
    git diff --stat schema/

# Sign in from the command line and store the session in the keyring
[confirm("Sign in with the browser and store the session in the system keyring?")]
[group('live')]
oauth-login:
    cargo run -p {{ core }} --example oauth_login

# Keyring session's scopes, plus a write cycle on a book NOT in your library
[confirm("This writes to your real Hardcover account, then undoes it. Continue?")]
[group('live')]
oauth-check book_id:
    cargo run -p {{ core }} --example oauth_check -- "$1"

# Live write test, phase add-remove or full. Also needs HARDCOVER_LIVE_WRITES=yes and HARDCOVER_TOKEN
[confirm("This writes to your real Hardcover account, then undoes it. Continue?")]
[group('live')]
live-writes phase="add-remove" book_id="":
    {{ load_env }}; export HARDCOVER_LIVE_PHASE="$1"; if [[ -n "$2" ]]; then export HARDCOVER_TEST_BOOK_ID="$2"; fi; cargo run -p {{ core }} --example live_writes

# Make a release, from the PR to the published GitHub release: `just release patch` (or minor, major, X.Y.Z). Without a level, finishes one that stopped
[group('release')]
release *args:
    scripts/release.sh ship "$@"

# Bump the version in this checkout (major, minor, patch or X.Y.Z) and add a <release> to the metainfo. Commits nothing
[group('release')]
bump level:
    scripts/release.sh bump "$1"

# Only open the release PR: bump origin/master in a temporary worktree, push release-X.Y.Z, open the PR
[group('release')]
release-pr level:
    scripts/release.sh pr "$1"

# Only tag: tag the commit on origin/master that bumped to its version, once CI passed on it; push the tag, follow the release build
[group('release')]
release-tag *args:
    scripts/release.sh tag "$@"

# Remove build artefacts
clean:
    cargo clean
