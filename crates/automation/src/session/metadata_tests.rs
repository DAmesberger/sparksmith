//! Catalog metadata (`docs/mcp-surface.md` §3, slice M2): every one of the
//! 237 entries has a title and a summary within budget, its kind, person,
//! long and result fields come from the catalog, and the sizes they promise
//! hold over MCP.
use super::metadata::{self, Annotations, Kind, Person, ResultSize};
use super::*;
use sparksmith_core::permission::MAX_TITLE;

type Fallible<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn names(family: &str) -> Vec<&'static str> {
    metadata::entries()
        .iter()
        .filter(|e| e.family == family)
        .map(|e| e.name.as_str())
        .collect()
}

#[test]
fn the_catalog_has_its_272_entries() {
    assert_eq!(names("command").len(), Request::ENTRIES.len());
    assert_eq!(names("operation").len(), Operation::entries().count());
    assert_eq!(names("query").len(), Query::ENTRIES.len());
    // The three tables above keep the per-family counts honest; this total
    // changes with every new request, operation or query. 262 after the fab
    // profile options; then fill.set_pad_override, component.place_board_only
    // and rules.set_project_source joined, and library.generate_pads went;
    // then schematic.set_net_disagreements joined, then
    // layout.set_refused_import_tracks, then library.package_from_generator;
    // then the nine template entries went (part-generation D2); then the two
    // turn entries joined (S4a); then the five assist entries (S6); then
    // the window's view.context, view.select and view.open (S5a); then
    // assist.workflows and assist.step_result (W1); then assist.offer (C2);
    // then layout.pads; then assist.ask; then library.replace_body,
    // library.body_fit and library.make_body; then project.preview_replace
    // and project.replace_component; then library.draft_part; then the six
    // variant.* operations and the variant.list and variant.substitutes
    // queries (assembly variants); then library.account and library.sign_out;
    // then variant.review_substitute; then schematic.pins.
    assert_eq!(metadata::entries().len(), 295);
}

/// A title is short, says more than the wire name, and is unique within its
/// family, so a person can tell two entries apart by it.
#[test]
fn every_entry_has_a_short_title() {
    for family in ["command", "operation", "query"] {
        let mut seen = std::collections::BTreeSet::new();
        for entry in metadata::entries().iter().filter(|e| e.family == family) {
            let title = entry.title;
            assert!(!title.is_empty(), "{} has no title", entry.name);
            assert!(title.len() <= MAX_TITLE, "{}: {title}", entry.name);
            assert_ne!(title, entry.name);
            assert!(seen.insert(title), "{family} title {title} twice");
        }
    }
}

/// `find` ranks by summaries, so none is empty, and none is cut: the whole
/// first sentence fits the budget.
#[test]
fn every_entry_has_a_one_line_summary_within_budget() {
    let mut failures = Vec::new();
    for entry in metadata::entries() {
        let summary = &entry.summary;
        if summary.is_empty() || summary.ends_with('…') || summary.len() > metadata::MAX_SUMMARY {
            failures.push(format!(
                "{} ({} bytes): {summary}",
                entry.name,
                summary.len()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every entry is declared beside its enum, in a declared module, and
/// what its declaration says reaches its metadata.
#[test]
fn every_entry_is_declared_in_a_module() {
    let modules: std::collections::BTreeSet<&str> = sparksmith_core::permission::MODULES
        .iter()
        .map(|m| m.id)
        .collect();
    for entry in metadata::entries() {
        assert!(
            modules.contains(entry.module),
            "{}: {}",
            entry.name,
            entry.module
        );
    }
    let entry = |name: &str| metadata::entries().iter().find(|e| e.name == name);
    let refill = entry("fill.refill");
    assert_eq!(refill.map(|e| (e.module, e.long)), Some(("layout", true)));
    let favourite = entry("library.favourite");
    assert!(favourite.is_some_and(|e| {
        e.reach
            .contains(sparksmith_core::permission::Reach::Settings)
    }));
    let open = entry("project.open");
    assert!(open.is_some_and(|e| e.module == "files"
        && e.reach.contains(sparksmith_core::permission::Reach::Files)
        && e.effect == sparksmith_core::permission::Effect::World));
    let via = entry("layout.add_via");
    assert!(via.is_some_and(
        |e| e.reach.is_empty() && e.effect == sparksmith_core::permission::Effect::Revision
    ));
}

#[test]
fn kind_and_person_follow_the_class_and_paths() {
    for entry in metadata::entries() {
        let expected = match (entry.class, entry.takes_path) {
            (Class::Never, _) => None,
            (Class::Ask, _) | (_, true) => Some(Kind::Action),
            (Class::Read, false) => Some(Kind::Query),
            (Class::Show, false) => Some(Kind::Show),
            (Class::Edit, false) => Some(Kind::Operation),
        };
        assert_eq!(entry.kind, expected, "{}", entry.name);
        if entry.kind == Some(Kind::Action) {
            assert_eq!(entry.person, Some(Person::Approval), "{}", entry.name);
        }
        let reviewed = entry.kind != Some(Kind::Action)
            && entry.class != Class::Never
            && sparksmith_core::permission::find(
                Request::ENTRIES
                    .iter()
                    .chain(Operation::entries())
                    .chain(Query::ENTRIES),
                &entry.name,
            )
            .is_some_and(|d| d.person == Person::Review);
        if reviewed {
            assert_eq!(entry.person, Some(Person::Review), "{}", entry.name);
        }
        if entry.family == "query" {
            assert_eq!(entry.kind, Some(Kind::Query), "{}", entry.name);
        }
    }
    let kind = |name: &str| {
        metadata::entries()
            .iter()
            .find(|e| e.name == name)
            .and_then(|e| e.kind)
    };
    assert_eq!(kind("layout.add_via"), Some(Kind::Operation));
    assert_eq!(kind("library.publish"), Some(Kind::Action));
    assert_eq!(kind("library.check_part_file"), Some(Kind::Action));
    assert_eq!(kind("project.issues"), Some(Kind::Query));
    assert_eq!(kind("library.source.add"), Some(Kind::Action));
    assert_eq!(kind("library.favourite"), None);
}

#[test]
fn annotations_follow_the_class() {
    for entry in metadata::entries() {
        let outside = entry.takes_path || !entry.reach.is_empty();
        let expected = match entry.class {
            Class::Read => Some(Annotations {
                read_only: true,
                destructive: None,
                idempotent: true,
                open_world: outside,
            }),
            Class::Show | Class::Edit => Some(Annotations {
                read_only: false,
                destructive: Some(false),
                idempotent: false,
                open_world: outside,
            }),
            Class::Ask => Some(Annotations {
                read_only: false,
                destructive: Some(true),
                idempotent: false,
                open_world: outside,
            }),
            Class::Never => None,
        };
        assert_eq!(entry.annotations, expected, "{}", entry.name);
    }
}

/// An entry marked inline answers within the inline budget on the shipped
/// showcase over MCP, and one marked linked can pass it there.
#[test]
fn inline_and_linked_results_hold_on_the_showcase() -> Fallible<()> {
    let tools = crate::mcp::testing::Tools::new(
        Session::new(showcase(
            sparksmith_core::library::standards::Context::fixture()?,
            &sparksmith_core::StackupPresets::fixture()?,
            sparksmith_core::rules::fixtures::prototype()?,
        )?),
        crate::gate::Grant::default(),
    )?;
    let size = |(failed, reply): (bool, Value)| -> usize {
        assert!(!failed, "{reply}");
        reply.to_string().len()
    };
    let result = |name: &str| {
        metadata::entries()
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.result)
    };
    // Its findings by code and drafts grow with the project: 3.5 KB here.
    assert_eq!(result("project.inspect"), Some(ResultSize::Link));
    let inspect = size(tools.query("project.inspect", json!({})));
    assert!(
        inspect > metadata::INLINE_RESULT,
        "inspect is {inspect} bytes"
    );
    assert_eq!(result("project.rename"), Some(ResultSize::Inline));
    let rename = size(tools.apply("project.rename", json!({"name": "Renamed"})));
    assert!(
        (1..=metadata::INLINE_RESULT).contains(&rename),
        "rename is {rename} bytes"
    );
    assert_eq!(result("project.issues"), Some(ResultSize::Link));
    let issues = size(tools.query("project.issues", json!({"scope": {"kind": "all"}})));
    assert!(issues > metadata::INLINE_RESULT, "issues is {issues} bytes");
    Ok(())
}

/// Each request the catalog describes is the declared entry of that name,
/// once, and its parameter schema is the one the dispatcher deserializes:
/// the `params` of that command in `Request`'s own schema, compared in
/// canonical form (definitions renumbered, doc comments and type names
/// left out).
#[test]
fn every_request_schema_is_its_variants_params() -> Fallible<()> {
    let catalog = Request::catalog();
    let listed: Vec<&str> = catalog.iter().map(|info| info.id).collect();
    let unique: std::collections::BTreeSet<&str> = listed.iter().copied().collect();
    assert_eq!(unique.len(), listed.len(), "a request is listed twice");
    let declared: std::collections::BTreeSet<&str> =
        Request::ENTRIES.iter().map(|decl| decl.name).collect();
    assert_eq!(unique, declared);

    let mut root = serde_json::to_value(schemars::schema_for!(Request))?;
    paths::unmark(&mut root);
    let variants = root["oneOf"].as_array().cloned().unwrap_or_default();
    assert_eq!(variants.len(), catalog.len());
    for info in &catalog {
        let variant = variants
            .iter()
            .find(|v| v["properties"]["command"]["const"] == info.id)
            .ok_or_else(|| format!("{} is not a command of Request", info.id))?;
        let wire = surface::resolve(&variant["properties"]["params"], &root);
        assert_eq!(
            surface::canonical(&info.params, &info.params),
            surface::canonical(wire, &root),
            "{}: the catalog's schema is not its variant's",
            info.id
        );
    }
    Ok(())
}
