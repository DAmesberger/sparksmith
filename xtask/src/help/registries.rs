use super::compare;
use crate::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use sparksmith_core::{
    help::{Facts, HelpEntry, HelpId},
    rules::RuleValue,
};
use sparksmith_io::{
    help, rule_profiles, settings,
    shipped::{Shelf, ShippedLibrary},
};
use std::{collections::BTreeMap, fmt::Write as _, path::Path};

type Shards = Vec<(&'static str, Vec<HelpEntry>)>;

pub(super) fn generate(library: &ShippedLibrary, root: &Path, update: bool) -> Result<Shards> {
    let (terms, markdown) = glossary(&library.shelf(Shelf::Help).join("glossary.json"))?;
    compare(&root.join("GLOSSARY.md"), &markdown, update)?;
    Ok(vec![
        ("glossary", terms),
        ("settings", settings_entries(library)?),
        ("rules", rules(library)?),
        ("generators", generators(library)?),
        ("workflows", workflows(library)?),
    ])
}

fn entry(id: &str, title: &str, summary: &str, facts: Facts) -> Result<HelpEntry> {
    Ok(HelpEntry {
        id: id.parse()?,
        title: title.into(),
        summary: summary.into(),
        since: env!("CARGO_PKG_VERSION").into(),
        facts,
        keywords: Vec::new(),
        related: Vec::new(),
        body: Vec::new(),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Term {
    id: HelpId,
    title: String,
    definition: String,
    aliases: Vec<String>,
    #[serde(default)]
    related: Vec<HelpId>,
    avoid: Vec<String>,
    developer_only: bool,
}

fn glossary(path: &Path) -> Result<(Vec<HelpEntry>, String)> {
    let terms: Vec<Term> = serde_json::from_str(&help::source(path)?)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut md = "# Sparksmith\n\n<!-- Generated from library/help/glossary.json by cargo xtask help --update. -->\n\nProposed vocabulary for the first-edition design. These terms distinguish reusable library definitions from their use in a circuit.\n\n## Language\n".to_owned();
    let mut entries = Vec::new();
    for t in terms {
        write!(md, "\n**{}**", t.title)?;
        if !t.aliases.is_empty() {
            write!(md, " ({})", t.aliases.join(", "))?;
        }
        writeln!(md, ":\n{}", t.definition)?;
        if !t.avoid.is_empty() {
            writeln!(md, "_Avoid_: {}", t.avoid.join(", "))?;
        }
        let mut row = entry(
            t.id.as_str(),
            &t.title,
            &t.definition,
            Facts::Term {
                definition: t.definition.clone(),
                avoid: t.avoid.clone(),
                developer_only: t.developer_only,
            },
        )?;
        row.related = t.related;
        row.keywords.extend(t.aliases);
        row.keywords.extend(t.avoid);
        entries.push(row);
    }
    Ok((entries, md))
}

fn sentence(label: &str) -> String {
    let words = label.replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

pub(super) fn settings_entries(library: &ShippedLibrary) -> Result<Vec<HelpEntry>> {
    use schemars::generate::SchemaSettings;
    let mut config = SchemaSettings::draft2020_12();
    config.inline_subschemas = true;
    let schema = config
        .into_generator()
        .into_root_schema_for::<settings::UserSettings>();
    let defaults_file: Value = serde_json::from_slice(&library.read(
        Shelf::Defaults,
        settings::DEFAULTS_FILE,
        64 * 1024,
    )?)?;
    // Check the installed defaults with the same parser Settings uses.
    settings::SettingsDefaults::load(library)?;
    let d = settings::UserSettings::default();
    let mut defaults = serde_json::to_value(&d)?;
    // Stored settings omit unchanged objects. Expand their actual defaults
    // for the reader, including the keyboard reveal delay and selection kinds.
    defaults["keyboard"] = serde_json::to_value(d.keyboard)?;
    defaults["selection_filter"] = serde_json::to_value(d.selection_filter)?;
    defaults["lists"] = serde_json::to_value(d.lists)?;
    let mut rows = Vec::new();
    setting_fields(
        &mut rows,
        "",
        schema.as_value(),
        Some(&defaults),
        &defaults_file,
    )?;
    Ok(rows)
}

fn setting_fields(
    rows: &mut Vec<HelpEntry>,
    parent: &str,
    object: &Value,
    defaults: Option<&Value>,
    shipped: &Value,
) -> Result<()> {
    let Some(properties) = object.get("properties").and_then(Value::as_object) else {
        return Ok(());
    };
    for (key, property) in properties {
        let field = if parent.is_empty() {
            key.clone()
        } else {
            format!("{parent}.{key}")
        };
        let mut schema = property.clone();
        let mut value = defaults.map_or(Value::Null, |obj| {
            obj.get(key)
                .or(property.get("default"))
                .cloned()
                .unwrap_or_else(|| match property["type"].as_str() {
                    Some("array") => json!([]),
                    Some("boolean") => json!(false),
                    _ => Value::Null,
                })
        });
        let mut source = "UserSettings default".to_owned();
        let mut unit = None;
        let mut id = field.clone();
        match field.as_str() {
            "impedance_tolerance" => {
                id = "impedance_tolerance_pct".into();
                value = shipped
                    .get(&id)
                    .cloned()
                    .ok_or("missing shipped impedance tolerance")?;
                source = shipped["sources"][&id]
                    .as_str()
                    .ok_or("missing impedance tolerance source")?
                    .into();
                unit = Some("%".into());
                let range = sparksmith_core::rules::signal::TOLERANCE_RANGE;
                schema["minimum"] = json!(range.start());
                schema["maximum"] = json!(range.end());
            }
            "keyboard.reveal.hold_ms" => {
                unit = Some("ms".into());
                schema["minimum"] = json!(settings::HOLD_MS_RANGE.start());
                schema["maximum"] = json!(settings::HOLD_MS_RANGE.end());
            }
            "browser_width" => {
                unit = Some("pt".into());
                schema["maximum"] = json!(settings::MAX_PANEL_WIDTH);
                schema["exclusiveMinimum"] = json!(0);
            }
            _ => {}
        }
        if let Some(limit) = match field.as_str() {
            "sources" => Some(settings::MAX_SOURCES),
            "registries" => Some(settings::MAX_REGISTRIES),
            "projects" => Some(settings::MAX_RECENT_PROJECTS),
            "production_folders" => Some(settings::MAX_PRODUCTION_FOLDERS),
            "part_files" => Some(settings::MAX_RECENT_PART_FILES),
            "favourites" => Some(settings::MAX_FAVOURITES),
            "recent" => Some(settings::MAX_RECENT),
            "keyboard.bindings" => Some(settings::MAX_BINDINGS),
            "lists.saved_filters" => Some(settings::MAX_SAVED_FILTERS),
            _ => None,
        } {
            schema["maxItems"] = json!(limit);
        }
        let title = sentence(key);
        let summary = schema["description"]
            .as_str()
            .unwrap_or(&title)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        rows.push(entry(
            &format!("setting/{id}"),
            &title,
            &summary,
            Facts::Setting {
                schema: schema.clone(),
                default: value.clone(),
                source,
                unit,
            },
        )?);
        let nested = defaults.and_then(|_| (!value.is_null()).then_some(&value));
        setting_fields(rows, &field, &schema, nested, shipped)?;
        if let Some(item) = schema.get("items") {
            setting_fields(rows, &field, item, None, shipped)?;
        }
        // A nullable object such as the saved window still has documented fields.
        if let Some(alts) = schema["anyOf"].as_array() {
            for alt in alts {
                setting_fields(rows, &field, alt, nested, shipped)?;
            }
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum RuleBasis {
    ProfileLimit,
    AuthorChoice,
}

fn rules(library: &ShippedLibrary) -> Result<Vec<HelpEntry>> {
    let mut out = Vec::new();
    for profile in rule_profiles::shipped_profiles(library)? {
        let path = library
            .shelf(Shelf::Profiles)
            .join("basis")
            .join(format!("{}.json", profile.id));
        let mut basis: BTreeMap<String, RuleBasis> = serde_json::from_str(&help::source(&path)?)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        for (key, _) in profile.entries() {
            let basis = basis
                .remove(&key.name())
                .ok_or_else(|| format!("{} needs a basis for {}", path.display(), key.name()))?;
            let resolved = key.get(profile.rules());
            let unit = match resolved {
                RuleValue::Length(_) => "mm",
                RuleValue::Area(_) => "mm²",
                RuleValue::Angle(_) => "°",
                RuleValue::Fraction(_) => "fraction",
                RuleValue::Ratio(_) => "ratio",
                RuleValue::Count(_) => "count",
                RuleValue::Flag(_) | RuleValue::Connection(_) => "",
            };
            let (_, source) = profile.active(key).ok_or("missing active profile rule")?;
            let row = if source.is_empty() {
                &profile.source.document
            } else {
                source
            };
            let value = serde_json::to_value(resolved)?;
            let title = format!("{} · {}", profile.name, key.label());
            out.push(entry(
                &format!("rule/{}/{}", profile.id, key.name()),
                &title,
                row,
                Facts::Rule {
                    value: value["value"].clone(),
                    unit: unit.into(),
                    row: row.into(),
                    source: profile.source.clone(),
                    author_choice: matches!(basis, RuleBasis::AuthorChoice),
                },
            )?);
        }
        if let Some(key) = basis.keys().next() {
            return Err(format!("{} names unknown rule {key}", path.display()).into());
        }
    }
    Ok(out)
}

fn generators(library: &ShippedLibrary) -> Result<Vec<HelpEntry>> {
    use sparksmith_generators::{Limits, WasmiRuntime};
    let runtime = WasmiRuntime::new(Limits::DEFAULT);
    let mut out = Vec::new();
    for name in library.generators()? {
        let bundle = library.generator(&name)?;
        let desc = runtime
            .load(&bundle.module)
            .and_then(|m| m.describe())
            .map_err(|e| format!("generator {name}: {e}"))?;
        let summary = desc
            .part()
            .and_then(|p| p.title.as_deref())
            .unwrap_or(&desc.title);
        let mut generator = entry(
            &format!("generator/{name}"),
            &desc.title,
            summary,
            Facts::Generator {
                parameters: desc.params().to_vec(),
                presets: desc.presets().to_vec(),
            },
        )?;
        if let Some(offer) = desc.offer() {
            generator.keywords.extend(offer.families.iter().cloned());
        }
        for p in desc.params() {
            let mut row = entry(
                &format!("generator/{name}#{}", p.key),
                &p.label,
                p.help.as_deref().unwrap_or(&p.label),
                Facts::Generator {
                    parameters: vec![p.clone()],
                    presets: vec![],
                },
            )?;
            row.related.push(generator.id.clone());
            generator.related.push(row.id.clone());
            out.push(row);
        }
        out.push(generator);
    }
    Ok(out)
}

#[derive(Deserialize)]
struct WorkflowSummary {
    id: String,
    title: String,
    summary: String,
    intents: Vec<String>,
}

fn workflows(library: &ShippedLibrary) -> Result<Vec<HelpEntry>> {
    let mut rows = Vec::new();
    for path in help::files(&library.shelf(Shelf::Assist).join("workflows"), "json")? {
        if path.file_name().is_none_or(|s| s != "workflow.json") {
            continue;
        }
        let w: WorkflowSummary = serde_json::from_str(&help::source(&path)?)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let mut row = entry(
            &format!("workflow/{}", w.id),
            &w.title,
            &w.summary,
            Facts::Workflow,
        )?;
        if path
            .parent()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            != Some(&w.id)
        {
            return Err(format!(
                "{} is not in the folder of workflow {}",
                path.display(),
                w.id
            )
            .into());
        }
        row.keywords = w.intents;
        rows.push(row);
    }
    Ok(rows)
}
