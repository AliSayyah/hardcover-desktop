//! Validates GraphQL documents against Hardcover's schema with apollo-compiler,
//! and checks them against Hardcover's OAuth scope map (`capabilities.json`).
//!
//! Shared by the unit tests (against the committed `schema/hardcover.graphql`
//! and `schema/hardcover-capabilities.json`) and the `api_check` example
//! (against the live ones), which includes this file with `#[path]` because
//! apollo-compiler is a dev-dependency.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use apollo_compiler::ast::{OperationType, Value as GqlValue};
use apollo_compiler::executable::{Selection, SelectionSet};
use apollo_compiler::schema::ExtendedType;
use apollo_compiler::validation::Valid;
use apollo_compiler::{ExecutableDocument, Schema};
use serde_json::Value;

/// The committed schema, parsed once.
pub fn schema() -> &'static Valid<Schema> {
    static SCHEMA: std::sync::OnceLock<Valid<Schema>> = std::sync::OnceLock::new();
    SCHEMA.get_or_init(|| {
        parse(include_str!("../../../../schema/hardcover.graphql"))
            .unwrap_or_else(|e| panic!("schema/hardcover.graphql is invalid:\n{e}"))
    })
}

/// Parses SDL in the format of `schema/hardcover.graphql`, adapting it for
/// apollo-compiler: the introspected SDL re-declares the built-in scalars and
/// names its roots `query_root`/`mutation_root`/`subscription_root`.
pub fn parse(raw: &str) -> Result<Valid<Schema>, String> {
    let mut sdl: String = raw
        .lines()
        .filter(|l| {
            !matches!(
                l.trim(),
                "scalar Boolean" | "scalar Float" | "scalar Int" | "scalar String" | "scalar ID"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    sdl.push_str(
        "\nschema { query: query_root mutation: mutation_root subscription: subscription_root }\n",
    );
    Schema::parse_and_validate(sdl, "hardcover.graphql").map_err(|e| e.errors.to_string())
}

pub fn validate(schema: &Valid<Schema>, name: &str, doc: &str) -> Result<(), String> {
    ExecutableDocument::parse_and_validate(schema, doc, format!("{name}.graphql"))
        .map(|_| ())
        .map_err(|e| e.errors.to_string())
}

// ----- OAuth scopes -----

/// What a document touches that a scope can gate.
#[derive(Debug, Default)]
pub struct Usage {
    /// Top-level fields: the scope map's `operations`.
    pub top_level: Vec<String>,
    /// Every field selected, at any depth: the scope map's `columns`.
    pub fields: BTreeSet<String>,
    /// A mutation's argument names and the keys of input objects written out
    /// in the document: the scope map's `arguments`.
    pub literal_arguments: BTreeSet<String>,
    /// The fields of a mutation's input-object variables, at any depth. The
    /// client builds those values at run time, so these are the arguments it
    /// *can* send, not the ones it does.
    pub variable_arguments: BTreeSet<String>,
}

/// `None` if the document does not validate.
pub fn usage(schema: &Valid<Schema>, name: &str, doc: &str) -> Option<Usage> {
    let doc =
        ExecutableDocument::parse_and_validate(schema, doc, format!("{name}.graphql")).ok()?;
    let mut usage = Usage::default();
    for op in doc.operations.iter() {
        usage.top_level.extend(
            op.selection_set
                .selections
                .iter()
                .filter_map(|s| s.as_field().map(|f| f.name.to_string())),
        );
        let mutation = op.operation_type == OperationType::Mutation;
        walk_selections(
            &doc,
            &op.selection_set,
            mutation,
            &mut usage,
            &mut BTreeSet::new(),
        );
        if mutation {
            for var in &op.variables {
                input_fields(
                    schema,
                    var.ty.inner_named_type().as_str(),
                    &mut usage.variable_arguments,
                    &mut BTreeSet::new(),
                );
            }
        }
    }
    Some(usage)
}

fn walk_selections(
    doc: &ExecutableDocument,
    set: &SelectionSet,
    mutation: bool,
    usage: &mut Usage,
    fragments: &mut BTreeSet<String>,
) {
    for selection in &set.selections {
        match selection {
            Selection::Field(field) => {
                usage.fields.insert(field.name.to_string());
                if mutation {
                    for arg in &field.arguments {
                        usage.literal_arguments.insert(arg.name.to_string());
                        literal_keys(&arg.value, &mut usage.literal_arguments);
                    }
                }
                walk_selections(doc, &field.selection_set, mutation, usage, fragments);
            }
            Selection::FragmentSpread(spread) => {
                let name = spread.fragment_name.to_string();
                if fragments.insert(name.clone())
                    && let Some(fragment) = doc.fragments.get(name.as_str())
                {
                    walk_selections(doc, &fragment.selection_set, mutation, usage, fragments);
                }
            }
            Selection::InlineFragment(inline) => {
                walk_selections(doc, &inline.selection_set, mutation, usage, fragments);
            }
        }
    }
}

fn literal_keys(value: &GqlValue, out: &mut BTreeSet<String>) {
    match value {
        GqlValue::Object(fields) => {
            for (key, value) in fields {
                out.insert(key.to_string());
                literal_keys(value, out);
            }
        }
        GqlValue::List(items) => items.iter().for_each(|v| literal_keys(v, out)),
        _ => {}
    }
}

fn input_fields(
    schema: &Schema,
    type_name: &str,
    out: &mut BTreeSet<String>,
    seen: &mut BTreeSet<String>,
) {
    if !seen.insert(type_name.to_owned()) {
        return;
    }
    if let Some(ExtendedType::InputObject(input)) = schema.types.get(type_name) {
        for (name, field) in &input.fields {
            out.insert(name.to_string());
            input_fields(schema, field.ty.inner_named_type().as_str(), out, seen);
        }
    }
}

/// What some scopes grant, following `implies`.
#[derive(Debug, Default)]
pub struct Grants {
    pub operations: BTreeSet<String>,
    pub arguments: BTreeSet<String>,
    pub columns: BTreeSet<String>,
    /// Requested scopes the map does not list.
    pub unknown: Vec<String>,
}

fn strings(v: &Value) -> impl Iterator<Item = &str> {
    v.as_array().into_iter().flatten().filter_map(Value::as_str)
}

fn scope_entries(caps: &Value) -> BTreeMap<&str, &Value> {
    caps["scopes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| Some((s["scope"].as_str()?, s)))
        .collect()
}

pub fn grants(caps: &Value, scopes: &[&str]) -> Result<Grants, String> {
    let by_scope = scope_entries(caps);
    if by_scope.is_empty() {
        return Err("capabilities.json lists no scopes".into());
    }
    let mut grants = Grants::default();
    let mut pending: Vec<&str> = scopes.to_vec();
    let mut seen = BTreeSet::new();
    while let Some(scope) = pending.pop() {
        if !seen.insert(scope) {
            continue;
        }
        let Some(s) = by_scope.get(scope) else {
            grants.unknown.push(scope.to_owned());
            continue;
        };
        grants
            .operations
            .extend(strings(&s["operations"]).map(str::to_owned));
        grants
            .arguments
            .extend(strings(&s["arguments"]).map(str::to_owned));
        grants
            .columns
            .extend(strings(&s["columns"]).map(str::to_owned));
        pending.extend(strings(&s["implies"]));
    }
    grants.unknown.sort();
    Ok(grants)
}

/// Arguments and columns that some scope gates, each with the scopes that
/// grant it (`all` left out).
fn gated(caps: &Value, key: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (scope, s) in scope_entries(caps) {
        if scope == "all" {
            continue;
        }
        for name in strings(&s[key]) {
            out.entry(name.to_owned())
                .or_default()
                .push(scope.to_owned());
        }
    }
    out
}

/// The outcome of [`check_scopes`].
#[derive(Debug, Default)]
pub struct ScopeReport {
    /// The app is broken: a requested scope is gone, or a document uses
    /// something the requested scopes don't grant.
    pub problems: Vec<String>,
    /// Arguments a document can send through a variable that the requested
    /// scopes don't grant. Fine as long as the client never sends them.
    pub notes: Vec<String>,
}

/// Checks documents against the scope map for the scopes the app requests.
///
/// Operations (top-level fields), columns (selected fields) and arguments
/// written out in a document are checked exactly. Arguments inside a
/// variable's value can't be seen here, so for those only a note is given.
pub fn check_scopes(
    caps: &Value,
    scopes: &[&str],
    documents: &[(&str, Usage)],
) -> Result<ScopeReport, String> {
    let grants = grants(caps, scopes)?;
    let gated_columns = gated(caps, "columns");
    let gated_arguments = gated(caps, "arguments");
    let mut report = ScopeReport::default();
    for scope in &grants.unknown {
        report
            .problems
            .push(format!("scope {scope} is no longer in the scope map"));
    }
    let needs = |gated: &BTreeMap<String, Vec<String>>, name: &str| -> String {
        gated.get(name).map(|s| s.join(" or ")).unwrap_or_default()
    };
    for (name, usage) in documents {
        for field in &usage.top_level {
            if !grants.operations.contains(field) {
                report.problems.push(format!("{name} uses `{field}`"));
            }
        }
        for column in usage
            .fields
            .iter()
            .filter(|f| gated_columns.contains_key(*f))
        {
            if !grants.columns.contains(column) {
                report.problems.push(format!(
                    "{name} selects `{column}`, which needs {}",
                    needs(&gated_columns, column)
                ));
            }
        }
        for arg in usage
            .literal_arguments
            .iter()
            .filter(|a| gated_arguments.contains_key(*a))
        {
            if !grants.arguments.contains(arg) {
                report.problems.push(format!(
                    "{name} sends `{arg}`, which needs {}",
                    needs(&gated_arguments, arg)
                ));
            }
        }
        for arg in usage
            .variable_arguments
            .iter()
            .filter(|a| gated_arguments.contains_key(*a) && !usage.literal_arguments.contains(*a))
        {
            if !grants.arguments.contains(arg) {
                report.notes.push(format!(
                    "{name} can send `{arg}` in a variable, which needs {}; the app must not",
                    needs(&gated_arguments, arg)
                ));
            }
        }
    }
    Ok(report)
}
