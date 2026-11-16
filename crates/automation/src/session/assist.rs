//! The `assist` module (extension ABI S6): what an agent reads and builds
//! that is not the document.
//!
//! - **Attached documents.** A host lends the session the documents a run
//!   was given ([`Documents`]: a datasheet's page text and rendered pages);
//!   `assist.document_text` and `assist.document_page` read them by title.
//!   Nothing is read from a path here, so the entries stay out of `files`.
//!   A session without attached documents refuses both, saying so.
//! - **Working pinout documents.** `assist.pinout_edit` builds a pinout
//!   document in chunks behind a handle (`pin_1`, …), so no single call has
//!   to carry a 100-row document; `assist.pinout_preview` previews it with
//!   `library.pinout_preview` and `assist.pinout_apply` imports exactly
//!   what was last previewed, by its base, without the document being sent
//!   again. A handle lasts as long as the session; a call may leave it out
//!   while one working document exists. Nothing here changes the project
//!   until `assist.pinout_apply`, which is one `library.pinout_apply`.
use super::Session;
use crate::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sparksmith_core::library::pinout::{
    MAX_PINOUT_PERIPHERALS, MAX_PINOUT_PINS, MAX_PINOUT_RULES, MAX_PINOUT_VARIANTS, PinoutAccept,
    PinoutTarget,
};
use sparksmith_core::{LibraryCommand, Operation, Query};
use std::collections::BTreeMap;

/// Pin rows one `assist.pinout_edit` call may carry: about 8 000 output
/// tokens of JSON, so a call never approaches a request's output limit.
pub const MAX_EDIT_PINS: usize = 40;
/// Working documents one session holds at once.
pub const MAX_WORKING: usize = 8;
/// Pages one `assist.document_text` call returns.
pub const MAX_TEXT_PAGES: usize = 200;
/// What a working document's handle starts with.
pub const HANDLE_PREFIX: &str = "pin_";
/// Findings, review items and changes a compact preview lists.
const MAX_PLAN_ITEMS: usize = 60;
/// The longest document title or handle a call may give, in bytes.
const MAX_NAME: usize = 256;

/// Documents a host attached for a run: read by title, never by path.
pub trait Documents: Send {
    /// The attached documents' titles, in order.
    fn titles(&self) -> Vec<String>;
    /// The text of `pages` (1-based; every page when empty) of `document`,
    /// each page marked `=== page N ===`.
    fn text(&mut self, document: &str, pages: &[u32]) -> std::result::Result<String, String>;
    /// Page `page` of `document`, rendered.
    fn page(&mut self, document: &str, page: u32) -> std::result::Result<PageImage, String>;
}

/// A rendered page: a line for the reader and the image.
#[derive(Clone, Debug)]
pub struct PageImage {
    pub text: String,
    pub media_type: String,
    /// Base64.
    pub data: String,
}

/// The session's `assist` state: attached documents and working pinouts.
#[derive(Default)]
pub(crate) struct Assist {
    documents: Option<Box<dyn Documents>>,
    working: BTreeMap<String, Working>,
    made: u32,
}

/// One working pinout document and what was last previewed from it.
#[derive(Clone, Debug, Default)]
struct Working {
    pinout: Map<String, Value>,
    /// The target, base and document of the last successful preview.
    previewed: Option<(Value, Value, Value)>,
}

/// `assist.document_text`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentTextParams {
    /// An attached document's title; may be left out when one is attached.
    #[serde(default)]
    pub document: Option<String>,
    /// Pages as `3`, `3-5` or `1, 4-6` (1-based); every page when left out.
    #[serde(default)]
    pub pages: Option<String>,
}

/// `assist.document_page`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentPageParams {
    /// An attached document's title; may be left out when one is attached.
    #[serde(default)]
    pub document: Option<String>,
    /// The 1-based page number, as the text marks it.
    pub page: u32,
}

/// `assist.pinout_edit`: change a working pinout document, all or nothing.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PinoutEditParams {
    /// The working document; may be left out while there is one.
    #[serde(default)]
    pub handle: Option<String>,
    /// Start another working document and answer with its handle.
    #[serde(default)]
    pub new: bool,
    /// Empty the working document first.
    #[serde(default)]
    pub clear: bool,
    #[serde(default)]
    pub format: Option<Value>,
    #[serde(default)]
    pub device: Option<Map<String, Value>>,
    /// Replaces the variants.
    #[serde(default)]
    pub variants: Option<Vec<Map<String, Value>>>,
    /// Upserted by label.
    #[serde(default)]
    pub peripherals: Vec<Map<String, Value>>,
    /// Upserted by row key, at most 40 a call.
    #[serde(default)]
    pub pins: Vec<Map<String, Value>>,
    /// Row keys to remove.
    #[serde(default)]
    pub remove_pins: Vec<Value>,
    /// Upserted by label.
    #[serde(default)]
    pub rules: Vec<Map<String, Value>>,
    /// Rule labels to remove.
    #[serde(default)]
    pub remove_rules: Vec<Value>,
}

/// `assist.pinout_preview`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkingPreviewParams {
    /// The working document; may be left out while there is one.
    #[serde(default)]
    pub handle: Option<String>,
    pub target: PinoutTarget,
    /// Replaces the working document before the preview.
    #[serde(default)]
    pub document: Option<Map<String, Value>>,
}

/// `assist.pinout_apply`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkingApplyParams {
    /// The revision the import was prepared against.
    pub expected_revision: u64,
    /// The working document; may be left out while there is one.
    #[serde(default)]
    pub handle: Option<String>,
    /// The last preview's base, exactly; left out, the last preview's.
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub base: Option<Value>,
    /// Rows to take; every row when left out.
    #[serde(default)]
    pub accept: Option<PinoutAccept>,
    /// The last preview's target when left out.
    #[serde(default)]
    pub target: Option<PinoutTarget>,
}

fn refused(code: &str, message: String) -> Error {
    Error::refused(Box::new(crate::front::Error {
        code: code.to_owned(),
        message,
        ..crate::front::Error::default()
    }))
}

impl Session {
    /// Lend the session `documents` for `assist.document_text` and
    /// `assist.document_page`, answering with what was attached before.
    pub fn attach_documents(
        &mut self,
        documents: Option<Box<dyn Documents>>,
    ) -> Option<Box<dyn Documents>> {
        std::mem::replace(&mut self.assist.documents, documents)
    }

    /// The title `asked` names among the attached documents.
    fn document_title(&self, asked: Option<&str>) -> Result<String> {
        let Some(documents) = &self.assist.documents else {
            return Err(refused(
                "assist.no_documents",
                "no documents are attached to this session: an assistant run attaches its datasheets; read files with your own tools".into(),
            ));
        };
        let titles = documents.titles();
        match asked {
            Some(title) if titles.iter().any(|t| t == title) => Ok(title.to_owned()),
            None if titles.len() == 1 => Ok(titles.into_iter().next().unwrap_or_default()),
            _ => Err(refused(
                "assist.unknown_document",
                format!("give document as one of: {}", titles.join(", ")),
            )),
        }
    }

    pub(super) fn document_text(&mut self, params: DocumentTextParams) -> Result<Value> {
        let title = self.document_title(bounded(params.document.as_deref())?)?;
        let pages = match params.pages.as_deref() {
            Some(text) => page_list(text).map_err(|why| refused("assist.pages", why))?,
            None => Vec::new(),
        };
        let documents =
            self.assist.documents.as_mut().ok_or_else(|| {
                refused("assist.no_documents", "no documents are attached".into())
            })?;
        let text = documents
            .text(&title, &pages)
            .map_err(|why| refused("assist.document", why))?;
        Ok(json!({"document": title, "text": text}))
    }

    pub(super) fn document_page(&mut self, params: DocumentPageParams) -> Result<Value> {
        let title = self.document_title(bounded(params.document.as_deref())?)?;
        let documents =
            self.assist.documents.as_mut().ok_or_else(|| {
                refused("assist.no_documents", "no documents are attached".into())
            })?;
        let page = documents
            .page(&title, params.page)
            .map_err(|why| refused("assist.document", why))?;
        Ok(json!({
            "document": title,
            "page": params.page,
            "text": page.text,
            "image": {"media_type": page.media_type, "data": page.data},
        }))
    }

    /// The handle `asked` names, a new one with `new`, or the only one
    /// (made when there is none and `make`).
    fn working_handle(&mut self, asked: Option<&str>, new: bool, make: bool) -> Result<String> {
        let asked = bounded(asked)?;
        if let Some(handle) = asked {
            if new {
                return Err(refused(
                    "assist.handle",
                    "give handle or new, not both".into(),
                ));
            }
            return if self.assist.working.contains_key(handle) {
                Ok(handle.to_owned())
            } else {
                Err(refused(
                    "assist.unknown_handle",
                    format!(
                        "no working pinout document has the handle {handle}; open ones: {}",
                        self.open_handles()
                    ),
                ))
            };
        }
        if !new {
            let mut open = self.assist.working.keys();
            match (open.next(), open.next()) {
                (Some(only), None) => return Ok(only.clone()),
                (Some(_), Some(_)) => {
                    return Err(refused(
                        "assist.ambiguous_handle",
                        format!(
                            "several working pinout documents are open ({}): give handle",
                            self.open_handles()
                        ),
                    ));
                }
                (None, _) if !make => {
                    return Err(refused(
                        "assist.no_working_document",
                        "there is no working pinout document: build one with assist.pinout_edit"
                            .into(),
                    ));
                }
                (None, _) => {}
            }
        }
        if self.assist.working.len() >= MAX_WORKING {
            return Err(refused(
                "assist.limit",
                format!(
                    "{MAX_WORKING} working pinout documents are open, as many as a session holds"
                ),
            ));
        }
        self.assist.made = self.assist.made.saturating_add(1);
        let handle = format!("{HANDLE_PREFIX}{}", self.assist.made);
        self.assist
            .working
            .insert(handle.clone(), Working::default());
        Ok(handle)
    }

    fn open_handles(&self) -> String {
        let open: Vec<&str> = self.assist.working.keys().map(String::as_str).collect();
        if open.is_empty() {
            "none".into()
        } else {
            open.join(", ")
        }
    }

    /// `assist.pinout_edit`: the working document changed all or nothing.
    pub(super) fn pinout_edit(&mut self, params: PinoutEditParams) -> Result<Value> {
        if params.pins.len() > MAX_EDIT_PINS {
            return Err(refused(
                "assist.pinout_edit",
                format!(
                    "send at most {MAX_EDIT_PINS} pin rows a call; split the rows over several calls"
                ),
            ));
        }
        let handle = self.working_handle(params.handle.as_deref(), params.new, true)?;
        let working = self
            .assist
            .working
            .get(&handle)
            .cloned()
            .unwrap_or_default();
        let mut doc = if params.clear {
            Map::new()
        } else {
            working.pinout
        };
        let changes = edit(&mut doc, params).map_err(|why| {
            refused(
                "assist.pinout_edit",
                format!("{why}; the working document is unchanged"),
            )
        })?;
        let summary = document_summary(&doc);
        if let Some(slot) = self.assist.working.get_mut(&handle) {
            slot.pinout = doc;
        }
        Ok(json!({"handle": handle, "changed": changes, "document": summary}))
    }

    /// `assist.pinout_preview`: the working document (or the given one,
    /// which replaces it) through `library.pinout_preview`, compacted.
    pub(super) fn working_preview(&mut self, params: WorkingPreviewParams) -> Result<Value> {
        // A preview of a given document starts a working document for it.
        let make = params.document.is_some();
        let handle = self.working_handle(params.handle.as_deref(), false, make)?;
        let mut working = self
            .assist
            .working
            .get(&handle)
            .cloned()
            .unwrap_or_default();
        if let Some(document) = params.document {
            working.pinout = document;
        }
        if working.pinout.is_empty() {
            return Err(refused(
                "assist.pinout_empty",
                "the working pinout document is empty: build it with assist.pinout_edit first"
                    .into(),
            ));
        }
        let target = serde_json::to_value(&params.target)?;
        let document = Value::Object(working.pinout.clone());
        working.previewed = None;
        let preview = serde_json::from_value::<Query>(json!({
            "query": "library.pinout_preview",
            "params": {"target": target, "document": document, "detail": true},
        }));
        let answer = match preview {
            Ok(query) => self
                .engine
                .query_using(
                    query,
                    &self.sources.context(),
                    Some(&sparksmith_solidcraft::footprint::SketchEvaluator),
                )
                .map_err(Error::from)
                .and_then(|result| Ok(serde_json::to_value(result)?)),
            Err(error) => Err(Error::from(error)),
        };
        let result = match answer {
            Ok(result) => result,
            Err(error) => {
                self.assist.working.insert(handle, working);
                return Err(error);
            }
        };
        // Query results come tagged (`{"report": …, "result": …}`).
        let plan = result.get("result").unwrap_or(&result).clone();
        working.previewed = Some((target, plan["base"].clone(), document));
        self.assist.working.insert(handle.clone(), working);
        let mut reply = compact_plan(&plan);
        reply["handle"] = json!(handle);
        Ok(reply)
    }

    /// `assist.pinout_apply`: `library.pinout_apply` with exactly the
    /// document last previewed, refusing a working document changed since.
    pub(super) fn working_apply(&mut self, params: WorkingApplyParams) -> Result<Value> {
        let handle = self.working_handle(params.handle.as_deref(), false, false)?;
        let working = self
            .assist
            .working
            .get(&handle)
            .cloned()
            .unwrap_or_default();
        let (target, base, document) =
            apply_inputs(&working, &params).map_err(|why| refused("assist.pinout_apply", why))?;
        let accept = params
            .accept
            .map(serde_json::to_value)
            .transpose()?
            .unwrap_or_else(|| json!({"accept": "all"}));
        let operation: Operation = serde_json::from_value(json!({
            "editor": "Library",
            "operation": {"command": "library.pinout_apply", "params": {
                "target": target, "document": document, "base": base, "accept": accept,
            }},
        }))?;
        if !matches!(
            operation,
            Operation::Library(LibraryCommand::PinoutApply(_))
        ) {
            return Err(refused("assist.pinout_apply", "not a pinout import".into()));
        }
        self.apply(params.expected_revision, operation)
    }
}

/// `text` when within bounds.
fn bounded(text: Option<&str>) -> Result<Option<&str>> {
    match text {
        Some(text) if text.len() > MAX_NAME => Err(refused(
            "arguments.invalid",
            format!("a name of {} bytes; at most {MAX_NAME}", text.len()),
        )),
        other => Ok(other),
    }
}

/// `1, 3-5` as page numbers, in order, without repeats.
fn page_list(text: &str) -> std::result::Result<Vec<u32>, String> {
    let mut pages = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let number = |n: &str| {
            n.trim()
                .parse::<u32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| format!("pages: {part} is not a page or a range such as 3-5"))
        };
        let (from, to) = match part.split_once('-') {
            Some((a, b)) => (number(a)?, number(b)?),
            None => (number(part)?, number(part)?),
        };
        if from > to {
            return Err(format!("pages: {part} runs backwards"));
        }
        for page in from..=to {
            if !pages.contains(&page) {
                pages.push(page);
            }
            if pages.len() > MAX_TEXT_PAGES {
                return Err(format!("pages: at most {MAX_TEXT_PAGES} pages a call"));
            }
        }
    }
    Ok(pages)
}

/// Apply `params`' changes to `doc`; the change counts, or why not.
fn edit(
    doc: &mut Map<String, Value>,
    params: PinoutEditParams,
) -> std::result::Result<BTreeMap<&'static str, usize>, String> {
    let mut changes = BTreeMap::new();
    if let Some(format) = params.format {
        doc.insert("format".into(), format);
    }
    if let Some(device) = params.device {
        doc.insert("device".into(), Value::Object(device));
    }
    if let Some(variants) = params.variants {
        if variants.len() > MAX_PINOUT_VARIANTS {
            return Err(format!("at most {MAX_PINOUT_VARIANTS} variants"));
        }
        doc.insert(
            "variants".into(),
            Value::Array(variants.into_iter().map(Value::Object).collect()),
        );
    }
    let (a, r) = upsert(
        doc,
        "peripherals",
        "label",
        params.peripherals,
        MAX_PINOUT_PERIPHERALS,
    )?;
    changes.insert("peripherals added", a);
    changes.insert("peripherals replaced", r);
    let (a, r) = upsert(doc, "pins", "row", params.pins, MAX_PINOUT_PINS)?;
    changes.insert("pins added", a);
    changes.insert("pins replaced", r);
    let (a, r) = upsert(doc, "rules", "label", params.rules, MAX_PINOUT_RULES)?;
    changes.insert("rules added", a);
    changes.insert("rules replaced", r);
    changes.insert(
        "pins removed",
        remove(doc, "pins", "row", &params.remove_pins),
    );
    changes.insert(
        "rules removed",
        remove(doc, "rules", "label", &params.remove_rules),
    );
    changes.retain(|_, n| *n > 0);
    Ok(changes)
}

/// Upsert `items` into the array `key` of `doc`, matching on `field`.
fn upsert(
    doc: &mut Map<String, Value>,
    key: &str,
    field: &str,
    items: Vec<Map<String, Value>>,
    limit: usize,
) -> std::result::Result<(usize, usize), String> {
    let list = doc
        .entry(key.to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(list) = list else {
        return Err(format!("{key} is not a list"));
    };
    let (mut added, mut replaced) = (0, 0);
    for item in items {
        let Some(id) = item.get(field).and_then(Value::as_str).map(str::to_owned) else {
            return Err(format!("each of {key} needs a text {field}"));
        };
        match list.iter_mut().find(|x| x[field].as_str() == Some(&id)) {
            Some(slot) => {
                *slot = Value::Object(item);
                replaced += 1;
            }
            None => {
                list.push(Value::Object(item));
                added += 1;
            }
        }
    }
    if list.len() > limit {
        return Err(format!("a pinout lists at most {limit} {key}"));
    }
    Ok((added, replaced))
}

fn remove(doc: &mut Map<String, Value>, key: &str, field: &str, ids: &[Value]) -> usize {
    let Some(Value::Array(list)) = doc.get_mut(key) else {
        return 0;
    };
    let before = list.len();
    list.retain(|x| !ids.iter().any(|id| x[field] == *id));
    before - list.len()
}

/// Counts of the working document and, per variant, the pin numbers it
/// does not yet list (when the variant numbers its terminals 1…N).
fn document_summary(doc: &Map<String, Value>) -> Value {
    let count = |key: &str| doc.get(key).and_then(Value::as_array).map_or(0, Vec::len);
    let pins = doc
        .get("pins")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut variants = Map::new();
    for v in doc
        .get("variants")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = v["name"].as_str() else {
            continue;
        };
        let total = v["terminal_count"].as_u64().unwrap_or(0);
        let mut listed: BTreeMap<String, usize> = BTreeMap::new();
        for p in &pins {
            if let Some(n) = p["number"][name].as_str() {
                *listed.entry(n.to_owned()).or_default() += 1;
            }
        }
        let missing: Vec<u64> = (1..=total.min(MAX_PINOUT_PINS as u64))
            .filter(|n| !listed.contains_key(&n.to_string()))
            .collect();
        let repeated: Vec<&String> = listed
            .iter()
            .filter(|(_, c)| **c > 1)
            .map(|(n, _)| n)
            .collect();
        variants.insert(
            name.to_owned(),
            json!({"terminal_count": total, "numbered": listed.len(), "missing": ranges(&missing), "repeated": repeated}),
        );
    }
    json!({
        "has_header": doc.contains_key("format") && doc.contains_key("device") && doc.contains_key("variants"),
        "pins": pins.len(),
        "peripherals": count("peripherals"),
        "rules": count("rules"),
        "variants": variants,
    })
}

/// "1-5, 9, 12-14".
fn ranges(numbers: &[u64]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut it = numbers.iter().copied().peekable();
    while let Some(start) = it.next() {
        let mut end = start;
        while it.peek() == Some(&(end + 1)) {
            end += 1;
            it.next();
        }
        out.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
    }
    out.join(", ")
}

/// The target, base and document `assist.pinout_apply` imports: the last
/// preview's, refusing a working document changed since or another base.
fn apply_inputs(
    working: &Working,
    params: &WorkingApplyParams,
) -> std::result::Result<(Value, Value, Value), String> {
    let Some((target, base, document)) = &working.previewed else {
        return Err("preview the working document first (assist.pinout_preview)".into());
    };
    if *document != Value::Object(working.pinout.clone()) {
        return Err("the working document changed since the last preview; preview it again".into());
    }
    let given = params.base.clone().unwrap_or(Value::Null);
    // The plan writes its base as a decimal string; a caller may copy it
    // as text or as an exact integer.
    let token = |value: &Value| match value {
        Value::String(text) => text.trim().parse::<u64>().ok(),
        other => other.as_u64(),
    };
    let same = given.is_null() || (token(&given).is_some() && token(&given) == token(base));
    if !same {
        let float = if given.is_f64() {
            " (it arrived as a rounded number: an integer this long loses digits as a float)"
        } else {
            ""
        };
        return Err(format!(
            "base {given} is not the last preview's base {base}{float}. Leave base out to apply exactly what was last previewed"
        ));
    }
    let target = match &params.target {
        Some(target) => serde_json::to_value(target).map_err(|e| e.to_string())?,
        None => target.clone(),
    };
    Ok((target, base.clone(), document.clone()))
}

/// A pinout plan without per-object detail: change counts (the changes
/// themselves when few), findings and review items without object lists,
/// and the rule checks.
pub fn compact_plan(plan: &Value) -> Value {
    let changes = plan["changes"].as_array().cloned().unwrap_or_default();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for c in &changes {
        let key = format!(
            "{} {}",
            c["object"].as_str().unwrap_or("?"),
            c["kind"].as_str().unwrap_or("?")
        );
        *counts.entry(key).or_default() += 1;
    }
    let findings: Vec<Value> = plan["findings"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|f| json!({"code": f["code"], "message": f["message"], "blocks": f["blocks"]}))
        .collect();
    let review = plan["review"].as_array().cloned().unwrap_or_default();
    let mut out = json!({
        "base": plan["base"],
        "change_counts": counts,
        "findings": findings.iter().take(MAX_PLAN_ITEMS).collect::<Vec<_>>(),
        "review": review.iter().take(MAX_PLAN_ITEMS).collect::<Vec<_>>(),
        "rules": plan["rules"].as_array().cloned().unwrap_or_default(),
    });
    if changes.len() <= MAX_PLAN_ITEMS {
        out["changes"] = Value::Array(
            changes
                .iter()
                .map(|c| json!({"kind": c["kind"], "object": c["object"], "row": c["row"], "summary": c["summary"]}))
                .collect(),
        );
    }
    for (key, n) in [("findings", findings.len()), ("review", review.len())] {
        if n > MAX_PLAN_ITEMS {
            out[format!("{key}_not_listed")] = json!(n - MAX_PLAN_ITEMS);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_lists_read_ranges_and_refuse_nonsense() {
        assert_eq!(page_list("1, 3-5, 4"), Ok(vec![1, 3, 4, 5]));
        assert!(page_list("5-3").is_err());
        assert!(page_list("0").is_err());
        assert!(page_list("a").is_err());
        assert!(page_list("1-100000").is_err());
    }

    #[test]
    fn missing_numbers_are_listed_as_ranges() {
        assert_eq!(ranges(&[1, 2, 3, 5, 7, 8]), "1-3, 5, 7-8");
        assert_eq!(ranges(&[]), "");
    }
}
