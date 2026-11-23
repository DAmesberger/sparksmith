//! Compact replies for the MCP and JSON-lines transports: what a request did,
//! not the whole document. A reply names objects by their stable ids; with
//! `verbose: true` the same request returns the snapshot instead.
//!
//! The ids are found by diffing the serialized document, so a collection the
//! document gains later is reported without changing this module. Only the
//! sections a transaction wrote are serialized: a copy-on-write section
//! ([`Shared`]) both states still share is left out of both
//! ([`unshared`]), so a local edit never serializes packed assets or
//! collections it did not touch.
use super::{InspectParams, Request, Session};
use crate::Result;
use crate::front::{self, AlreadyAllowed, Asker, CallCx, NoOne, Surface, Target, from_request};
use crate::gate::Caller;
use serde_json::{Map, Value, json};
use sparksmith_core::issues::{IssueReport, IssueScope};
use sparksmith_core::{Class, Document, Finding, Project, Query, QueryResult, Shared, layout};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Ids listed per reply; beyond this `changes.omitted` counts the rest.
const MAX_IDS: usize = 200;
/// New findings listed per reply; beyond this `findings.omitted` counts the
/// rest and `project.issues` lists them all.
const MAX_FINDINGS: usize = 20;
/// Findings listed per changed draft.
const MAX_DRAFT_FINDINGS: usize = 40;
/// Snapshots a change names as one field: a fab profile switch names
/// `rules.profile`, not each of its rules' values and sources
/// (`rules.effective` lists them).
const WHOLE: &[&str] = &["rules.profile"];

/// The last document a compact reply described, kept so the next mutation
/// checks only its result. Holding the `Arc`s keeps the pointer comparison
/// sound. The report holds under the check settings it was made with: a
/// settings change (an impedance tolerance) at the same revision changes
/// findings that no edit made.
pub(super) struct Baseline {
    document: Document,
    checks: layout::CheckSettings,
    report: Arc<IssueReport>,
}

impl Baseline {
    /// `document` and its report of every issue under the engine's check
    /// settings: the engine's, made from the last report it made
    /// ([`sparksmith_core::Engine::issue_report_of`]).
    fn of(engine: &sparksmith_core::Engine, document: &Document) -> Result<Self> {
        Ok(Self {
            document: document.clone(),
            checks: *engine.check_settings(),
            report: engine.issue_report_of(document)?,
        })
    }

    /// Whether this is the report of `document` under `checks`.
    fn describes(&self, document: &Document, checks: &layout::CheckSettings) -> bool {
        Arc::ptr_eq(&self.document.project, &document.project)
            && Arc::ptr_eq(&self.document.drafts, &document.drafts)
            && self.checks == *checks
    }
}

/// `before` and `after` as JSON without the copy-on-write sections they
/// share: a [`Shared`] section both still hold, unchanged by construction,
/// is left out of both, so the diff serializes only what the transaction
/// wrote (a via edit: the vias, never the packed assets). Every other field
/// is serialized and compared in full, so a field the document gains later
/// is still reported.
fn unshared(before: &Project, after: &Project) -> Result<(Value, Value)> {
    let mut old = before.clone();
    let mut new = after.clone();
    macro_rules! leave_out_shared {
        ($($($field:ident).+),* $(,)?) => {
            $(
                if Shared::ptr_eq(&old.$($field).+, &new.$($field).+) {
                    old.$($field).+ = Shared::default();
                    new.$($field).+ = Shared::default();
                }
            )*
        };
    }
    leave_out_shared!(
        instances,
        occurrences,
        wires,
        junctions,
        nets,
        placements,
        tracks,
        vias,
        part_dimensions,
        model_traces,
        fills,
        keepouts,
        labels,
        sheet_symbols,
        drawings,
        interface,
        library.components,
        library.symbols,
        library.packages,
        library.footprints,
        library.bindings,
        library.quantity_kinds,
        library.fields,
        library.parts,
        library.models,
        library.package_rows,
        library.origins,
        library.assets,
        library.generated,
        library.reviews,
    );
    // A local edit changes a few of the board's many tracks and vias:
    // both sides leave out the ones they hold unchanged.
    if let Some((a, b)) = trim(&old.tracks, &new.tracks, |t| t.id) {
        (old.tracks, new.tracks) = (Shared::new(a), Shared::new(b));
    }
    if let Some((a, b)) = trim(&old.vias, &new.vias, |v| v.id) {
        (old.vias, new.vias) = (Shared::new(a), Shared::new(b));
    }
    Ok((serde_json::to_value(&old)?, serde_json::to_value(&new)?))
}

/// `old` and `new` without the elements both hold equal under one
/// identity, when that leaves their diff as it was: identities unique on
/// both sides (so the diff matches elements by identity) and some element
/// added, removed or changed (an order change alone is reported as a field,
/// so it keeps both whole). `None` keeps both whole.
fn trim<T: Clone + PartialEq>(
    old: &[T],
    new: &[T],
    id: impl Fn(&T) -> sparksmith_core::Id,
) -> Option<(Vec<T>, Vec<T>)> {
    let mut at: HashMap<sparksmith_core::Id, usize> = HashMap::with_capacity(new.len());
    for (k, item) in new.iter().enumerate() {
        if at.insert(id(item), k).is_some() {
            return None;
        }
    }
    let mut seen = std::collections::HashSet::with_capacity(old.len());
    let mut same = vec![false; new.len()];
    let mut kept = Vec::new();
    for item in old {
        if !seen.insert(id(item)) {
            return None;
        }
        match at.get(&id(item)) {
            Some(&k) if new.get(k) == Some(item) => {
                if let Some(s) = same.get_mut(k) {
                    *s = true;
                }
            }
            _ => kept.push(item.clone()),
        }
    }
    let rest: Vec<T> = new
        .iter()
        .zip(&same)
        .filter(|(_, same)| !**same)
        .map(|(item, _)| item.clone())
        .collect();
    (!kept.is_empty() || !rest.is_empty()).then_some((kept, rest))
}

/// The open findings of an issue report, each once: every row's finding
/// and the findings merged into it. The summary, the change replies and
/// Issues count from these same rows, so they never disagree on scope
/// (purchasing to-dos and component findings included).
fn open_findings(report: &IssueReport) -> Vec<&Finding> {
    report
        .rows
        .iter()
        .flat_map(|row| std::iter::once(&row.finding).chain(&row.also))
        .collect()
}

impl Session {
    /// The transports' entry point: one JSON request for `caller` as a call
    /// through the front door ([`crate::front::Surface::call`]), gated,
    /// run and answered in the shared envelope with a compact result. A refused request changes nothing and answers with
    /// the refusal. `verbose: true`, beside `command` or inside `params`,
    /// answers as [`Session::dispatch`] does.
    pub fn respond(&mut self, caller: Caller<'_>, request: Value) -> Value {
        self.answer(caller, request, false)
    }

    /// A dry run of one document edit for `caller`: what [`Session::respond`]
    /// would answer (outcome, changes, findings, drafts), run on a fork of
    /// the engine, so document, revision, history and path stay as they are.
    /// The gate decides as for the edit itself, except that what would ask
    /// a person runs, since nothing is kept. Only requests that change the
    /// engine and nothing else can be tried.
    pub fn rehearse(&mut self, caller: Caller<'_>, request: Value) -> Value {
        self.answer(caller, request, true)
    }

    /// [`Session::respond`] and [`Session::rehearse`]: the request as a
    /// call through the front door.
    fn answer(&mut self, caller: Caller<'_>, request: Value, dry_run: bool) -> Value {
        let (who, allowed) = caller.principal();
        let result = from_request(request).and_then(|(mut call, verbose)| {
            call.dry_run = dry_run;
            let mut no_one = NoOne;
            let mut allow = AlreadyAllowed;
            let asker: &mut dyn Asker = if allowed { &mut allow } else { &mut no_one };
            let cx = CallCx {
                who,
                asker,
                verbose,
                window: None,
            };
            self.call(cx, call)
        });
        front::envelope(result)
    }

    /// The dry run of `target` ([`Session::rehearse`]).
    pub(crate) fn rehearse_target(&mut self, target: Target) -> Result<Value> {
        let Target { request, expected } = target;
        // Only what changes the engine alone: a folder import, for one,
        // also notes a recent in the person's settings.
        let pure = matches!(
            request,
            Request::Apply(_)
                | Request::ApplyAll(_)
                | Request::Undo(_)
                | Request::Redo(_)
                | Request::ApplyUpdate(_)
                | Request::ReplaceComponent(_)
        );
        if !pure || request.effective_class() != Class::Edit {
            return Err(sparksmith_core::Error::Invalid(format!(
                "{} has no dry run: only document operations, undo, redo and updates do",
                request.subject()
            ))
            .into());
        }
        let mut fork = self.engine.fork();
        std::mem::swap(&mut self.engine, &mut fork);
        let path = self.path.clone();
        let baseline = self.baseline.take();
        let tried = self.run_compact(request, expected);
        std::mem::swap(&mut self.engine, &mut fork);
        self.path = path;
        self.baseline = baseline;
        let mut reply = tried?;
        reply["dry_run"] = json!(true);
        // The document is where it was: the next edit names this.
        reply["revision"] = json!(self.engine.revision());
        reply["can_undo"] = json!(self.engine.can_undo());
        reply["can_redo"] = json!(self.engine.can_redo());
        Ok(reply)
    }

    /// Run `request` at `expected` and answer with what it changed.
    pub(crate) fn run_compact(&mut self, request: Request, expected: Option<u64>) -> Result<Value> {
        let replaced = matches!(request, Request::Open(_) | Request::New(_));
        let before = self.engine.document().clone();
        self.compact = true;
        let result = self.run(request, expected);
        self.compact = false;
        let mut reply = result?;
        // A command that took a full snapshot itself still answers compactly.
        if let Some(object) = reply.as_object_mut() {
            for key in ["project", "drafts", "erc", "board_checks"] {
                object.remove(key);
            }
        }
        if replaced {
            reply["summary"] = self.summary()?;
        } else {
            self.add_changes(&before, &mut reply)?;
        }
        Ok(reply)
    }

    /// The document state a mutation answers with, before its own fields.
    pub(super) fn document_reply(&self) -> Value {
        if self.compact {
            json!({
                "revision": self.engine.revision(),
                "can_undo": self.engine.can_undo(),
                "can_redo": self.engine.can_redo(),
            })
        } else {
            self.snapshot()
        }
    }

    /// `project.inspect`: one part of the snapshot, the snapshot, or in a
    /// compact reply a summary.
    pub(super) fn inspect(&self, params: InspectParams) -> Result<Value> {
        if let Some(at) = params.at {
            let snapshot = self.snapshot();
            let value = snapshot
                .pointer(&at)
                .ok_or_else(|| sparksmith_core::Error::NotFound(nothing_at(&snapshot, &at)))?;
            return Ok(json!({"revision": self.engine.revision(), "at": at, "value": value}));
        }
        if self.compact {
            let mut reply = self.document_reply();
            reply["summary"] = self.summary()?;
            return Ok(reply);
        }
        Ok(self.snapshot())
    }

    /// Project name, object counts, drafts and the open findings: from
    /// `project.issues`' own rows, `open` findings and `by_code`, and
    /// `issues`, the rows by severity as Issues counts them.
    fn summary(&self) -> Result<Value> {
        let project = self.engine.project();
        let value = serde_json::to_value(project)?;
        let mut counts = Map::new();
        count_collections(&value, "", &mut counts);
        let QueryResult::Issues(report) = self.engine.query(Query::Issues {
            scope: IssueScope::All,
        })?
        else {
            return Err(sparksmith_core::Error::Invalid(
                "project.issues gave another report".into(),
            )
            .into());
        };
        let findings = open_findings(&report);
        let mut by_code: BTreeMap<&str, u64> = BTreeMap::new();
        for finding in &findings {
            *by_code.entry(&finding.code).or_default() += 1;
        }
        let drafts = self.draft_reports(None, 0)?;
        Ok(json!({
            "name": project.name,
            "path": self.path.as_ref().map(|p| p.display().to_string()),
            "counts": counts,
            "drafts": drafts,
            "findings": {"open": findings.len(), "by_code": by_code, "issues": report.counts},
        }))
    }

    /// Add `changes`, `findings` and `drafts` for what differs from `before`.
    fn add_changes(&mut self, before: &Document, reply: &mut Value) -> Result<()> {
        let after = self.engine.document().clone();
        let mut changes = Changes::default();
        let mut design = None;
        if !Arc::ptr_eq(&before.project, &after.project) {
            let old = match self.baseline.take() {
                Some(baseline) if baseline.describes(before, self.engine.check_settings()) => {
                    baseline
                }
                _ => Baseline::of(&self.engine, before)?,
            };
            let new = Baseline::of(&self.engine, &after)?;
            let (old_value, new_value) = unshared(&before.project, &after.project)?;
            changes.diff("", &old_value, &new_value);
            design = Some(findings_delta(
                &open_findings(&old.report),
                &open_findings(&new.report),
            ));
            self.baseline = Some(new);
        }
        let mut drafts = None;
        if !Arc::ptr_eq(&before.drafts, &after.drafts) {
            let old = serde_json::to_value(before.drafts.as_ref())?;
            let new = serde_json::to_value(after.drafts.as_ref())?;
            let mut delta = Changes::default();
            delta.diff("drafts", &old, &new);
            let ids: Vec<Value> = delta
                .added
                .get("drafts.items")
                .into_iter()
                .chain(delta.changed.get("drafts.items"))
                .flatten()
                .cloned()
                .collect();
            if !ids.is_empty() {
                drafts = Some(self.draft_reports(Some(&ids), MAX_DRAFT_FINDINGS)?);
            }
            changes.merge(delta);
        }
        if let Some(changes) = changes.into_value() {
            reply["changes"] = changes;
        }
        if let Some(design) = design.filter(|d| d["new"] != json!([]) || d["resolved"] != json!({}))
        {
            reply["findings"] = design;
        }
        if let Some(drafts) = drafts {
            reply["drafts"] = Value::Array(drafts);
        }
        Ok(())
    }

    /// Each selected draft (all without `only`) as id, kind, name, whether it
    /// is publishable, its finding count and up to `listed` of its findings.
    fn draft_reports(&self, only: Option<&[Value]>, listed: usize) -> Result<Vec<Value>> {
        let items = &self.engine.drafts().items;
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let QueryResult::Drafts(reports) =
            self.engine.query(Query::CheckDrafts { detail: true })?
        else {
            return Err(sparksmith_core::Error::Invalid(
                "component.check_drafts gave another report".into(),
            )
            .into());
        };
        let mut out = Vec::new();
        for (draft, report) in items.iter().zip(reports) {
            let draft = serde_json::to_value(draft)?;
            if only.is_some_and(|ids| identity(&draft).is_none_or(|id| !ids.contains(&id))) {
                continue;
            }
            let check = serde_json::to_value(QueryResult::Definition(report))?;
            let all = check["result"]["findings"].as_array();
            let findings: Vec<Value> = all
                .into_iter()
                .flatten()
                .take(listed)
                .map(|f| json!({"code": f["code"], "message": f["message"], "blocks": f["blocks"]}))
                .collect();
            let mut row = json!({
                "id": draft["id"],
                "kind": draft["definition"]["kind"],
                "name": draft["meta"]["name"],
                "publishable": check["result"]["publishable"],
                "finding_count": all.map_or(0, Vec::len),
            });
            if listed > 0 {
                row["findings"] = Value::Array(findings);
            }
            out.push(row);
        }
        Ok(out)
    }
}

/// The keys a refusal lists at most.
const MAX_LISTED_KEYS: usize = 40;

/// Why `at` names nothing in `snapshot`, with what is valid where it
/// stopped matching: an object's keys, a list's index range.
fn nothing_at(snapshot: &Value, at: &str) -> String {
    if !at.starts_with('/') {
        return format!(
            "at {at} is not a JSON pointer: start it with /, as in /drafts/0/draft; the top-level keys are {}",
            valid_below(snapshot)
        );
    }
    let mut here = snapshot;
    let mut matched = String::new();
    for raw in at.split('/').skip(1) {
        let token = raw.replace("~1", "/").replace("~0", "~");
        let next = match here {
            Value::Object(map) => map.get(&token),
            Value::Array(items) => token.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        };
        match next {
            Some(value) => {
                here = value;
                matched.push('/');
                matched.push_str(raw);
            }
            None => {
                let place = if matched.is_empty() { "/" } else { &matched };
                // A path that lives under the project (`/library/…`)
                // names where it is.
                let under_project = matched.is_empty() && snapshot["project"].pointer(at).is_some();
                let hint = if under_project {
                    format!(
                        ". It is at /project{at}. To list library components by name, query library.search {{query: \"src:project kind:component\", limit: 200}}."
                    )
                } else {
                    String::new()
                };
                return format!(
                    "nothing at {at} in the snapshot: {raw} is not in {place}, where {}{hint}",
                    valid_below(here)
                );
            }
        }
    }
    format!("nothing at {at} in the snapshot")
}

/// What may follow a pointer that reached `value`.
fn valid_below(value: &Value) -> String {
    match value {
        Value::Object(map) if map.is_empty() => "there is nothing (an empty object)".into(),
        Value::Object(map) => {
            let keys: Vec<&str> = map.keys().map(String::as_str).collect();
            let more = keys.len().saturating_sub(MAX_LISTED_KEYS);
            let listed = keys[..keys.len().min(MAX_LISTED_KEYS)].join(", ");
            if more > 0 {
                format!("the keys are {listed} and {more} more")
            } else {
                format!("the keys are {listed}")
            }
        }
        Value::Array(items) if items.is_empty() => "the list is empty".into(),
        Value::Array(items) => format!(
            "the list takes a position from 0 to {} (not an id)",
            items.len().saturating_sub(1)
        ),
        _ => "the value has nothing inside it".into(),
    }
}

/// Remove `verbose` from a request and from its parameters; true when either
/// asked for the snapshot.
/// Objects one finding names in a summarised report; the rest are counted
/// in `objects_more`.
pub(crate) const MAX_FINDING_OBJECTS: usize = 5;

/// `component.check_drafts` without `detail`: each finding of each report
/// names its first [`MAX_FINDING_OBJECTS`] objects and counts the rest, so
/// a draft with every pin unreviewed answers in a few hundred bytes.
pub(crate) fn summarise_finding_objects(reports: &mut Value) {
    let Some(reports) = reports.as_array_mut() else {
        return;
    };
    for finding in reports
        .iter_mut()
        .filter_map(|r| r.get_mut("findings").and_then(Value::as_array_mut))
        .flatten()
    {
        let Some(objects) = finding.get_mut("objects").and_then(Value::as_array_mut) else {
            continue;
        };
        let more = objects.len().saturating_sub(MAX_FINDING_OBJECTS);
        if more > 0 {
            objects.truncate(MAX_FINDING_OBJECTS);
            finding["objects_more"] = json!(more);
        }
    }
}

pub(crate) fn take_verbose(request: &mut Value) -> bool {
    fn take(value: &mut Value) -> bool {
        value
            .as_object_mut()
            .and_then(|object| object.remove("verbose"))
            .is_some_and(|verbose| verbose == true)
    }
    let outer = take(request);
    let inner = request.get_mut("params").is_some_and(take);
    outer || inner
}

/// The identity of a collection element: its `id` (with `revision` for a
/// published library record), or the `instance` a placement belongs to, or
/// an asset's `hash`.
fn identity(element: &Value) -> Option<Value> {
    let object = element.as_object()?;
    if let Some(id) = object.get("id") {
        return Some(match object.get("revision") {
            Some(revision) => json!({"id": id, "revision": revision}),
            None => id.clone(),
        });
    }
    object
        .get("instance")
        .or_else(|| object.get("hash"))
        .cloned()
}

#[derive(Default)]
struct Changes {
    added: BTreeMap<String, Vec<Value>>,
    changed: BTreeMap<String, Vec<Value>>,
    removed: BTreeMap<String, Vec<Value>>,
    fields: Vec<String>,
}

impl Changes {
    fn diff(&mut self, path: &str, before: &Value, after: &Value) {
        if before == after {
            return;
        }
        if WHOLE.contains(&path) {
            self.fields.push(path.to_owned());
            return;
        }
        match (before, after) {
            (Value::Object(old), Value::Object(new)) => {
                let empty = Value::Array(Vec::new());
                for key in old
                    .keys()
                    .chain(new.keys().filter(|k| !old.contains_key(*k)))
                {
                    // Allocators move with every new object and say nothing.
                    if key.starts_with("next") {
                        continue;
                    }
                    let inner = join(path, key);
                    match (old.get(key), new.get(key)) {
                        (Some(a), Some(b)) => self.diff(&inner, a, b),
                        // An empty collection is left out when serialized.
                        (Some(a @ Value::Array(_)), None) => self.diff(&inner, a, &empty),
                        (None, Some(b @ Value::Array(_))) => self.diff(&inner, &empty, b),
                        _ => self.fields.push(inner),
                    }
                }
            }
            (Value::Array(old), Value::Array(new)) => {
                if !self.diff_collection(path, old, new) {
                    self.fields.push(path.to_owned());
                }
            }
            _ => self.fields.push(path.to_owned()),
        }
    }

    /// Compare two arrays element by element when every element has a unique
    /// identity. False when they do not, or when only the order differs.
    fn diff_collection(&mut self, path: &str, old: &[Value], new: &[Value]) -> bool {
        let keyed = |elements: &[Value]| -> Option<HashMap<String, usize>> {
            let mut map = HashMap::with_capacity(elements.len());
            for (index, element) in elements.iter().enumerate() {
                if map.insert(identity(element)?.to_string(), index).is_some() {
                    return None;
                }
            }
            Some(map)
        };
        let (Some(old_keys), Some(new_keys)) = (keyed(old), keyed(new)) else {
            return false;
        };
        let mut any = false;
        let mut record = |list: &mut BTreeMap<String, Vec<Value>>, element: &Value| {
            if let Some(id) = identity(element) {
                list.entry(path.to_owned()).or_default().push(id);
            }
            any = true;
        };
        for element in new {
            let key = identity(element).map(|id| id.to_string());
            match key.and_then(|key| old_keys.get(&key).copied()) {
                None => record(&mut self.added, element),
                Some(index) => {
                    if old.get(index) != Some(element) {
                        record(&mut self.changed, element);
                    }
                }
            }
        }
        for element in old {
            let key = identity(element).map(|id| id.to_string());
            if !key.is_some_and(|key| new_keys.contains_key(&key)) {
                record(&mut self.removed, element);
            }
        }
        any
    }

    fn merge(&mut self, other: Changes) {
        self.added.extend(other.added);
        self.changed.extend(other.changed);
        self.removed.extend(other.removed);
        self.fields.extend(other.fields);
    }

    /// The reply field, or `None` when nothing changed. At most [`MAX_IDS`]
    /// ids are listed; `omitted` counts the rest.
    fn into_value(self) -> Option<Value> {
        let mut budget = MAX_IDS;
        let mut omitted = 0;
        let mut out = Map::new();
        for (name, lists) in [
            ("added", self.added),
            ("changed", self.changed),
            ("removed", self.removed),
        ] {
            if lists.is_empty() {
                continue;
            }
            let mut listed = Map::new();
            for (path, mut ids) in lists {
                let keep = ids.len().min(budget);
                omitted += ids.len() - keep;
                budget -= keep;
                ids.truncate(keep);
                listed.insert(path, Value::Array(ids));
            }
            out.insert(name.to_owned(), Value::Object(listed));
        }
        if !self.fields.is_empty() {
            out.insert("fields".to_owned(), json!(self.fields));
        }
        if omitted > 0 {
            out.insert("omitted".to_owned(), json!(omitted));
        }
        (!out.is_empty()).then_some(Value::Object(out))
    }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// How many elements each collection of the project holds, by dotted path.
fn count_collections(value: &Value, path: &str, counts: &mut Map<String, Value>) {
    let Some(object) = value.as_object() else {
        return;
    };
    for (key, inner) in object {
        match inner {
            Value::Array(items) if items.iter().all(|i| identity(i).is_some()) => {
                counts.insert(join(path, key), json!(items.len()));
            }
            Value::Object(_) => count_collections(inner, &join(path, key), counts),
            _ => {}
        }
    }
}

/// Which findings appeared and which went: findings compare by code, message
/// and objects, so a repeated finding counts each time it occurs.
fn findings_delta(old: &[&Finding], new: &[&Finding]) -> Value {
    fn key(finding: &Finding) -> (&str, &str, &[sparksmith_core::ObjectRef]) {
        (&finding.code, &finding.message, &finding.objects)
    }
    let mut remaining: HashMap<(&str, &str, &[sparksmith_core::ObjectRef]), (usize, &str)> =
        HashMap::with_capacity(old.len());
    for finding in old.iter().copied() {
        remaining
            .entry(key(finding))
            .or_insert((0, &finding.code))
            .0 += 1;
    }
    let mut appeared = Vec::new();
    for finding in new.iter().copied() {
        match remaining.get_mut(&key(finding)) {
            Some((count, _)) if *count > 0 => *count -= 1,
            _ => appeared.push(finding),
        }
    }
    let mut resolved: BTreeMap<&str, usize> = BTreeMap::new();
    for (count, code) in remaining.values() {
        if *count > 0 {
            *resolved.entry(code).or_default() += count;
        }
    }
    let listed: Vec<Value> = appeared
        .iter()
        .take(MAX_FINDINGS)
        .map(|f| {
            let mut row = json!({"code": f.code, "message": f.message, "objects": f.objects});
            if let Some(stage) = f.blocks {
                row["blocks"] = json!(stage);
            }
            row
        })
        .collect();
    let mut out = json!({"new": listed, "resolved": resolved, "open": new.len()});
    if appeared.len() > MAX_FINDINGS {
        out["omitted"] = json!(appeared.len() - MAX_FINDINGS);
    }
    out
}

/// What changed between two documents, as a compact reply says it:
/// `changes` (the ids added, changed and removed per collection, and the
/// fields), `names` (by the id's JSON text, the name a person reads for each
/// listed id that has one: a designator, a net's or a sheet's name) and
/// `findings` (design findings new and resolved, and how many are open).
/// The chat's turn summary card reads it (`docs/assistant-panel.md` §7).
pub fn difference(
    before: &Document,
    after: &Document,
    settings: &layout::CheckSettings,
) -> Result<Value> {
    let mut changes = Changes::default();
    let mut names = Map::new();
    let mut out = Map::new();
    if !Arc::ptr_eq(&before.project, &after.project) {
        use sparksmith_core::issues::incremental::Checked;
        let graph = |p: &Arc<Project>| {
            let p = p.clone();
            move || Arc::new(layout::CopperGraph::of(&p))
        };
        let old = Checked::full(
            &before.project,
            &before.drafts,
            settings,
            &graph(&before.project),
        )?;
        // The state after is made from the one before: only what the
        // change touched is checked again.
        let new = old.next(
            &after.project,
            &after.drafts,
            settings,
            &graph(&after.project),
        )?;
        let (old_value, new_value) = unshared(&before.project, &after.project)?;
        changes.diff("", &old_value, &new_value);
        name_listed(&old_value, &changes.removed, &mut names);
        name_listed(&new_value, &changes.added, &mut names);
        name_listed(&new_value, &changes.changed, &mut names);
        out.insert(
            "findings".to_owned(),
            findings_delta(&open_findings(old.report()), &open_findings(new.report())),
        );
    }
    if !Arc::ptr_eq(&before.drafts, &after.drafts) {
        let old = json!({"drafts": serde_json::to_value(before.drafts.as_ref())?});
        let new = json!({"drafts": serde_json::to_value(after.drafts.as_ref())?});
        let mut delta = Changes::default();
        delta.diff("drafts", &old["drafts"], &new["drafts"]);
        name_listed(&old, &delta.removed, &mut names);
        name_listed(&new, &delta.added, &mut names);
        name_listed(&new, &delta.changed, &mut names);
        changes.merge(delta);
    }
    if let Some(changes) = changes.into_value() {
        out.insert("changes".to_owned(), changes);
    }
    if !names.is_empty() {
        out.insert("names".to_owned(), Value::Object(names));
    }
    Ok(Value::Object(out))
}

/// The person's name of each id in `lists` (collection path to ids) that
/// `root` holds and that has one.
/// What changed between two documents, without the findings: each object
/// added, changed or removed, by its collection's path and its id, with
/// its designator or name where it has one; and the changed places that
/// are no collection of objects.
pub(crate) struct Touched {
    pub(crate) objects: BTreeMap<(String, String), Option<String>>,
    pub(crate) fields: Vec<String>,
}

/// [`Touched`] from `before` to `after`; only the sections that differ are
/// read.
pub(crate) fn touched(before: &Document, after: &Document) -> Result<Touched> {
    let mut objects = BTreeMap::new();
    let mut fields = Vec::new();
    let mut compare = |path: &str, old: Value, new: Value| {
        let mut changes = Changes::default();
        changes.diff(path, &old, &new);
        let mut names = Map::new();
        let root = |value: Value| {
            let mut map = Map::new();
            map.insert(path.to_owned(), value);
            Value::Object(map)
        };
        let (old, new) = if path.is_empty() {
            (old, new)
        } else {
            (root(old), root(new))
        };
        name_listed(&old, &changes.removed, &mut names);
        name_listed(&new, &changes.added, &mut names);
        name_listed(&new, &changes.changed, &mut names);
        for lists in [&changes.added, &changes.changed, &changes.removed] {
            for (list, ids) in lists {
                for id in ids {
                    let id = id.to_string();
                    let name = names.get(&id).and_then(Value::as_str).map(str::to_owned);
                    objects.insert((list.clone(), id), name);
                }
            }
        }
        fields.extend(changes.fields);
    };
    if !Arc::ptr_eq(&before.project, &after.project) {
        // Only the sections either changed: a shared one (the packed
        // assets, the unchanged tracks) is never serialized (review,
        // rebase cost).
        let (old, new) = unshared(&before.project, &after.project)?;
        compare("", old, new);
    }
    if !Arc::ptr_eq(&before.drafts, &after.drafts) {
        compare(
            "drafts",
            serde_json::to_value(before.drafts.as_ref())?,
            serde_json::to_value(after.drafts.as_ref())?,
        );
    }
    Ok(Touched { objects, fields })
}

fn name_listed(root: &Value, lists: &BTreeMap<String, Vec<Value>>, names: &mut Map<String, Value>) {
    for (path, ids) in lists {
        let Some(Value::Array(items)) = path.split('.').try_fold(root, |value, key| value.get(key))
        else {
            continue;
        };
        for id in ids {
            let name = items
                .iter()
                .find(|item| identity(item).as_ref() == Some(id))
                .and_then(|item| {
                    ["designator", "name"]
                        .iter()
                        .find_map(|key| item.get(*key).and_then(Value::as_str))
                });
            if let Some(name) = name.filter(|n| !n.is_empty()) {
                names.insert(id.to_string(), json!(name));
            }
        }
    }
}

#[cfg(test)]
pub(super) mod tests_support {
    use super::*;

    pub fn changes_of(before: &Value, after: &Value) -> Value {
        let mut changes = Changes::default();
        changes.diff("", before, after);
        changes.into_value().unwrap_or(Value::Null)
    }

    /// The changes a compact reply reports from `before` to `after`.
    pub fn changes_between(before: &Project, after: &Project) -> Result<Value> {
        let (a, b) = unshared(before, after)?;
        Ok(changes_of(&a, &b))
    }

    pub fn delta(old: &[Finding], new: &[Finding]) -> Value {
        findings_delta(
            &old.iter().collect::<Vec<_>>(),
            &new.iter().collect::<Vec<_>>(),
        )
    }

    pub fn name_listed(
        root: &Value,
        lists: &BTreeMap<String, Vec<Value>>,
        names: &mut Map<String, Value>,
    ) {
        super::name_listed(root, lists, names);
    }
}
