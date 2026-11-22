#!/usr/bin/env bash
# Release tooling. The justfile's release recipes call it.
#
#   scripts/release.sh bump <major|minor|patch|X.Y.Z>    bump the working tree; commits nothing
#   scripts/release.sh pr <major|minor|patch|X.Y.Z>      open the release PR from origin/master
#   scripts/release.sh tag [--yes]                       tag the bump commit on origin/master, push the tag
#   scripts/release.sh ship [level] [--yes]              all of it, from the PR to the published release
#
# `ship` opens the release PR, waits for CI, squash-merges it, waits for CI on
# the merge commit, then tags that commit, pushes the tag, and waits for
# release.yml to publish. It asks once, before the first step. Run it again
# after a failure and it picks up where it stopped: an open release PR (or a
# pushed release branch without one) is reused, a bump already merged to
# master is tagged, and a tag not yet published is followed, all without a
# level.
#
# It rides out a flaky network: a call to GitHub that the network fails is
# tried again for about three minutes, a change that seemed to fail is first
# checked for having gone through, and a wait for CI polls the run and keeps
# going through 15 minutes without contact. When the network stays down, it
# says so and stops; running it again picks up from there.
#
# What is tagged is always the bump commit itself, never whatever master's tip
# is by then, and only once it is checked to change nothing but the version.
#
# Never touches the current checkout: the release commit is made in a
# temporary worktree off origin/master.
set -euo pipefail
# Without it, a failing command inside $(…) would not stop the script.
shopt -s inherit_errexit

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$root"

remote=origin
base=master
app_toml=crates/app/Cargo.toml
metainfo=packaging/linux/io.github.alisayyah.HardcoverDesktop.metainfo.xml

die() {
  echo "release: $*" >&2
  exit 1
}
step() { printf '\n==> %s\n' "$*" >&2; }
info() { printf '    %s\n' "$*" >&2; }

usage() {
  sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

# --- versions ---------------------------------------------------------------

# Read from stdin: the first `version = "…"` of a Cargo.toml, the newest
# <release> of the metainfo.
cargo_version() { sed -n 's/^version = "\(.*\)"$/\1/p' | head -n1; }
metainfo_version() { grep -o '<release version="[^"]*"' | head -n1 | cut -d'"' -f2; }
metainfo_date() { grep -o '<release version="[^"]*" date="[^"]*"' | head -n1 | cut -d'"' -f4; }

# True if $1 is a newer version than $2.
newer() { [[ $1 != "$2" && $(printf '%s\n' "$1" "$2" | sort -V | tail -n1) == "$1" ]]; }

# The version after $1 for a level (major, minor, patch) or an explicit X.Y.Z.
next_version() {
  local major minor patch
  IFS=. read -r major minor patch <<<"$1"
  case "$2" in
    major) echo "$((major + 1)).0.0" ;;
    minor) echo "$major.$((minor + 1)).0" ;;
    patch) echo "$major.$minor.$((patch + 1))" ;;
    *)
      local v=${2#v}
      [[ $v =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "expected major, minor, patch or X.Y.Z, got '$2'"
      echo "$v"
      ;;
  esac
}

# The bump, as filters from stdin to stdout. `apply_bump` makes it with them
# and `check_bump` accepts exactly what they make, so a commit passes only if
# it changes these lines and nothing else.

# crates/app/Cargo.toml: the package's version, the file's first `version =`.
bump_manifest() { sed "0,/^version = \".*\"$/s//version = \"$1\"/"; }
# Cargo.lock: the version of the hardcover-desktop package.
bump_lock() {
  awk -v new="$1" '
    app && /^version = / { print "version = \"" new "\""; app = 0; next }
    { app = ($0 == "name = \"hardcover-desktop\""); print }'
}
# The metainfo: a <release> of $1, dated $2, first under <releases>.
bump_metainfo() { sed "s|^\( *\)<releases>$|&\n\1  <release version=\"$1\" date=\"$2\"/>|"; }

# Sets the version in the checkout at $1 to $2 and adds today's <release>.
apply_bump() {
  local dir=$1 new=$2
  ! grep -q "<release version=\"$new\"" "$dir/$metainfo" || die "the metainfo already has a <release> for $new"
  bump_manifest "$new" <"$dir/$app_toml" >"$dir/$app_toml.bump"
  mv -- "$dir/$app_toml.bump" "$dir/$app_toml"
  bump_metainfo "$new" "$(date +%F)" <"$dir/$metainfo" >"$dir/$metainfo.bump"
  mv -- "$dir/$metainfo.bump" "$dir/$metainfo"
  # Cargo writes the lock file itself; `check_bump` holds it to `bump_lock`.
  (cd "$dir" && cargo update --offline --quiet -p hardcover-desktop)
  "$dir/ci/check-version.sh" "$new" >&2 || die "the bump left the versions disagreeing"
}

# True if $4 at commit $2 is $4 at commit $1 run through the filter $5…
bump_matches() {
  local parent=$1 commit=$2 file=$3
  shift 3
  cmp -s <(git show "$parent:$file" | "$@") <(git show "$commit:$file")
}

# Dies unless commit $2 is exactly the bump of commit $1 to version $3: the
# app's version in its Cargo.toml and in Cargo.lock, and a new first
# <release> in the metainfo, and nothing else. $4 names it in messages.
check_bump() {
  local parent=$1 commit=$2 version=$3 what=$4 files want old date file
  files=$(git diff --name-only "$parent" "$commit" | sort | paste -sd' ')
  want=$(printf '%s\n' Cargo.lock "$app_toml" "$metainfo" | sort | paste -sd' ')
  [[ $files == "$want" ]] || die "$what is not just the version bump: it changes $files"
  old=$(git show "$parent:$app_toml" | cargo_version)
  newer "$version" "$old" || die "$what does not bump $old to a newer version ($version)"
  date=$(git show "$commit:$metainfo" | metainfo_date)
  [[ $date =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || die "$what has no dated <release> first in the metainfo"
  for file in "$app_toml" Cargo.lock "$metainfo"; do
    case $file in
      "$app_toml") bump_matches "$parent" "$commit" "$file" bump_manifest "$version" ;;
      Cargo.lock) bump_matches "$parent" "$commit" "$file" bump_lock "$version" ;;
      *) bump_matches "$parent" "$commit" "$file" bump_metainfo "$version" "$date" ;;
    esac || die "$what changes more in $file than the bump to $version:
$(git diff "$parent" "$commit" -- "$file")"
  done
}

# --- git and GitHub ---------------------------------------------------------
#
# The network drops: an "unexpected EOF" from GitHub once ended the wait for
# a release build that then passed. So every read from GitHub goes through
# `retry`, every change through `write`, and a CI run is followed by polling
# it. When the network stays down the script stops in a state `ship` picks up
# again (`lost_contact`).

# Seconds to wait before each new try of a call the network failed.
retry_delays=(5 10 20 30 60 60)
# A CI run is polled every `poll` seconds. A wait gives up once its failed
# polls add up to `patience` seconds; a suspended laptop doesn't count.
poll=15
patience=900

# True if the error $1 is the network's, not an answer: gh's transport errors
# (Go's `Get "https://…": …`), git's (`unable to access '…'`), a server error
# or a rate limit.
transient() {
  grep -qiE '^(Get|Post|Put|Patch|Delete|Head) "https?://[^"]*": |unable to access|error connecting to|HTTP (5[0-9]{2}|429)|rate limit|Something went wrong|RPC failed|early EOF|remote end hung up|unexpected EOF|connection (reset|refused|closed|timed out)|could not resolve host|TLS handshake|deadline exceeded|timed out|timeout' <<<"$1"
}

lost_contact() {
  echo "release: can't reach GitHub. Once the network is back, run \`just release\` (no level needed) again: it picks up where this stopped." >&2
}

# Says why a try failed (the first line of $1) and waits $2 seconds.
again() {
  info "${1%%$'\n'*}"
  info "trying again in $2 s"
  sleep "$2"
}

# Runs the read $@ and prints its output, trying again while the network
# fails it. An answer is not tried again:
#   -a PATTERN  an error matching it is an answer ("release not found"):
#               returns 1, quietly
#   -s STATUS   so is that exit status (`git ls-remote --exit-code`'s 2)
# Any other error is shown and returns 2; a network that stays down, 75.
retry() {
  local answer='' answer_status='' errfile out err status try
  while [[ ${1-} == -[as] ]]; do
    case $1 in
      -a) answer=$2 ;;
      -s) answer_status=$2 ;;
    esac
    shift 2
  done
  errfile=$(mktemp)
  for ((try = 0; ; try++)); do
    status=0
    out=$("$@" 2>"$errfile") || status=$?
    err=$(<"$errfile")
    if ((status == 0)); then
      rm -f "$errfile"
      [[ -z $out ]] || printf '%s\n' "$out"
      return 0
    fi
    if [[ $status == "$answer_status" ]] || { [[ -n $answer ]] && grep -qiE "$answer" <<<"$err"; }; then
      rm -f "$errfile"
      return 1
    fi
    if ! transient "$err" || ((try == ${#retry_delays[@]})); then
      rm -f "$errfile"
      printf '%s\n' "$err" >&2
      if transient "$err"; then
        lost_contact
        return 75
      fi
      return 2
    fi
    again "$err" "${retry_delays[try]}"
  done
}

# `write CHECK… -- COMMAND…` runs the change COMMAND (its output to stderr).
# When it fails, CHECK says whether it happened anyway and only the answer was
# lost; if not, a network failure is tried again as in `retry`, and anything
# else returns 1, shown. CHECK is one call, and a network failure there counts
# as not yet: each change here is safe to send twice.
write() {
  local check=() err status try
  while [[ $1 != -- ]]; do
    check+=("$1")
    shift
  done
  shift
  for ((try = 0; ; try++)); do
    status=0
    { err=$("$@" 2>&1 >&3 3>&-); } 3>&2 || status=$?
    ((status != 0)) || return 0
    if "${check[@]}" >/dev/null 2>&1; then
      info "it went through, though the answer was lost"
      return 0
    fi
    if ! transient "$err" || ((try == ${#retry_delays[@]})); then
      printf '%s\n' "$err" >&2
      ! transient "$err" || lost_contact
      return 1
    fi
    again "$err" "${retry_delays[try]}"
  done
}

# Fetches $base and the tags, and notes which release tags $remote has: those
# count, not a local tag that a stopped run made and never pushed.
fetch() {
  retry git fetch --quiet --tags "$remote" "$base"
  remote_tags=$(retry git ls-remote --tags --refs "$remote" 'refs/tags/v[0-9]*' | sed 's|.*refs/tags/||')
}
base_sha() { git rev-parse "$remote/$base"; }
base_version() { git show "$remote/$base:$app_toml" | cargo_version; }
base_listed() { git show "$remote/$base:$metainfo" | metainfo_version; }
# The newest release tag on $remote, as of the last `fetch`.
latest_tag() { sed '/^$/d' <<<"$remote_tags" | sort -V | tail -n1; }
tag_on_remote() { grep -qxF -- "$1" <<<"$remote_tags"; }

# Whether tag $1 has a GitHub release. Stops when GitHub can't say.
released() {
  local status=0
  retry -a 'release not found' gh release view "$1" --json tagName >/dev/null || status=$?
  case $status in
    0) return 0 ;;
    1) return 1 ;;
    *) exit 1 ;; # retry said why
  esac
}

# The versions of the release branches (`release-X.Y.Z`) in `git ls-remote`'s
# output, on stdin.
release_branches() { sed -n 's|.*\trefs/heads/release-\([0-9]\{1,\}\.[0-9]\{1,\}\.[0-9]\{1,\}\)$|\1|p'; }

# The number of the open PR from this repository's branch $1, if any. A fork
# can name its branch anything.
pr_for_branch() {
  gh pr list --base "$base" --head "$1" --state open --json number,isCrossRepository \
    --jq '[.[] | select(.isCrossRepository | not)][0].number // empty'
}

# The checks `write` asks after a failure, one try each.
pr_opened() { [[ -n $(pr_for_branch "$1") ]]; }
pr_merged() { [[ $(gh pr view "$1" --json state --jq .state) == MERGED ]]; }
branch_at() { [[ $(git ls-remote --heads "$remote" "refs/heads/$1" | cut -f1) == "$2" ]]; }
# Tag $1 on $remote, peeled, is commit $2.
tag_at() {
  [[ $(git ls-remote --tags "$remote" "refs/tags/$1" "refs/tags/$1^{}" | awk '{ sha = $1 } END { print sha }') == "$2" ]]
}

need_gh() {
  command -v gh >/dev/null || die "needs the GitHub CLI (gh)"
  # Not `gh auth status`: offline, it says the token is invalid.
  gh auth token >/dev/null 2>&1 || die "gh is not signed in: gh auth login"
}

# The changes since the latest tag, one "- subject" line each.
changes() {
  local latest
  latest=$(latest_tag)
  git log --format='- %s' "${latest:+$latest..}$remote/$base"
}

# Asks $1 [y/N]; --yes answers for it.
confirm() {
  [[ ${yes:-} == 1 ]] && return
  [[ -t 0 ]] || die "not a terminal: pass --yes to go ahead without asking"
  local answer
  read -rp "$1 [y/N] " answer
  [[ $answer == [yY]* ]] || die "cancelled"
}

# The newest run of workflow $1 for commit $2 and event $3, waiting up to five
# minutes for it to appear (and longer while GitHub can't be reached).
find_run() {
  local id polls=0 offline=0 status
  while :; do
    status=0
    id=$(poll_once gh run list --workflow "$1" --commit "$2" --event "$3" --limit 1 \
      --json databaseId --jq '.[0].databaseId // empty') || status=$?
    if ((status == 0)); then
      if [[ -n $id ]]; then
        echo "$id"
        return
      fi
      ((++polls < 60)) || die "no $1 run started for ${2:0:7} ($3) after five minutes"
    else
      ((status == 1)) || die "couldn't look for the $1 run of ${2:0:7} (above)"
      ((offline += 5))
      if ((offline >= patience)); then
        lost_contact
        exit 1
      fi
    fi
    sleep 5
  done
}

# One poll of a wait: runs $@ and prints its output. Returns 1 when the
# network failed it, and 2, shown, on any other error: no reason to wait on.
poll_once() {
  local errfile status=0
  errfile=$(mktemp)
  "$@" 2>"$errfile" || status=$?
  if ((status != 0)); then
    if transient "$(<"$errfile")"; then
      status=1
    else
      cat "$errfile" >&2
      status=2
    fi
  fi
  rm -f "$errfile"
  return "$status"
}

# A run's state for `wait_for_run`: "status|conclusion|what it is doing".
run_state='"\(.status)|\(.conclusion // "")|\([.jobs[] | select(.status == "in_progress")
  | .name + (first(.steps[]? | select(.status == "in_progress") | ": " + .name) // "")] | join("; "))"'

# Waits for workflow $1 on commit $2 (event $3) to finish; dies unless it
# passed. It polls the run rather than use `gh run watch`, which ends at the
# first dropped connection, so a run that failed and a network that did are
# told apart.
wait_for_run() {
  local workflow=$1 sha=$2 event=$3 id url state status conclusion now shown='' offline=0 polled
  id=$(find_run "$workflow" "$sha" "$event")
  url=$(retry gh run view "$id" --json url --jq .url)
  info "$url"
  while :; do
    polled=0
    state=$(poll_once gh run view "$id" --json status,conclusion,jobs --jq "$run_state") || polled=$?
    if ((polled == 0)); then
      ((offline == 0)) || info "reached GitHub again"
      offline=0
      IFS='|' read -r status conclusion now <<<"$state"
      if [[ $status == completed ]]; then
        [[ $conclusion != success ]] || return 0
        die "$workflow ended in $conclusion on ${sha:0:7}: $url
  Push a fix (or re-run it: gh run rerun --failed $id), then run \`just release\` again."
      fi
      now=${now:-$status}
      [[ $now == "$shown" ]] || info "$now"
      shown=$now
    else
      ((polled == 1)) || die "couldn't follow $workflow: $url (above)"
      ((offline > 0)) || info "can't reach GitHub; still waiting (the run goes on regardless)"
      ((offline += poll))
      if ((offline >= patience)); then
        info "as far as is known, $workflow is still running: $url"
        lost_contact
        exit 1
      fi
    fi
    sleep "$poll"
  done
}

# --- steps ------------------------------------------------------------------

cmd_bump() {
  [[ $# -eq 1 ]] || usage
  local current new
  current=$(cargo_version <"$app_toml")
  new=$(next_version "$current" "$1")
  newer "$new" "$current" || die "$new is not newer than $current"
  apply_bump "$root" "$new"
  git --no-pager diff --stat -- "$app_toml" Cargo.lock "$metainfo"
  echo "Bumped $current -> $new. Commit nothing else with it; or use \`just release\` to do the rest."
}

# Opens (or finds) the PR that bumps origin/master to $1; prints its number.
# A branch an earlier run pushed but couldn't open the PR for is reused, if it
# is still exactly the bump on master's tip.
open_pr() {
  local new=$1 branch="release-$1" number tmp head tip latest body
  number=$(retry pr_for_branch "$branch")
  if [[ -n $number ]]; then
    info "reusing the open PR #$number"
    echo "$number"
    return
  fi

  tip=$(base_sha)
  head=$(retry git ls-remote --heads "$remote" "refs/heads/$branch" | cut -f1)
  if [[ -n $head ]]; then
    retry git fetch --quiet "$remote" "refs/heads/$branch"
    [[ $(git rev-parse FETCH_HEAD) == "$head" && $(git rev-parse "$head^") == "$tip" ]] ||
      die "$branch exists on $remote with no open PR, and is not the bump on $base's tip: delete it (git push $remote --delete $branch) and run again"
    check_bump "$tip" "$head" "$new" "$branch"
    info "$branch is already pushed; opening its PR"
  else
    tmp=$(mktemp -d)
    # shellcheck disable=SC2064 # expand now: tmp is local
    trap "git -C '$root' worktree remove --force '$tmp/tree' 2>/dev/null || true; rm -rf '$tmp'; git -C '$root' worktree prune" EXIT
    git worktree add --quiet --detach "$tmp/tree" "$tip"
    apply_bump "$tmp/tree" "$new"
    git -C "$tmp/tree" commit --quiet -am "Release $new"
    head=$(git -C "$tmp/tree" rev-parse HEAD)
    write branch_at "$branch" "$head" -- git -C "$tmp/tree" push --quiet "$remote" "HEAD:refs/heads/$branch"
  fi

  latest=$(latest_tag)
  body=$(
    echo "Bumps the app to $new and adds the metainfo release entry. Once CI passes, \`just release\` merges it and tags \`v$new\` to publish."
    echo
    echo "Since ${latest:-the start}:"
    echo
    changes
  )
  write pr_opened "$branch" -- \
    gh pr create --base "$base" --head "$branch" --title "Release $new" --assignee @me --body "$body"
  number=$(retry pr_for_branch "$branch")
  [[ -n $number ]] || die "opened the PR for $branch, but GitHub doesn't list it; run \`just release\` again"
  echo "$number"
}

# Dies unless PR $1 is exactly the bump to $2: from this repository, into
# master, one commit (its head, $3) on top of master's tip, changing only the
# version lines. A PR on an older master would ship changes it doesn't list.
check_pr() {
  local number=$1 version=$2 head=$3 shape tip
  shape=$(retry gh pr view "$number" --json isCrossRepository,baseRefName \
    --jq '"\(.isCrossRepository) \(.baseRefName)"')
  [[ $shape == "false $base" ]] ||
    die "PR #$number is not from this repository into $base ($shape), so it is not merged automatically"
  fetch
  retry git fetch --quiet "$remote" "refs/pull/$number/head"
  [[ $(git rev-parse FETCH_HEAD) == "$head" ]] || die "PR #$number moved while being checked; run again"
  tip=$(base_sha)
  git merge-base --is-ancestor "$tip" "$head" ||
    die "$base has moved since PR #$number was opened, so the release would ship changes it doesn't list.
  Close it (gh pr close $number --delete-branch) and run \`just release <level>\` again."
  [[ $(git rev-parse "$head^") == "$tip" ]] ||
    die "PR #$number is more than one commit on $base, so it is not merged automatically"
  check_bump "$tip" "$head" "$version" "PR #$number"
}

# Squash-merges PR $1, the bump to $2, once it checks out and its CI has
# passed; prints the merge commit. Dies unless that commit is exactly what the
# PR's CI tested: had master moved while it merged, it would hold more.
merge_pr() {
  local number=$1 version=$2 head merged i
  head=$(retry gh pr view "$number" --json headRefOid --jq .headRefOid)
  check_pr "$number" "$version" "$head"
  step "Waiting for CI on PR #$number"
  wait_for_run ci.yml "$head" pull_request
  check_pr "$number" "$version" "$head"
  step "Merging PR #$number"
  # --match-head-commit makes a second try safe.
  write pr_merged "$number" -- gh pr merge "$number" --squash --delete-branch --match-head-commit "$head" \
    --subject "Release $version (#$number)" --body ""
  # GitHub can take a moment to show the merge commit.
  for ((i = 0; ; i++)); do
    merged=$(retry gh pr view "$number" --json mergeCommit --jq '.mergeCommit.oid // empty')
    [[ -z $merged ]] || break
    ((i < 12)) || die "PR #$number shows no merge commit; finish with \`just release\`"
    sleep 5
  done
  fetch
  git merge-base --is-ancestor "$merged" "$remote/$base" || die "the merge of PR #$number (${merged:0:7}) is not on $remote/$base"
  [[ $(git rev-parse "$merged^{tree}") == "$(git rev-parse "$head^{tree}")" ]] ||
    die "the merge of PR #$number (${merged:0:7}) is not what its CI tested: $base moved while it merged.
  Nothing was tagged. Check ${merged:0:7}, then tag it with \`just release-tag\`."
  echo "$merged"
}

# The commit on $base that bumped the app to $1: the newest on its
# first-parent history that added the <release> for $1. Dies unless it is
# exactly a bump. Commits after it are not part of the release.
release_commit() {
  local version=$1 sha
  sha=$(git log --first-parent --format=%H -n1 -S"<release version=\"$version\"" "$remote/$base" -- "$metainfo")
  [[ -n $sha ]] || die "no commit on $remote/$base adds the <release> for $version"
  check_bump "$sha^" "$sha" "$version" "${sha:0:7} ($(git log -1 --format=%s "$sha"))"
  echo "$sha"
}

# Dies unless $remote/$base's version and its newest metainfo <release> agree;
# prints the version.
base_release() {
  local version listed
  version=$(base_version)
  listed=$(base_listed)
  [[ $version == "$listed" ]] ||
    die "on $remote/$base, Cargo.toml says $version but the newest metainfo <release> is $listed"
  echo "$version"
}

# Tags commit $1, the bump to $2 on $base, and pushes the tag. Unless asked
# to wait ($3 = wait), CI on that commit must already have passed. A local
# tag left by a run that stopped while pushing it is pushed as it is, if it
# is on that commit.
tag_release() {
  local sha=$1 version=$2 tag="v$2" latest ci later
  [[ $(git show "$sha:$app_toml" | cargo_version) == "$version" &&
    $(git show "$sha:$metainfo" | metainfo_version) == "$version" ]] ||
    die "${sha:0:7} is not version $version"
  git merge-base --is-ancestor "$sha" "$remote/$base" || die "${sha:0:7} is not on $remote/$base"
  ! tag_on_remote "$tag" || die "$tag already exists on $remote: start a new release with \`just release patch\` (or minor, major)"
  if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    [[ $(git rev-parse "$tag^{commit}") == "$sha" ]] ||
      die "a local tag $tag, not on $remote, points at $(git rev-parse --short "$tag^{commit}"), not ${sha:0:7}: delete it (git tag -d $tag) and run again"
  fi
  latest=$(latest_tag)
  [[ -z $latest ]] || newer "$version" "${latest#v}" || die "$version is not newer than the latest release, $latest"

  if [[ ${3:-} == wait ]]; then
    step "Waiting for CI on ${sha:0:7}"
    wait_for_run ci.yml "$sha" push
  else
    ci=$(retry gh run list --workflow ci.yml --commit "$sha" --event push --limit 1 \
      --json status,conclusion --jq '.[0] | select(.) | "\(.status) \(.conclusion)"')
    [[ $ci == "completed success" ]] || die "CI on ${sha:0:7} has not passed: ${ci:-no run yet}"
  fi

  step "Tagging $tag"
  info "commit:   $(git log -1 --format='%h %s' "$sha")"
  info "previous: ${latest:-none}"
  later=$(git rev-list --count "$sha..$remote/$base")
  [[ $later == 0 ]] || info "not in it: the $later later commits on $base"
  confirm "Tag and push $tag? This publishes a release that installed copies update to."
  if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
    info "pushing the local tag $tag, from a run that stopped"
  else
    git tag -a "$tag" -m "Hardcover Desktop $version" "$sha"
  fi
  write tag_at "$tag" "$sha" -- git push --quiet "$remote" "refs/tags/$tag" || die "couldn't push $tag"
  info "pushed $tag"
}

# Waits for release.yml to publish tag $1, then prints the release.
watch_release() {
  local tag=$1 sha
  sha=$(git rev-parse "$tag^{commit}")
  step "Waiting for the release build of $tag"
  wait_for_run release.yml "$sha" push
  released "$tag" || die "release.yml passed but there is no GitHub release for $tag"
  step "Released $tag"
  local url
  ! url=$(gh release view "$tag" --json url --jq .url 2>/dev/null) || info "$url"
}

cmd_pr() {
  [[ $# -eq 1 ]] || usage
  need_gh
  fetch
  local current new
  current=$(base_version)
  new=$(next_version "$current" "$1")
  newer "$new" "$current" || die "$new is not newer than $current"
  step "Opening the release PR for $new"
  open_pr "$new" >/dev/null
}

cmd_tag() {
  need_gh
  fetch
  local version sha
  version=$(base_release)
  sha=$(release_commit "$version")
  tag_release "$sha" "$version"
  watch_release "v$version"
}

cmd_ship() {
  local level=${1:-}
  need_gh
  fetch
  local current latest
  current=$(base_version)
  latest=$(latest_tag)

  # Picking up an earlier run.
  if [[ -n $latest ]] && newer "$current" "${latest#v}"; then
    # The bump is on master but not tagged.
    [[ -z $level ]] || die "$base already has $current, not yet released: run \`just release\` without a level to finish it"
    step "Finishing $current: the bump is already on $base"
    local sha
    current=$(base_release)
    sha=$(release_commit "$current")
    tag_release "$sha" "$current" wait
    watch_release "v$current"
    return
  fi
  if tag_on_remote "v$current" && ! released "v$current"; then
    # Tagged, but the release build hasn't published it.
    [[ -z $level ]] || die "v$current is tagged but not published yet: run \`just release\` without a level to follow it"
    watch_release "v$current"
    return
  fi
  # A release PR, or a release branch whose PR a run couldn't open.
  # Only `release-X.Y.Z` is a release branch.
  local open version='' what pushed drop
  open=$(retry gh pr list --state open --base "$base" --json number,headRefName,isCrossRepository \
    --jq '[.[] | select((.isCrossRepository | not) and (.headRefName | test("^release-[0-9]+\\.[0-9]+\\.[0-9]+$")))][0]
      | select(.) | "\(.number) \(.headRefName)"')
  if [[ -n $open ]]; then
    version=${open#* release-}
    what="PR #${open%% *} is open"
    drop="close it (gh pr close ${open%% *} --delete-branch)"
  else
    pushed=$(retry git ls-remote --heads "$remote" 'refs/heads/release-*' | release_branches | sort -V | tail -n1)
    if [[ -n $pushed ]] && newer "$pushed" "$current"; then
      version=$pushed
      what="release-$pushed is pushed, with no PR yet"
      drop="delete it (git push $remote --delete release-$pushed)"
    fi
  fi
  if [[ -n $version ]]; then
    [[ -z $level || $version == "$(next_version "$current" "$level")" ]] ||
      die "$version is already under way ($what): finish it with \`just release\`, or $drop"
    step "Finishing $version: $what"
    confirm "Merge its PR once CI passes, then tag and publish v$version?"
    yes=1
    local number merged
    number=$(open_pr "$version")
    merged=$(merge_pr "$number" "$version")
    tag_release "$merged" "$version" wait
    watch_release "v$version"
    return
  fi

  # A new release.
  [[ -n $level ]] || die "nothing in progress: say which release, e.g. \`just release patch\` (or minor, major, X.Y.Z)"
  local new log
  new=$(next_version "$current" "$level")
  newer "$new" "$current" || die "$new is not newer than $current"
  [[ -z $latest ]] || newer "$new" "${latest#v}" || die "$new is not newer than the latest release, $latest"
  log=$(changes)
  [[ -n $log ]] || die "nothing to release: $base has no commits since ${latest:-the start}"

  step "Release $current -> $new"
  info "Changes since ${latest:-the start}:"
  printf '%s\n' "$log" | sed 's/^/      /' >&2
  info ""
  info "This opens a release PR, squash-merges it once CI passes, waits for CI on $base,"
  info "then pushes the tag v$new. release.yml publishes it, and installed copies update to it."
  confirm "Go ahead?"
  yes=1 # one answer covers the whole run

  local number merged
  step "Opening the release PR"
  number=$(open_pr "$new")
  merged=$(merge_pr "$number" "$new")
  tag_release "$merged" "$new" wait
  watch_release "v$new"
}

# --- main -------------------------------------------------------------------

# Sourced (to try a function), it stops here.
[[ ${BASH_SOURCE[0]} == "$0" ]] || return 0

[[ $# -ge 1 ]] || usage
command=$1
shift
args=()
yes=0
for arg in "$@"; do
  case "$arg" in
    -y | --yes) yes=1 ;;
    -h | --help) usage ;;
    *) args+=("$arg") ;;
  esac
done
set -- "${args[@]+"${args[@]}"}"

case "$command" in
  bump) cmd_bump "$@" ;;
  pr) cmd_pr "$@" ;;
  tag) cmd_tag "$@" ;;
  ship) cmd_ship "$@" ;;
  *) usage ;;
esac
