//! The review report of a component made from datasheets, and its check
//! (`library.review_check`). Whoever reads the datasheets (an agent over MCP,
//! the in-app assistant, a person) states every value it used with its
//! document, page, table or figure and a confidence, and the open questions
//! a person must settle before publishing. Sparksmith never reads the
//! datasheets itself: it checks that the report is complete and cites what
//! it lists, and holds it against the drafts in the project (their import
//! review state, the source pages of their imported functions and their
//! blocking findings). The check changes nothing and publishes nothing.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt::Write;

/// Bytes of one free-text field.
pub const MAX_TEXT: usize = 4096;
/// Values one report lists.
pub const MAX_VALUES: usize = 1024;
/// Problems one check lists.
pub const MAX_PROBLEMS: usize = 20;
/// Terminals one problem names.
const MAX_NAMED: usize = 8;

#[derive(
    Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Read directly from a dimensioned drawing or a table cell.
    High,
    /// Derived arithmetically from stated values, or read from a drawing
    /// whose dimension line is ambiguous.
    Medium,
    /// Inferred, measured from a drawing's scale, or not stated.
    Low,
}

/// Where in which document a value comes from.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Citation {
    /// The document's short name, as listed in `sources`.
    pub document: String,
    /// 1-based page number of the PDF.
    pub page: u32,
    /// Table, figure or section, e.g. "Table 3-1" or "Figure 10-1".
    #[serde(default)]
    pub location: String,
}

/// One source document.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SourceDocument {
    /// Short name used by citations.
    pub name: String,
    pub title: String,
    /// The document's own revision and date.
    #[serde(default)]
    pub revision: String,
}

/// One value taken from the datasheets.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ExtractedValue {
    /// What it is, e.g. "pad pitch", "pin 45 strapping default".
    pub item: String,
    /// The value with its unit as used, e.g. "0.85 mm".
    pub value: String,
    pub source: Citation,
    pub confidence: Confidence,
    #[serde(default)]
    pub note: String,
}

/// The review report.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReviewReport {
    /// The part, e.g. its manufacturer part name.
    pub part: String,
    /// What was built and what a reviewer should check first.
    pub summary: String,
    pub sources: Vec<SourceDocument>,
    pub values: Vec<ExtractedValue>,
    /// Questions the datasheets leave open, each to settle before publish.
    #[serde(default)]
    pub open_questions: Vec<String>,
}

impl ReviewReport {
    /// Bounds every report must meet: text sizes, listed sources, and a
    /// listed document and a page for every value. `library.review_check`
    /// refuses a report with any of these.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let long = |s: &str| s.len() > MAX_TEXT;
        if self.part.trim().is_empty() || long(&self.part) || long(&self.summary) {
            out.push("part and summary must be non-empty text of at most 4 KiB".into());
        }
        if self.values.len() > MAX_VALUES {
            out.push(format!("at most {MAX_VALUES} values"));
        }
        if self.sources.is_empty() {
            out.push("list the source documents".into());
        }
        for (k, v) in self.values.iter().enumerate() {
            if !self.sources.iter().any(|s| s.name == v.source.document) {
                out.push(format!(
                    "value {} ({}) cites document {:?}, which sources does not list",
                    k + 1,
                    v.item,
                    v.source.document
                ));
            }
            if v.source.page == 0 {
                out.push(format!("value {} ({}) needs a page", k + 1, v.item));
            }
            if v.item.trim().is_empty() || v.value.trim().is_empty() {
                out.push(format!(
                    "value {} needs what it is in item and the value as used in value",
                    k + 1
                ));
            }
        }
        out.truncate(MAX_PROBLEMS);
        out
    }

    /// [`Self::problems`] and what `library.review_check` adds on the report
    /// alone: at least one value, distinct source names, a table, figure or
    /// section for every value, and an open question whenever a value is
    /// low confidence.
    pub fn citation_problems(&self) -> Vec<String> {
        let mut out = self.problems();
        if self.values.is_empty() {
            out.push("list every value you used, each with its citation".into());
        }
        let mut names: Vec<&str> = self.sources.iter().map(|s| s.name.as_str()).collect();
        names.sort_unstable();
        if names.windows(2).any(|w| w[0] == w[1]) {
            out.push("source names must be distinct".into());
        }
        for (k, v) in self.values.iter().enumerate() {
            if v.source.location.trim().is_empty() {
                out.push(format!(
                    "value {} ({}) needs its table, figure or section in location",
                    k + 1,
                    v.item
                ));
            }
        }
        let low = self
            .values
            .iter()
            .filter(|v| v.confidence == Confidence::Low)
            .count();
        if low > 0 && self.open_questions.is_empty() {
            let values = if low == 1 { "value is" } else { "values are" };
            out.push(format!(
                "{low} {values} low confidence: list what a reviewer must settle in open_questions"
            ));
        }
        out.truncate(MAX_PROBLEMS);
        out
    }
}

/// The report's own sections as Markdown: summary, sources, open questions
/// (as a checklist) and the values, least confident first.
pub fn report_markdown(r: &ReviewReport) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "## Summary\n\n{}\n", r.summary);
    let _ = writeln!(s, "## Sources\n");
    for d in &r.sources {
        let _ = writeln!(s, "- **{}**: {} {}", d.name, d.title, d.revision);
    }
    let _ = writeln!(s, "\n## Open questions\n");
    if r.open_questions.is_empty() {
        let _ = writeln!(s, "None listed.");
    }
    for q in &r.open_questions {
        let _ = writeln!(s, "- [ ] {q}");
    }
    let _ = writeln!(s, "\n## Extracted values\n");
    let _ = writeln!(s, "| Item | Value | Source | Confidence | Note |");
    let _ = writeln!(s, "|---|---|---|---|---|");
    let mut values: Vec<&ExtractedValue> = r.values.iter().collect();
    // Least confident first: they need the reviewer most.
    values.sort_by_key(|v| std::cmp::Reverse(v.confidence));
    for v in values {
        let _ = writeln!(
            s,
            "| {} | {} | {} p.{} {} | {:?} | {} |",
            cell(&v.item),
            cell(&v.value),
            cell(&v.source.document),
            v.source.page,
            cell(&v.source.location),
            v.confidence,
            cell(&v.note)
        );
    }
    s
}

pub fn cell(t: &str) -> String {
    t.replace('|', "\\|").replace('\n', " ")
}

/// `library.review_check` parameters.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewCheckParams {
    pub report: ReviewReport,
    /// The placed part, by designator, whose 3D body the report is of
    /// (`library.make_body`): the report is held against that body instead
    /// of the drafts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The component lineage the report is of: its draft and the drafts of
    /// its symbol, binding, package and footprint, or, with no draft open,
    /// its latest published revision with the pins it was published
    /// unreviewed. Without it, the component draft named as the report's
    /// `part`, else every open draft.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<sparksmith_core::ids::DefinitionId>,
}

/// One draft as the check sees it.
struct DraftState {
    id: Value,
    kind: String,
    name: String,
    blocking: Vec<String>,
    warnings: Vec<String>,
    /// Imported terminals still unreviewed and flagged uncertain.
    unreviewed: usize,
    uncertain: usize,
    /// Terminal names with an imported function that has no source page.
    unsourced: Vec<String>,
    /// The pages (with their table) the imported functions cite.
    pages: std::collections::BTreeMap<u64, String>,
}

fn drafts(snapshot: &Value) -> Vec<DraftState> {
    snapshot["drafts"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|d| {
            let definition = &d["draft"]["definition"];
            let body = &definition["body"];
            let mut blocking = Vec::new();
            let mut warnings = Vec::new();
            for f in d["check"]["result"]["findings"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let code = f["code"].as_str().unwrap_or("").to_owned();
                let list = if f["blocks"].is_null() {
                    &mut warnings
                } else {
                    &mut blocking
                };
                if !list.contains(&code) {
                    list.push(code);
                }
            }
            let review = &body["review"];
            let count = |key: &str| review[key].as_array().map_or(0, Vec::len);
            // Only an import's terminals carry sources; a draft with no
            // review state pending was authored or already confirmed.
            let mut unsourced = Vec::new();
            let mut pages = std::collections::BTreeMap::new();
            if !review.is_null() {
                for t in body["terminals"].as_array().into_iter().flatten() {
                    let functions = t["class"]["electrical"]["functions"].as_array();
                    let missing = functions
                        .into_iter()
                        .flatten()
                        .any(|f| f["source"]["page"].as_u64().is_none_or(|p| p == 0));
                    if missing {
                        unsourced.push(t["name"].as_str().unwrap_or("?").to_owned());
                    }
                    for source in functions.into_iter().flatten().map(|f| &f["source"]) {
                        if let Some(page) = source["page"].as_u64().filter(|p| *p > 0) {
                            let table = source["table"].as_str().unwrap_or_default();
                            pages.entry(page).or_insert_with(|| table.to_owned());
                        }
                    }
                }
            }
            DraftState {
                id: d["draft"]["id"].clone(),
                kind: definition["kind"].as_str().unwrap_or("").to_owned(),
                name: d["draft"]["meta"]["name"].as_str().unwrap_or("").to_owned(),
                blocking,
                warnings,
                unreviewed: count("unreviewed"),
                uncertain: count("uncertain"),
                unsourced,
                pages,
            }
        })
        .collect()
}

/// `library.review_check`: the report's citation problems, then what the
/// drafts in `snapshot` show against it. `accepted` means the report may go
/// to a person for review; it never means the part is verified or
/// publishable.
pub fn check(snapshot: &Value, report: &ReviewReport) -> Value {
    let mut problems = report.citation_problems();
    let drafts = drafts(snapshot);
    let components: Vec<&DraftState> = drafts.iter().filter(|d| d.kind == "component").collect();
    if components.is_empty() {
        problems.push(
            "there is no component draft to review: import the pinout or create the drafts first"
                .into(),
        );
    }
    let uncertain: usize = drafts.iter().map(|d| d.uncertain).sum();
    if uncertain > 0 && report.open_questions.is_empty() {
        let pins = if uncertain == 1 { "pin is" } else { "pins are" };
        problems.push(format!(
            "{uncertain} imported {pins} flagged uncertain: say in open_questions what a reviewer must check on each"
        ));
    }
    for d in &drafts {
        if !d.unsourced.is_empty() {
            let mut named: Vec<&str> = d.unsourced.iter().map(String::as_str).collect();
            named.truncate(MAX_NAMED);
            let pins = if d.unsourced.len() == 1 {
                "pin has"
            } else {
                "pins have"
            };
            problems.push(format!(
                "{} {} ‘{}’: {} imported {pins} a function without a source page ({}{}); give each pinout row its page and table and re-import",
                d.kind,
                d.id,
                d.name,
                d.unsourced.len(),
                named.join(", "),
                if d.unsourced.len() > MAX_NAMED { ", …" } else { "" }
            ));
        }
    }
    // The pages the imported pinout cites are pages or tables the report
    // cites, so a reviewer finds each pin's table among the report's
    // values. Named for the reviewer, not a refusal: the pin rows carry
    // their own sources.
    let cited: std::collections::BTreeSet<u64> = report
        .values
        .iter()
        .map(|v| u64::from(v.source.page))
        .collect();
    let tables: std::collections::BTreeSet<String> = report
        .values
        .iter()
        .map(|v| v.source.location.trim().to_lowercase())
        .filter(|l| !l.is_empty())
        .collect();
    let uncited: std::collections::BTreeMap<u64, &str> = drafts
        .iter()
        .flat_map(|d| d.pages.iter())
        .filter(|(page, table)| {
            !cited.contains(page) && !tables.contains(&table.trim().to_lowercase())
        })
        .map(|(page, table)| (*page, table.as_str()))
        .collect();
    let mut uncited: Vec<String> = uncited
        .into_iter()
        .map(|(page, table)| {
            if table.is_empty() {
                format!("page {page}")
            } else {
                format!("page {page} ({table})")
            }
        })
        .collect();
    let mut uncited_note = None;
    if !uncited.is_empty() {
        let more = uncited.len() > MAX_NAMED;
        uncited.truncate(MAX_NAMED);
        uncited_note = Some(format!(
            "the imported pinout cites {}{}, which no value in the report cites: list each pin table among the values with its document, page and table",
            uncited.join(", "),
            if more { ", …" } else { "" }
        ));
    }
    problems.truncate(MAX_PROBLEMS);
    let unreviewed: usize = drafts.iter().map(|d| d.unreviewed).sum();
    let mut md = String::new();
    let _ = writeln!(md, "# Datasheet import review: {}\n", report.part);
    if snapshot["published"] == true {
        let pins: usize = drafts.iter().map(|d| d.unreviewed).sum();
        let _ = writeln!(
            md,
            "Status: **published**, with {pins} imported pins nobody has reviewed yet. A person checks each against the datasheet in the Pins tab.\n"
        );
    } else {
        let _ = writeln!(
            md,
            "Status: **draft, not published**. Nothing enters a library until a person reviews the rows below and publishes.\n"
        );
    }
    md.push_str(&report_markdown(report));
    let _ = writeln!(md, "\n## Drafts\n");
    for d in &drafts {
        let mut codes: Vec<String> = d.blocking.iter().map(|c| format!("{c}!")).collect();
        codes.extend(d.warnings.iter().cloned());
        let _ = writeln!(
            md,
            "- {} {} ‘{}’: {}",
            d.kind,
            d.id,
            d.name,
            codes.join(", ")
        );
    }
    let _ = writeln!(
        md,
        "\nImported pins: {unreviewed} unreviewed, {uncertain} uncertain. A person checks each against the datasheet in the Pins tab before publishing."
    );
    json!({
        "accepted": problems.is_empty(),
        "problems": problems,
        "drafts": drafts.iter().map(|d| json!({
            "id": d.id,
            "kind": d.kind,
            "name": d.name,
            "blocking": d.blocking,
            "warnings": d.warnings,
        })).collect::<Vec<_>>(),
        "pins": {"unreviewed": unreviewed, "uncertain": uncertain},
        "uncited": uncited_note,
        "markdown": md,
        "next": "Show the person the problems (fix them and check again) or, when accepted, the markdown: they review every unreviewed pin in the Pins tab and publish. Never publish for them.",
    })
}

/// The body a review report is of, as the check sees it: the placed
/// part, its package revision and model, the parts that share it, and
/// whether its provenance records an AI recreation.
pub struct BodyState {
    pub part: String,
    pub package: String,
    pub model: String,
    pub parts: Vec<String>,
    pub recreated: bool,
}

/// `library.review_check` with `body`: the report's citation problems and
/// whether the part's body is the recreation the report describes. Like
/// the drafts' check it changes nothing and never means the body is
/// verified.
pub fn check_body(body: &BodyState, report: &ReviewReport) -> Value {
    let mut problems = report.citation_problems();
    if !body.recreated {
        problems.push(format!(
            "{}'s body is not a recreation from datasheets yet: make it with library.make_body first",
            body.part
        ));
    }
    problems.truncate(MAX_PROBLEMS);
    let mut md = String::new();
    let _ = writeln!(md, "# 3D body review: {}\n", report.part);
    let _ = writeln!(
        md,
        "Status: **recreated by AI, not reviewed**. Compare the body with the datasheet's drawing and photos before relying on it.\n"
    );
    md.push_str(&report_markdown(report));
    let _ = writeln!(md, "\n## Body\n");
    let _ = writeln!(md, "- Part {}: package {}", body.part, body.package);
    let _ = writeln!(md, "- Model: {}", body.model);
    let _ = writeln!(md, "- Parts that use it: {}", body.parts.join(", "));
    json!({
        "accepted": problems.is_empty(),
        "problems": problems,
        "markdown": md,
        "next": "Show the person the problems (fix them and check again) or, when accepted, the markdown: they compare the body with the drawing and photos.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn report() -> ReviewReport {
        ReviewReport {
            part: "EX-MOD-1".into(),
            summary: "A module.".into(),
            sources: vec![SourceDocument {
                name: "DS".into(),
                title: "EX-MOD-1 datasheet".into(),
                revision: "v1".into(),
            }],
            values: vec![ExtractedValue {
                item: "pitch".into(),
                value: "1.27 mm".into(),
                source: Citation {
                    document: "DS".into(),
                    page: 12,
                    location: "Figure 3".into(),
                },
                confidence: Confidence::High,
                note: String::new(),
            }],
            open_questions: vec!["Is the land pattern the pad?".into()],
        }
    }

    #[test]
    fn a_report_cites_listed_documents_with_pages() {
        let mut r = report();
        assert!(r.problems().is_empty());
        r.values[0].source.document = "Other".into();
        r.values[0].source.page = 0;
        assert_eq!(r.problems().len(), 2);
    }

    #[test]
    fn citations_need_a_location_and_low_confidence_needs_a_question() {
        let mut r = report();
        assert!(r.citation_problems().is_empty());
        r.values[0].source.location = " ".into();
        r.values[0].confidence = Confidence::Low;
        r.open_questions.clear();
        r.sources.push(r.sources[0].clone());
        let problems = r.citation_problems();
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems.iter().any(|p| p.contains("location")));
        assert!(
            problems
                .iter()
                .any(|p| p.starts_with("1 value is low confidence:")),
            "one value reads in the singular: {problems:?}"
        );
        assert!(problems.iter().any(|p| p.contains("distinct")));
        r.values.clear();
        assert!(
            r.citation_problems()
                .iter()
                .any(|p| p.contains("every value"))
        );
    }

    #[test]
    fn a_report_without_drafts_is_not_accepted() {
        let checked = check(&json!({"drafts": []}), &report());
        assert_eq!(checked["accepted"], false);
        assert!(
            checked["markdown"]
                .as_str()
                .is_some_and(|m| m.contains("| pitch | 1.27 mm | DS p.12 Figure 3 | High |"))
        );
    }

    /// The pages an imported pinout cites that the report does not are named,
    /// and every value says what it is and gives its value.
    #[test]
    fn the_report_cites_the_pages_the_pinout_cites() {
        let snapshot = |page: u64| {
            json!({"drafts": [{"draft": {"id": 1, "meta": {"name": "EX-MOD-1"}, "definition": {
                "kind": "component", "body": {"review": {"unreviewed": [1], "uncertain": []},
                "terminals": [{"name": "IO1", "class": {"electrical": {"functions": [
                    {"label": "gpio", "source": {"page": page, "table": "Table 2"}}]}}}]}}},
                "check": {"result": {"findings": []}}}]})
        };
        let cited = check(&snapshot(12), &report());
        assert!(
            cited["uncited"].is_null(),
            "{cited}"
        );
        let uncited = check(&snapshot(7), &report());
        assert!(
            uncited["uncited"]
                .as_str()
                .is_some_and(|n| n.contains("cites page 7 (Table 2), which no value in the report cites")),
            "{uncited}"
        );
        let mut r = report();
        r.values[0].value = " ".into();
        assert!(r.problems().iter().any(|p| p.contains("value as used")));
    }
}
