//! The same input schemas and examples that project.schema serves, checked
//! without executing any example (including examples of file operations).
use super::{discovery, metadata, surface};
use sparksmith_core::help::{Block, CallExample, Facts, HelpEntry};
use sparksmith_generator_abi::surface::Kind;
use sparksmith_io::{help, shipped::ShippedLibrary};
use std::path::Path;

type Fallible<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn check_example(example: &CallExample) -> Fallible {
    let name = example.entry.name();
    if example.entry.kind() != sparksmith_core::help::HelpKind::Entry {
        return Err(format!("{} is not a catalog entry", example.entry).into());
    }
    let (params, root) =
        discovery::params_schema(name).ok_or_else(|| format!("unknown entry {name}"))?;
    let mut schema = params.clone();
    if let (Some(obj), Some(defs)) = (schema.as_object_mut(), root.get("$defs")) {
        obj.insert("$defs".into(), defs.clone());
    }
    let validator = jsonschema::draft202012::new(&schema).map_err(|e| format!("{name}: {e}"))?;
    validator
        .validate(&example.params)
        .map_err(|e| format!("{name} example at {}: {e}", e.instance_path).into())
}

#[test]
fn example_with_wrong_parameter_type_fails_without_mutation() -> Fallible {
    let example = CallExample {
        entry: "entry/project.schema".parse()?,
        params: serde_json::json!({"name":314}),
    };
    assert!(check_example(&example).is_err());
    Ok(())
}

#[test]
fn entries_match_the_catalog() -> Fallible {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library");
    let library = ShippedLibrary::at(&root)?;
    let path = root.join("help/generated/entries.json");
    let mut rows = vec![];
    for module in surface::current().modules {
        for locked in module.entries {
            let meta =
                metadata::find(&locked.name).ok_or_else(|| format!("missing {}", locked.name))?;
            let id = format!("entry/{}", locked.name).parse::<sparksmith_core::help::HelpId>()?;
            let examples = discovery::examples(&locked.name)
                .into_iter()
                .map(|params| CallExample {
                    entry: id.clone(),
                    params,
                })
                .collect::<Vec<_>>();
            for example in &examples {
                check_example(example)?;
            }
            let entry_kind = meta.kind.map(|k| match k {
                metadata::Kind::Query => Kind::Query,
                metadata::Kind::Operation => Kind::Operation,
                metadata::Kind::Show => Kind::Show,
                metadata::Kind::Action => Kind::Action,
            });
            rows.push(HelpEntry {
                id,
                title: meta.title.into(),
                summary: meta.summary.clone(),
                since: "1.0".into(),
                facts: Facts::Entry {
                    module: module.id.clone(),
                    class: meta.class,
                    entry_kind,
                    schema_digest: locked.schema,
                    examples,
                },
                body: vec![],
                keywords: vec![],
                related: vec![],
            });
        }
    }
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    let next = serde_json::to_string_pretty(&rows)? + "\n";
    if std::env::var_os("SPARKSMITH_UPDATE_HELP").is_some() {
        std::fs::write(path, next)?;
    } else if help::source(&path)? != next {
        return Err("catalog help is stale; run cargo xtask help --update".into());
    }
    for entry in help::load(&library)?.into_values() {
        for block in entry.body {
            if let Block::Example(example) = block {
                check_example(&example).map_err(|e| format!("{}: {e}", entry.id))?;
            }
        }
    }
    Ok(())
}
