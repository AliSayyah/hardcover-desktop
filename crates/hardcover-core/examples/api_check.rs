//! Checks the live Hardcover API for changes that matter to this app.
//!
//! 1. Introspects the GraphQL schema and renders it in the format of
//!    `schema/hardcover.graphql`, then lists the types and fields that were
//!    added, removed or changed since the committed copy.
//! 2. Validates every document the client sends against the live schema.
//! 3. Fetches the public OAuth scope map (`/capabilities.json`) and checks that
//!    every scope the app requests still exists and that together they grant
//!    every top-level field (operation), gated column and gated argument the
//!    documents use, then lists scope changes since
//!    `schema/hardcover-capabilities.json`. Some scopes gate only arguments
//!    (`write:reviews` grants the `review*` fields of `update_user_book`, no
//!    operation). Arguments written out in a document are checked exactly;
//!    the fields of an input-object *variable* are only what the client can
//!    send, so an ungranted one is printed as a note, not a failure. The same
//!    check runs offline against `schema/` in the unit tests.
//!
//! Then it writes both files, so `git diff schema/` shows the full change.
//!
//! Exit codes:
//! - 0: nothing is broken, and with `--check` `schema/` matches the live API
//!   (without it, any change has been written to `schema/`).
//! - 1: broken: a document no longer validates against the live schema, or
//!   the app's scopes no longer cover it, so the released app is broken
//!   against the live API. This wins over drift.
//! - 2: it could not run: a bad argument, no token, a network error or an
//!   answer it can't read.
//! - 3 (`--check` only): drift: nothing is broken, but the live schema or
//!   scope map differs from `schema/`; run it without `--check` (`just
//!   schema-update`) to update it.
//!
//! `--summary <path>` appends a short Markdown report to the file (meant for
//! `$GITHUB_STEP_SUMMARY`): the outcome, the broken documents and ungranted
//! uses, and the schema and scope changes. A run that could not finish says
//! so there too. Failing to write it is only a warning.
//!
//! Uses `HARDCOVER_TOKEN` if set, otherwise the app's keyring session (sign
//! in with the app or the `oauth_login` example first). Two API requests.
//!
//! ```sh
//! cargo run -p hardcover-core --example api_check             # check and update schema/
//! cargo run -p hardcover-core --example api_check -- --check  # check only: exit 1 broken, 3 drift
//! cargo run -p hardcover-core --example api_check -- --check --summary report.md
//! ```

#[path = "../src/api/sdl.rs"]
// Its `schema()` is for the crate's own tests, so unused in this one's.
#[cfg_attr(test, allow(dead_code))]
mod sdl;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use hardcover_core::auth::{DEFAULT_SCOPES, KeyringTokenStore, OAuthConfig, Session, StaticToken};
use hardcover_core::{TokenSource, USER_AGENT};
use serde_json::Value;

const ENDPOINT: &str = "https://api.hardcover.app/v1/graphql";
const CAPABILITIES: &str = "https://api.hardcover.app/capabilities.json";
const HEADER: &str = "# Generated from Hardcover GraphQL introspection (https://api.hardcover.app/v1/graphql). Do not edit by hand.\n";

const INTROSPECTION: &str = "query IntrospectionQuery { __schema { types { kind name \
    fields(includeDeprecated: true) { name args { name type { ...T } } type { ...T } isDeprecated deprecationReason } \
    inputFields { name type { ...T } } enumValues(includeDeprecated: true) { name } } } } \
    fragment T on __Type { kind name ofType { kind name ofType { kind name ofType { kind name \
    ofType { kind name ofType { kind name ofType { kind name } } } } } } }";

const USAGE: &str = "usage: api_check [--check] [--summary <path>]";

/// The summary's one heading.
const SUMMARY_HEADING: &str = "## Live API: schema and scopes (api_check)";

/// At most this many lines of each list go into the summary; the log has all.
const SUMMARY_MAX_LINES: usize = 200;

type Error = Box<dyn std::error::Error>;

fn main() -> ExitCode {
    let args = match Args::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(args.check) {
        Ok(report) => {
            if let Some(path) = &args.summary {
                append_summary(path, &report.markdown(args.check));
            }
            ExitCode::from(report.outcome().exit_code(args.check))
        }
        Err(e) => {
            eprintln!("error: {e}");
            if let Some(path) = &args.summary {
                append_summary(path, &setup_failure_markdown(&e.to_string()));
            }
            ExitCode::from(2)
        }
    }
}

struct Args {
    /// Compare only; write nothing to `schema/`.
    check: bool,
    /// Where to append the Markdown summary.
    summary: Option<PathBuf>,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
        let mut parsed = Args {
            check: false,
            summary: None,
        };
        while let Some(arg) = args.next() {
            let path = if arg == "--check" {
                parsed.check = true;
                continue;
            } else if arg == "--summary" {
                args.next().unwrap_or_default()
            } else if let Some(path) = arg.strip_prefix("--summary=") {
                path.to_owned()
            } else {
                return Err(format!("unknown argument {arg:?}"));
            };
            if path.is_empty() {
                return Err("--summary needs a path".into());
            }
            parsed.summary = Some(PathBuf::from(path));
        }
        Ok(parsed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// Nothing broken, and `schema/` matches the live API.
    Ok,
    /// Nothing broken, but the live schema or scope map differs from `schema/`.
    Drift,
    /// A document no longer validates, or a scope no longer covers it.
    Broken,
}

impl Outcome {
    fn exit_code(self, check_only: bool) -> u8 {
        match self {
            Outcome::Broken => 1,
            Outcome::Drift if check_only => 3,
            Outcome::Drift | Outcome::Ok => 0,
        }
    }
}

/// What one run found: the exit code and the summary are worked out from it.
#[derive(Default)]
struct Report {
    /// Documents that no longer validate: (name, the validator's message).
    broken_documents: Vec<(String, String)>,
    /// Uses the app's scopes no longer grant.
    not_granted: Vec<String>,
    /// Ungranted fields of input-object variables, which may never be sent.
    notes: Vec<String>,
    /// The schema diff's lines, headline first, when the schema changed.
    schema_diff: Option<Vec<String>>,
    /// The scope map diff's lines, headline first, when it changed.
    scope_diff: Option<Vec<String>>,
}

impl Report {
    fn drifted(&self) -> bool {
        self.schema_diff.is_some() || self.scope_diff.is_some()
    }

    fn outcome(&self) -> Outcome {
        if !self.broken_documents.is_empty() || !self.not_granted.is_empty() {
            Outcome::Broken
        } else if self.drifted() {
            Outcome::Drift
        } else {
            Outcome::Ok
        }
    }

    fn markdown(&self, check_only: bool) -> String {
        let mut md = format!("{SUMMARY_HEADING}\n\n");
        md.push_str(match (self.outcome(), check_only) {
            (Outcome::Ok, _) => {
                "**Outcome: ok.** Every document validates against the live schema, \
                 the app's scopes cover them, and `schema/` matches the live API."
            }
            (Outcome::Drift, true) => {
                "**Outcome: drift.** Nothing is broken, but the live API differs from \
                 `schema/`: run `just schema-update` and review `git diff schema/`."
            }
            (Outcome::Drift, false) => {
                "**Outcome: drift.** Nothing is broken; `schema/` was updated to match \
                 the live API: review `git diff schema/`."
            }
            (Outcome::Broken, _) if self.drifted() => {
                "**Outcome: broken.** The app is broken against the live API. The live \
                 API also differs from `schema/` (below)."
            }
            (Outcome::Broken, _) => "**Outcome: broken.** The app is broken against the live API.",
        });
        md.push_str("\n\n");

        if !self.broken_documents.is_empty() {
            let lines: Vec<String> = self
                .broken_documents
                .iter()
                .flat_map(|(name, error)| {
                    std::iter::once(format!("{name}:"))
                        .chain(error.lines().map(|l| format!("  {l}")))
                })
                .collect();
            let label = format!(
                "Documents that no longer validate ({})",
                self.broken_documents.len()
            );
            md_block(&mut md, &label, "text", &lines);
        }
        if !self.not_granted.is_empty() {
            let label = format!(
                "Not granted by the app's scopes ({})",
                self.not_granted.len()
            );
            md_block(&mut md, &label, "text", &self.not_granted);
        }
        if !self.notes.is_empty() {
            md_block(&mut md, "Notes", "text", &self.notes);
        }
        for (label, diff) in [
            ("Schema changes", &self.schema_diff),
            ("Scope map changes", &self.scope_diff),
        ] {
            let Some((headline, lines)) = diff.as_deref().and_then(<[String]>::split_first) else {
                continue;
            };
            let label = format!("{label}: {}", headline.trim());
            let lines: Vec<String> = lines
                .iter()
                .map(|l| l.strip_prefix("  ").unwrap_or(l).to_owned())
                .collect();
            md_block(&mut md, &label, "diff", &lines);
        }
        md
    }
}

/// Appends a bold label and, when there are any, the lines in a fenced block,
/// where Markdown leaves `+`, `_` and `[T!]` alone, cut at
/// [`SUMMARY_MAX_LINES`].
fn md_block(md: &mut String, label: &str, lang: &str, lines: &[String]) {
    let _ = write!(md, "**{label}**\n\n");
    if lines.is_empty() {
        return;
    }
    let _ = writeln!(md, "```{lang}");
    for line in lines.iter().take(SUMMARY_MAX_LINES) {
        // A fence inside would end the block early.
        let _ = writeln!(md, "{}", line.replace("```", "` ` `"));
    }
    if lines.len() > SUMMARY_MAX_LINES {
        let more = lines.len() - SUMMARY_MAX_LINES;
        let _ = writeln!(md, "… and {more} more (see the job log)");
    }
    md.push_str("```\n\n");
}

fn setup_failure_markdown(error: &str) -> String {
    let mut md = format!(
        "{SUMMARY_HEADING}\n\n**Outcome: could not run.** The check did not finish, \
         so it says nothing about the API.\n\n"
    );
    md_block(&mut md, "Error", "text", &[error.to_owned()]);
    md
}

fn append_summary(path: &Path, markdown: &str) {
    let result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(markdown.as_bytes()));
    if let Err(e) = result {
        eprintln!(
            "warning: could not write the summary to {}: {e}",
            path.display()
        );
    }
}

fn run(check_only: bool) -> Result<Report, Error> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let schema_path = root.join("schema/hardcover.graphql");
    let caps_path = root.join("schema/hardcover-capabilities.json");
    let mut report = Report::default();

    // ----- schema -----
    let (tokens, source) = token_source()?;
    println!("Introspecting {ENDPOINT} with {source}");
    let introspection = graphql(&*tokens, INTROSPECTION)?;
    let live_sdl = render_sdl(&introspection)?;
    let old_sdl = std::fs::read_to_string(&schema_path).unwrap_or_default();
    let schema_changed = live_sdl != old_sdl;
    if schema_changed {
        let lines = schema_diff(&old_sdl, &live_sdl);
        print_lines(&lines);
        report.schema_diff = Some(lines);
    } else {
        println!("  schema unchanged");
    }

    let live = sdl::parse(&live_sdl).map_err(|e| format!("live schema does not parse:\n{e}"))?;
    for (name, doc) in hardcover_core::api::DOCUMENTS {
        if let Err(e) = sdl::validate(&live, name, doc) {
            println!("  BROKEN  {name}:\n{}", indent(&e, 10));
            report.broken_documents.push(((*name).to_owned(), e));
        }
    }
    if report.broken_documents.is_empty() {
        println!(
            "  all {} documents validate against the live schema",
            hardcover_core::api::DOCUMENTS.len()
        );
    }

    // ----- OAuth scopes -----
    println!("\nFetching {CAPABILITIES}");
    let caps = get_json(CAPABILITIES)?;
    let usages: Vec<(&str, sdl::Usage)> = hardcover_core::api::DOCUMENTS
        .iter()
        .filter_map(|(name, doc)| Some((*name, sdl::usage(&live, name, doc)?)))
        .collect();
    let scopes = sdl::check_scopes(&caps, &DEFAULT_SCOPES, &usages)?;
    if scopes.problems.is_empty() {
        println!(
            "  the app's scopes ({}) cover every operation, column and argument it uses",
            DEFAULT_SCOPES.join(" ")
        );
    } else {
        for p in &scopes.problems {
            println!("  NOT GRANTED  {p}");
        }
    }
    for note in &scopes.notes {
        println!("  note: {note}");
    }
    report.not_granted = scopes.problems;
    report.notes = scopes.notes;
    let caps_json = serde_json::to_string_pretty(&caps)? + "\n";
    let old_caps = std::fs::read_to_string(&caps_path).unwrap_or_default();
    let caps_changed = caps_json != old_caps;
    if caps_changed {
        let lines = scope_diff(&old_caps, &caps);
        print_lines(&lines);
        report.scope_diff = Some(lines);
    } else {
        println!("  scope map unchanged");
    }

    // ----- result -----
    println!();
    if schema_changed || caps_changed {
        if check_only {
            println!("The live API differs from schema/. Run without --check to update it.");
        } else {
            write(&schema_path, &live_sdl, schema_changed)?;
            write(&caps_path, &caps_json, caps_changed)?;
            println!("Updated schema/. Review with `git diff schema/` and run `cargo test`.");
        }
    } else {
        println!("schema/ matches the live API.");
    }
    if report.outcome() == Outcome::Broken {
        println!("The app is BROKEN against the live API: see BROKEN / NOT GRANTED above.");
    }
    Ok(report)
}

fn print_lines(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

fn token_source() -> Result<(Arc<dyn TokenSource>, &'static str), Error> {
    if let Some(token) = StaticToken::from_env() {
        return Ok((Arc::new(token), "HARDCOVER_TOKEN"));
    }
    let cfg = OAuthConfig::default();
    let session = Session::restore(
        cfg.clone(),
        Arc::new(KeyringTokenStore::for_client(cfg.client_id())?),
    )?
    .ok_or("no token: set HARDCOVER_TOKEN or sign in with the app (or the oauth_login example)")?;
    Ok((Arc::new(session), "the keyring session"))
}

fn graphql(tokens: &dyn TokenSource, query: &str) -> Result<Value, Error> {
    let token = tokens.access_token()?;
    let mut response = ureq::post(ENDPOINT)
        .header("user-agent", USER_AGENT)
        .header("authorization", &format!("Bearer {token}"))
        .send_json(serde_json::json!({ "query": query }))?;
    let body: Value = response
        .body_mut()
        .with_config()
        .limit(64 * 1024 * 1024)
        .read_json()?;
    if let Some(errors) = body.get("errors") {
        return Err(format!("GraphQL errors: {errors}").into());
    }
    Ok(body)
}

fn get_json(url: &str) -> Result<Value, Error> {
    Ok(ureq::get(url)
        .header("user-agent", USER_AGENT)
        .call()?
        .body_mut()
        .with_config()
        .limit(16 * 1024 * 1024)
        .read_json()?)
}

// ----- SDL rendering (the format of schema/hardcover.graphql) -----

fn render_sdl(introspection: &Value) -> Result<String, Error> {
    let mut types: Vec<&Value> = introspection["data"]["__schema"]["types"]
        .as_array()
        .ok_or("introspection has no types")?
        .iter()
        .collect();
    types.sort_by_key(|t| t["name"].as_str().unwrap_or_default().to_owned());

    let mut out = vec![HEADER.to_owned()];
    for t in types {
        let name = t["name"].as_str().ok_or("type without a name")?;
        if name.starts_with("__") {
            continue;
        }
        match t["kind"].as_str() {
            Some("SCALAR") => out.push(format!("scalar {name}\n")),
            Some("ENUM") => {
                let values: Vec<&str> = list(&t["enumValues"])
                    .filter_map(|v| v["name"].as_str())
                    .collect();
                out.push(format!("enum {name} {{ {} }}\n", values.join(" ")));
            }
            Some(kind @ ("OBJECT" | "INPUT_OBJECT")) => {
                let keyword = if kind == "OBJECT" { "type" } else { "input" };
                out.push(format!("{keyword} {name} {{"));
                let fields = if kind == "OBJECT" {
                    &t["fields"]
                } else {
                    &t["inputFields"]
                };
                for f in list(fields) {
                    out.push(render_field(f)?);
                }
                out.push("}\n".to_owned());
            }
            other => return Err(format!("unsupported kind {other:?} for type {name}").into()),
        }
    }
    Ok(out.join("\n"))
}

fn render_field(f: &Value) -> Result<String, Error> {
    let name = f["name"].as_str().ok_or("field without a name")?;
    let args: Vec<String> = list(&f["args"])
        .map(|a| {
            Ok(format!(
                "{}: {}",
                a["name"].as_str().unwrap_or("?"),
                type_ref(&a["type"])?
            ))
        })
        .collect::<Result<_, Error>>()?;
    let args = if args.is_empty() {
        String::new()
    } else {
        format!("({})", args.join(", "))
    };
    let mut line = format!("  {name}{args}: {}", type_ref(&f["type"])?);
    if f["isDeprecated"].as_bool() == Some(true) {
        let reason = f["deprecationReason"]
            .as_str()
            .unwrap_or("No longer supported");
        write!(
            line,
            " @deprecated(reason: {})",
            serde_json::to_string(reason)?
        )?;
    }
    Ok(line)
}

fn type_ref(t: &Value) -> Result<String, Error> {
    match t["kind"].as_str() {
        Some("NON_NULL") => Ok(format!("{}!", type_ref(&t["ofType"])?)),
        Some("LIST") => Ok(format!("[{}]", type_ref(&t["ofType"])?)),
        _ => Ok(t["name"]
            .as_str()
            .ok_or("type reference without a name")?
            .to_owned()),
    }
}

fn list(v: &Value) -> impl Iterator<Item = &Value> {
    v.as_array().into_iter().flatten()
}

// ----- diff reports -----

/// Type name -> (declaration line, member lines), parsed from rendered SDL.
fn blocks(sdl: &str) -> BTreeMap<String, (String, Vec<String>)> {
    let mut out = BTreeMap::new();
    let mut current: Option<(String, String, Vec<String>)> = None;
    for line in sdl.lines() {
        if let Some((name, decl, members)) = current.as_mut() {
            if line == "}" {
                out.insert(name.clone(), (decl.clone(), std::mem::take(members)));
                current = None;
            } else {
                members.push(line.trim().to_owned());
            }
            continue;
        }
        let mut words = line.split_whitespace();
        let (Some(keyword), Some(name)) = (words.next(), words.next()) else {
            continue;
        };
        match keyword {
            "type" | "input" => current = Some((name.to_owned(), keyword.to_owned(), Vec::new())),
            "enum" => {
                let values = line
                    .split_once('{')
                    .map(|(_, rest)| rest.trim_end_matches('}').split_whitespace())
                    .into_iter()
                    .flatten()
                    .map(str::to_owned)
                    .collect();
                out.insert(name.to_owned(), ("enum".to_owned(), values));
            }
            "scalar" => {
                out.insert(name.to_owned(), ("scalar".to_owned(), Vec::new()));
            }
            _ => {}
        }
    }
    out
}

fn member_name(line: &str) -> &str {
    line.split(['(', ':']).next().unwrap_or(line).trim()
}

/// The schema's changes as printed: a headline, then one line per added,
/// removed or changed type or field.
fn schema_diff(old: &str, new: &str) -> Vec<String> {
    let (old, new) = (blocks(old), blocks(new));
    let mut breaking = 0;
    let mut lines = Vec::new();
    for (name, (kind, _)) in &new {
        if !old.contains_key(name) {
            lines.push(format!("  + {kind} {name}"));
        }
    }
    for (name, (kind, _)) in &old {
        if !new.contains_key(name) {
            lines.push(format!("  - {kind} {name}  (removed)"));
            breaking += 1;
        }
    }
    for (name, (kind, new_members)) in &new {
        let Some((_, old_members)) = old.get(name) else {
            continue;
        };
        let by_name = |members: &[String]| -> BTreeMap<String, String> {
            members
                .iter()
                .map(|m| (member_name(m).to_owned(), m.clone()))
                .collect()
        };
        let (o, n) = (by_name(old_members), by_name(new_members));
        for (member, line) in &n {
            match o.get(member) {
                None => lines.push(format!("  + {kind} {name}.{member}: {}", signature(line))),
                Some(before) if before != line => {
                    lines.push(format!(
                        "  ~ {kind} {name}.{member}: {} -> {}  (changed)",
                        signature(before),
                        signature(line)
                    ));
                    breaking += 1;
                }
                Some(_) => {}
            }
        }
        for member in o.keys().filter(|m| !n.contains_key(*m)) {
            lines.push(format!("  - {kind} {name}.{member}  (removed)"));
            breaking += 1;
        }
    }
    let headline = format!(
        "  schema changed: {} differences, {breaking} of them removals or changes",
        lines.len()
    );
    lines.insert(0, headline);
    lines
}

fn signature(line: &str) -> &str {
    let name = member_name(line);
    line.strip_prefix(name)
        .map_or(line, |s| s.trim_start_matches(':').trim())
}

/// The scope map's changes as printed: a headline, then one line per scope
/// added or removed and per operation that moved.
fn scope_diff(old: &str, new: &Value) -> Vec<String> {
    let old: Value = serde_json::from_str(old).unwrap_or(Value::Null);
    if old.is_null() {
        return vec!["  no previous scope map; saving the first snapshot".to_owned()];
    }
    let ops = |caps: &Value| -> BTreeMap<String, BTreeSet<String>> {
        list(&caps["scopes"])
            .filter_map(|s| {
                let scope = s["scope"].as_str()?.to_owned();
                let ops = list(&s["operations"])
                    .filter_map(|o| o.as_str().map(str::to_owned))
                    .collect();
                Some((scope, ops))
            })
            .collect()
    };
    let (old, new) = (ops(&old), ops(new));
    let mut lines = vec!["  scope map changed:".to_owned()];
    for scope in old.keys().chain(new.keys()).collect::<BTreeSet<_>>() {
        let empty = BTreeSet::new();
        let (o, n) = (
            old.get(scope).unwrap_or(&empty),
            new.get(scope).unwrap_or(&empty),
        );
        if !old.contains_key(scope) {
            lines.push(format!("  + scope {scope}"));
        } else if !new.contains_key(scope) {
            lines.push(format!("  - scope {scope}  (removed)"));
        }
        for op in n.difference(o) {
            lines.push(format!("  + {scope}: {op}"));
        }
        for op in o.difference(n) {
            lines.push(format!("  - {scope}: {op}"));
        }
    }
    if lines.len() == 1 {
        lines.push(
            "  no operations moved between scopes; see `git diff schema/` for the details"
                .to_owned(),
        );
    }
    lines
}

fn write(path: &PathBuf, contents: &str, changed: bool) -> Result<(), Error> {
    if changed {
        std::fs::write(path, contents)?;
    }
    Ok(())
}

fn indent(text: &str, by: usize) -> String {
    let pad = " ".repeat(by);
    text.lines()
        .map(|l| format!("{pad}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

// Offline only: `cargo test -p hardcover-core --example api_check`.
#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        Args::parse(list.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn arguments() {
        let a = args(&["--check", "--summary", "out.md"]).unwrap();
        assert!(a.check);
        assert_eq!(a.summary, Some(PathBuf::from("out.md")));
        assert_eq!(
            args(&["--summary=x.md"]).unwrap().summary,
            Some(PathBuf::from("x.md"))
        );
        assert!(args(&["--summary"]).is_err());
        assert!(args(&["--summary="]).is_err());
        assert!(args(&["--write"]).is_err());
    }

    #[test]
    fn broken_wins_over_drift_and_drift_fails_only_a_check() {
        let mut report = Report::default();
        assert_eq!(report.outcome(), Outcome::Ok);
        assert_eq!(report.outcome().exit_code(true), 0);
        report.schema_diff = Some(vec!["  schema changed: 1 differences".into()]);
        assert_eq!(report.outcome(), Outcome::Drift);
        assert_eq!(report.outcome().exit_code(true), 3);
        assert_eq!(report.outcome().exit_code(false), 0);
        report.not_granted.push("X uses `y`".into());
        assert_eq!(report.outcome(), Outcome::Broken);
        assert_eq!(report.outcome().exit_code(true), 1);
        assert_eq!(report.outcome().exit_code(false), 1);
    }

    #[test]
    fn summary_lists_what_broke_and_what_changed() {
        let report = Report {
            broken_documents: vec![("BookDetails".into(), "no field `x`\nat 1:2".into())],
            schema_diff: Some(vec![
                "  schema changed: 2 differences, 1 of them removals or changes".into(),
                "  + type books.new_field: [Int!]".into(),
                "  - type books.x  (removed)".into(),
            ]),
            ..Report::default()
        };
        let md = report.markdown(true);
        assert_eq!(
            md.matches("\n## ").count() + usize::from(md.starts_with("## ")),
            1
        );
        assert!(md.contains("**Outcome: broken.**"));
        assert!(md.contains("```text\nBookDetails:\n  no field `x`\n  at 1:2\n```"));
        assert!(md.contains("**Schema changes: schema changed: 2 differences"));
        assert!(
            md.contains("```diff\n+ type books.new_field: [Int!]\n- type books.x  (removed)\n```")
        );
    }

    #[test]
    fn summary_cuts_long_lists() {
        let mut lines = vec!["  scope map changed:".to_owned()];
        lines.extend((0..SUMMARY_MAX_LINES + 5).map(|i| format!("  + read:x: op_{i}")));
        let report = Report {
            scope_diff: Some(lines),
            ..Report::default()
        };
        let md = report.markdown(true);
        assert!(md.contains("**Outcome: drift.**"));
        assert!(md.contains("just schema-update"));
        assert!(md.contains("… and 5 more"));
        assert!(!md.contains(&format!("op_{SUMMARY_MAX_LINES}\n")));
    }
}
