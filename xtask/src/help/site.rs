//! Static release manual. Render into private staging directories so a
//! failure keeps the previous release and unrelated release folders intact.
use crate::Result;
use serde_json::{Value, json};
use sparksmith_core::help::{
    Block, CallExample, Facts, HelpEntry, HelpId, HelpKind, Index, Inline,
};
use sparksmith_io::shipped::{Shelf, ShippedLibrary};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::Path,
};

pub(super) fn render(out: &Path, index: &Index, library: &ShippedLibrary) -> Result<()> {
    let docs = out.join("docs");
    fs::create_dir_all(&docs)?;
    let version = env!("CARGO_PKG_VERSION");
    let stage = tempfile::Builder::new()
        .prefix(".help-")
        .tempdir_in(&docs)?;
    let media = library.shelf(Shelf::Help).join("media");
    super::validate_entries(index, &media)?;
    for name in [version, "latest"] {
        let existing = docs.join(name);
        if existing.try_exists()?
            && (existing.is_symlink() || !existing.join(".sparksmith-help").is_file())
        {
            return Err(format!(
                "{} already exists and is not a generated manual",
                existing.display()
            )
            .into());
        }
        let dir = stage.path().join(name);
        fs::create_dir(&dir)?;
        build(&dir, version, index, &media)?;
    }
    // Both copies have rendered. Preserve the old copies until installed.
    let mut installed = Vec::new();
    let result = (|| -> Result<()> {
        for name in [version, "latest"] {
            let target = docs.join(name);
            let old = stage.path().join(format!("previous-{name}"));
            let existed = target.try_exists()?;
            if existed {
                fs::rename(&target, &old)?;
            }
            installed.push((target.clone(), old, existed));
            fs::rename(stage.path().join(name), target)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        for (i, (target, old, existed)) in installed.into_iter().enumerate().rev() {
            if target.try_exists()? {
                fs::rename(&target, stage.path().join(format!("unused-{i}")))?;
            }
            if existed {
                fs::rename(old, target)?;
            }
        }
        return Err(error);
    }
    println!(
        "help: {} topics rendered in {}",
        index.len(),
        docs.display()
    );
    Ok(())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn page(id: &HelpId) -> String {
    match id.as_str().split_once('#') {
        Some((name, key)) => format!("{name}/parameters/{key}.html"),
        None => format!("{id}.html"),
    }
}

fn kind_name(kind: HelpKind) -> &'static str {
    match kind {
        HelpKind::Entry => "Catalog entries",
        HelpKind::Finding => "Findings",
        HelpKind::Command => "Commands",
        HelpKind::Setting => "Settings",
        HelpKind::Rule => "Fabrication rules",
        HelpKind::Generator => "Generators",
        HelpKind::Term => "Glossary",
        HelpKind::Guide => "Guides",
        HelpKind::Panel => "Panels",
        HelpKind::Workflow => "Workflows",
    }
}

fn start(title: &str, prefix: &str, version: &str) -> String {
    let product = esc(sparksmith_core::product::NAME);
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{} · {product} Help</title><link rel="stylesheet" href="{prefix}manual.css"></head><body><header><a href="{prefix}index.html">{product} Help</a><small>{}</small></header><main>"#,
        esc(title),
        esc(version)
    )
}

fn build(dir: &Path, version: &str, index: &Index, media: &Path) -> Result<()> {
    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/help/site");
    for asset in ["search.js", "manual.css"] {
        fs::copy(assets.join(asset), dir.join(asset))?;
    }
    let mut groups: BTreeMap<&str, Vec<&HelpEntry>> = BTreeMap::new();
    let mut search = Vec::new();
    let mut images = BTreeSet::new();
    for e in index.values() {
        let file = page(&e.id);
        let path = dir.join(&file);
        fs::create_dir_all(path.parent().ok_or("missing page folder")?)?;
        let ctx = Page {
            prefix: "../".repeat(file.matches('/').count()),
            index,
        };
        let mut html = start(&e.title, &ctx.prefix, version);
        write!(
            html,
            "<div class=\"context\">{}</div><h1>{}</h1>",
            kind_name(e.id.kind()),
            esc(&e.title)
        )?;
        if !e.summary.is_empty() && e.summary != e.title {
            write!(html, "<p>{}</p>", esc(&e.summary))?;
        }
        ctx.facts(&mut html, e)?;
        for block in &e.body {
            ctx.block(&mut html, block)?;
            if let Block::Image { file, .. } = block {
                images.insert(file);
            }
        }
        if !e.related.is_empty() {
            html.push_str("<h2>Related topics</h2><ul>");
            for related in &e.related {
                write!(html, "<li>{}</li>", ctx.link(related)?)?;
            }
            html.push_str("</ul>");
        }
        html.push_str("</main></body></html>");
        fs::write(path, html)?;
        groups.entry(kind_name(e.id.kind())).or_default().push(e);
        search.push(json!({"id":e.id,"title":e.title,"summary":e.summary,"kind":e.id.kind(),"keywords":e.keywords,"href":file}));
    }
    for image in images {
        let target = dir.join("media");
        fs::create_dir_all(&target)?;
        fs::copy(media.join(image), target.join(image))?;
    }
    let mut html = start("Topic index", "", version);
    html.push_str("<h1>Help</h1><label for=\"search\">Search topics</label><input type=\"search\" id=\"search\"><span id=\"count\" role=\"status\"></span>");
    for (kind, mut entries) in groups {
        write!(
            html,
            "<section data-group><h2>{kind}</h2><ul class=\"index\">"
        )?;
        entries.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
        for e in entries {
            let terms = format!(
                "{} {} {} {}",
                e.id,
                e.title,
                e.summary,
                e.keywords.join(" ")
            )
            .to_lowercase();
            write!(
                html,
                r#"<li data-terms="{}"><a href="{}">{}</a><p>{}</p></li>"#,
                esc(&terms),
                page(&e.id),
                esc(&e.title),
                esc(&e.summary)
            )?;
        }
        html.push_str("</ul></section>");
    }
    html.push_str(r#"</main><script src="search.js"></script></body></html>"#);
    fs::write(dir.join("index.html"), html)?;
    fs::write(dir.join("search.json"), serde_json::to_vec(&search)?)?;
    fs::write(dir.join(".sparksmith-help"), version)?;
    Ok(())
}

struct Page<'a> {
    prefix: String,
    index: &'a Index,
}
impl Page<'_> {
    fn link(&self, id: &HelpId) -> Result<String> {
        let e = self
            .index
            .get(id)
            .ok_or_else(|| format!("unresolved {id}"))?;
        Ok(format!(
            r#"<a href="{}{}">{}</a>"#,
            self.prefix,
            page(id),
            esc(&e.title)
        ))
    }
    fn spans(&self, parts: &[Inline]) -> Result<String> {
        let mut s = String::new();
        for span in parts {
            match span {
                Inline::Text(t) => s.push_str(&esc(t)),
                Inline::Reference(id) => s.push_str(&self.link(id)?),
                Inline::Key(id) => {
                    let e = self
                        .index
                        .get(id)
                        .ok_or_else(|| format!("unknown key {id}"))?;
                    if let Facts::Command { default_key, .. } = &e.facts {
                        write!(
                            s,
                            "<kbd>{}</kbd>",
                            esc(default_key.as_deref().unwrap_or("Unbound"))
                        )?;
                    } else {
                        return Err(format!("{id} is not a command").into());
                    }
                }
            }
        }
        Ok(s)
    }
    fn example(&self, out: &mut String, call: &CallExample) -> Result<()> {
        write!(
            out,
            "<p>{}</p><pre><code>{}</code></pre>",
            self.link(&call.entry)?,
            esc(&serde_json::to_string_pretty(&call.params)?)
        )?;
        Ok(())
    }
    fn block(&self, out: &mut String, block: &Block) -> Result<()> {
        match block {
            Block::Paragraph(spans) => write!(out, "<p>{}</p>", self.spans(spans)?)?,
            Block::Note(spans) => write!(out, "<aside>{}</aside>", self.spans(spans)?)?,
            Block::List(items) => {
                out.push_str("<ul>");
                for row in items {
                    write!(out, "<li>{}</li>", self.spans(row)?)?;
                }
                out.push_str("</ul>");
            }
            Block::Table(rows) => {
                out.push_str("<div class=\"table\"><table>");
                for (i, row) in rows.iter().enumerate() {
                    let tag = if i == 0 { "th" } else { "td" };
                    out.push_str("<tr>");
                    for cell in row {
                        write!(out, "<{tag}>{}</{tag}>", self.spans(cell)?)?;
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</table></div>");
            }
            Block::Image { file, alt, version } => write!(
                out,
                r#"<figure><img src="{}media/{}" alt="{}"><figcaption>Captured in {}</figcaption></figure>"#,
                self.prefix,
                esc(file),
                esc(alt),
                esc(version)
            )?,
            Block::Example(call) => self.example(out, call)?,
        }
        Ok(())
    }
    fn facts(&self, out: &mut String, e: &HelpEntry) -> Result<()> {
        use std::borrow::Cow;
        fn field(out: &mut String, name: &str, value: &str) -> std::fmt::Result {
            write!(out, "<dt>{}</dt><dd>{}</dd>", esc(name), esc(value))
        }
        fn display(v: &Value) -> Cow<'_, str> {
            if let Some(s) = v.as_str() {
                Cow::Borrowed(s)
            } else {
                Cow::Owned(v.to_string())
            }
        }
        let mut fields = String::new();
        let mut after = String::new();
        match &e.facts {
            Facts::Entry {
                module,
                class,
                entry_kind,
                schema_digest,
                examples,
            } => {
                field(&mut fields, "Module", module)?;
                field(&mut fields, "Class", class.name())?;
                if let Some(kind) = entry_kind {
                    field(&mut fields, "Kind", &display(&serde_json::to_value(kind)?))?;
                }
                field(&mut fields, "Schema digest", schema_digest)?;
                if !examples.is_empty() {
                    after.push_str("<h2>Example calls</h2>");
                    for call in examples {
                        self.example(&mut after, call)?;
                    }
                }
            }
            Facts::Finding {
                severity,
                blocks,
                group,
                fix,
                cause,
                explanation,
                fixed_by,
                checks,
                ..
            } => {
                field(
                    &mut fields,
                    "Severity",
                    &display(&serde_json::to_value(severity)?),
                )?;
                field(&mut fields, "Group", group)?;
                if let Some(stage) = blocks {
                    field(
                        &mut fields,
                        "Blocks",
                        &display(&serde_json::to_value(stage)?),
                    )?;
                }
                if let Some(id) = checks {
                    write!(fields, "<dt>Checks</dt><dd>{}</dd>", self.link(id)?)?;
                }
                write!(
                    after,
                    "<h2>Cause</h2><p>{}</p><h2>Fix</h2><p>{}</p>",
                    esc(cause),
                    esc(fix)
                )?;
                if !fixed_by.is_empty() {
                    after.push_str("<ul>");
                    for id in fixed_by {
                        write!(after, "<li>{}</li>", self.link(id)?)?;
                    }
                    after.push_str("</ul>");
                }
                if let Some(text) = explanation {
                    write!(after, "<p>{}</p>", esc(text))?;
                }
            }
            Facts::Command {
                context,
                default_key,
                reaches,
            } => {
                field(&mut fields, "Context", context)?;
                field(
                    &mut fields,
                    "Default key",
                    default_key.as_deref().unwrap_or("Unbound"),
                )?;
                if let Some(id) = reaches {
                    write!(fields, "<dt>Opens</dt><dd>{}</dd>", self.link(id)?)?;
                }
            }
            Facts::Setting {
                schema,
                unit,
                default,
                source,
            } => {
                field(&mut fields, "Default", &display(default))?;
                if let Some(u) = unit {
                    field(&mut fields, "Unit", u)?;
                }
                for (key, label) in [
                    ("type", "Type"),
                    ("minimum", "Minimum"),
                    ("maximum", "Maximum"),
                ] {
                    if let Some(v) = schema.get(key) {
                        field(&mut fields, label, &display(v))?;
                    }
                }
                field(&mut fields, "Default source", source)?;
            }
            Facts::Rule {
                value,
                unit,
                source,
                row,
                author_choice,
            } => {
                field(&mut fields, "Value", &format!("{} {unit}", display(value)))?;
                field(
                    &mut fields,
                    "Basis",
                    if *author_choice {
                        "Profile author’s choice"
                    } else {
                        "Fabrication profile limit"
                    },
                )?;
                field(&mut fields, "Publisher", &source.publisher)?;
                field(&mut fields, "Read on", &source.date)?;
                if source.url.starts_with("https://") || source.url.starts_with("http://") {
                    write!(
                        fields,
                        r#"<dt>Source</dt><dd><a href="{}">{}</a></dd>"#,
                        esc(&source.url),
                        esc(&source.document)
                    )?;
                } else {
                    field(&mut fields, "Source", &source.document)?;
                }
                if e.summary != *row {
                    field(&mut fields, "Source row", row)?;
                }
            }
            Facts::Term {
                definition, avoid, ..
            } => {
                if e.summary != *definition {
                    write!(after, "<p>{}</p>", esc(definition))?;
                }
                if !avoid.is_empty() {
                    field(&mut fields, "Words to avoid", &avoid.join(", "))?;
                }
            }
            Facts::Generator {
                parameters,
                presets,
            } => {
                if !parameters.is_empty() {
                    after.push_str("<h2>Parameters</h2><div class=\"table\"><table><tr><th>Parameter</th><th>Type and range</th><th>Default</th></tr>");
                    for param in parameters {
                        let base = e.id.as_str().split('#').next().unwrap_or(e.id.as_str());
                        let id: HelpId = format!("{base}#{}", param.key).parse()?;
                        write!(
                            after,
                            r#"<tr id="{}"><th>{}</th><td>{}</td><td>{}</td></tr>"#,
                            esc(&param.key),
                            self.link(&id)?,
                            esc(&parameter_type(&param.ty)),
                            esc(&parameter_value(&param.ty, &param.default)?)
                        )?;
                    }
                    after.push_str("</table></div>");
                }
                if !presets.is_empty() {
                    after.push_str("<h2>Presets</h2><ul>");
                    for preset in presets {
                        write!(
                            after,
                            "<li><strong>{}</strong> {}</li>",
                            esc(&preset.name),
                            esc(&preset.description)
                        )?;
                    }
                    after.push_str("</ul>");
                }
            }
            Facts::Guide | Facts::Panel | Facts::Workflow => {}
        }
        if !fields.is_empty() {
            write!(out, "<dl>{fields}</dl>")?;
        }
        out.push_str(&after);
        Ok(())
    }
}

use sparksmith_core::help::{parameter_type, parameter_value};
