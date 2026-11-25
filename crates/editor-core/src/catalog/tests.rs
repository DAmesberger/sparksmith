use super::*;
use crate::format::{EditorFile, FILE_VERSION};
use crate::{Engine, Query, QueryResult};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

/// A validated engine over a synthetic project of `parts` parts.
fn engine(parts: usize) -> Result<Engine> {
    Engine::from_file(EditorFile {
        version: FILE_VERSION,
        project: synthetic::project(parts, &crate::NewBoard::fixture())?,
        drafts: Drafts::default(),
    })
}

fn index(parts: usize) -> Result<Index> {
    let engine = engine(parts)?;
    Index::of_project(engine.project(), engine.drafts())
}

fn names(report: &SearchReport) -> Vec<&str> {
    report
        .results
        .iter()
        .map(|h| h.entry.name.as_str())
        .collect()
}

fn search(index: &Index, text: &str) -> std::result::Result<SearchReport, QueryError> {
    let query = index.parse(text)?;
    Ok(index.search(
        &query,
        &CatalogContext::default(),
        SearchOptions {
            limit: MAX_RESULTS,
            offset: 0,
            include_drafts: false,
        },
    ))
}

fn field_id(index: &Index, name: &str) -> Result<DefinitionId> {
    index
        .vocabulary()
        .fields
        .iter()
        .find(|f| f.name == name)
        .map(|f| f.field)
        .ok_or_else(|| Error::NotFound(name.into()))
}

/// The SI value a term reads for one field.
fn reading(term: &Term, field: DefinitionId) -> Option<f64> {
    let tests = match term {
        Term::Value { readings, .. } => readings,
        Term::Field { tests, .. } => tests,
        _ => return None,
    };
    tests
        .iter()
        .find(|t| t.field == field)
        .and_then(|t| match t.test {
            Test::Compare { value, .. } => Some(value),
            _ => None,
        })
}

/// Each term's source range, quotes included, in the order the parser
/// reads the terms, so a filter bar can remove one chip's term.
#[test]
fn term_spans_cover_each_term_and_its_quotes() -> TestResult {
    let text = "res>=10k  \"top side\" 0603 µF";
    let spans = term_spans(text)?;
    let shown: Vec<&str> = spans.iter().filter_map(|r| text.get(r.clone())).collect();
    assert_eq!(shown, vec!["res>=10k", "\"top side\"", "0603", "µF"]);
    assert!(term_spans("\"open").is_err());
    assert!(term_spans("  ")?.is_empty());
    Ok(())
}

#[test]
fn synthetic_project_is_a_valid_document() -> TestResult {
    let engine = engine(24)?;
    assert_eq!(engine.project().library.components.len(), 24);
    let index = Index::of_project(engine.project(), engine.drafts())?;
    assert_eq!(index.entries().len(), 24);
    let names: Vec<&str> = index
        .vocabulary()
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["Resistance", "Capacitance", "Max voltage", "Tolerance"]
    );
    Ok(())
}

#[test]
fn bare_values_read_rkm_and_si_text_through_the_fields_own_units() -> TestResult {
    let index = index(24)?;
    let resistance = field_id(&index, "Resistance")?;
    let capacitance = field_id(&index, "Capacitance")?;
    let one = |text: &str| -> std::result::Result<Term, Box<dyn std::error::Error>> {
        let mut query = index.parse(text)?;
        Ok(query.terms.pop().ok_or("no term")?)
    };
    let close = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() <= 1e-12 * b.abs());
    for (text, expected) in [
        ("4k7", 4700.0),
        ("4.7k", 4700.0),
        ("4K7", 4700.0),
        ("1R0", 1.0),
        ("2M2", 2.2e6),
        ("0R1", 0.1),
        ("1R0Ω", 1.0),
        ("-4k7", -4700.0),
        ("+4K7Ω", 4700.0),
    ] {
        assert!(close(reading(&one(text)?, resistance), expected), "{text}");
    }
    for (text, expected) in [("100n", 1e-7), ("1u0", 1e-6), ("4n7", 4.7e-9)] {
        assert!(close(reading(&one(text)?, capacitance), expected), "{text}");
    }
    assert_eq!(reading(&one("4k7k")?, resistance), None);
    assert!(index.parse("res:4k7k").is_err());
    assert!(close(reading(&one("4.7k")?, resistance), 4700.0));
    assert!(close(reading(&one("R47")?, resistance), 0.47));
    assert!(close(reading(&one("4R7")?, resistance), 4.7));
    // R is not a capacitance letter, so R47 reads only as a resistance.
    assert_eq!(reading(&one("R47")?, capacitance), None);
    assert!(close(reading(&one("4n7")?, capacitance), 4.7e-9));
    assert!(close(reading(&one("100nF")?, capacitance), 100e-9));
    // Plain numbers and names stay text.
    assert_eq!(
        one("0603")?,
        Term::Text {
            text: "0603".into()
        }
    );
    assert_eq!(
        one("EX-R0603")?,
        Term::Text {
            text: "ex-r0603".into()
        }
    );

    // The invented name happens to contain "4k7"; "4.7k" matches the value
    // only.
    let found = search(&index, "4k7")?;
    assert_eq!(names(&found), ["EX-R0402-4K70-J"]);
    assert_eq!(found.results[0].tier, MatchTier::Substring);
    // The chip shows only the readings that found something.
    let found = search(&index, "10k")?;
    assert_eq!(found.chips, ["Resistance = 10 kΩ or 10k"]);
    let found = search(&index, "4.7k")?;
    assert_eq!(names(&found), ["EX-R0402-4K70-J"]);
    assert_eq!(found.results[0].tier, MatchTier::Field);
    let found = search(&index, "R47")?;
    assert_eq!(names(&found), ["EX-R0603-R470-F"]);
    Ok(())
}

#[test]
fn a_unit_key_refuses_a_bare_multiplier_with_the_dimension_it_lacks() -> TestResult {
    let index = index(24)?;
    let error = index.parse("V>=10k").err().ok_or("V>=10k parsed")?;
    assert_eq!(error.to_string(), "V>=10k: 10 k has no unit of voltage");
    let error = index.parse("V>=10kΩ").err().ok_or("V>=10kΩ parsed")?;
    assert_eq!(error.term, "V>=10kΩ");
    assert!(
        error.message.contains("‘kΩ’ is not a unit of voltage"),
        "{error}"
    );
    // A plain number is in the key's unit; a stated unit of the kind is fine.
    let voltage = field_id(&index, "Max voltage")?;
    for text in ["V>=25", "V>=25V", "V>=0.025kV", "kV>=0.025"] {
        let query = index.parse(text)?;
        assert_eq!(reading(&query.terms[0], voltage), Some(25.0), "{text}");
    }
    let found = search(&index, "V>=25")?;
    assert!(names(&found).iter().all(|n| n.starts_with("EX-C")));
    assert_eq!(found.total, 4, "{:?}", names(&found));
    // The engine reports the same message as a typed error.
    let engine = engine(24)?;
    let error = engine
        .query(Query::Search {
            query: "V>=10k".into(),
            limit: None,
            include_drafts: false,
        })
        .err()
        .ok_or("engine accepted V>=10k")?;
    assert_eq!(error.code(), "invalid_input");
    assert_eq!(error.to_string(), "V>=10k: 10 k has no unit of voltage");
    Ok(())
}

#[test]
fn field_keys_resolve_from_data_and_report_unknown_or_ambiguous_keys() -> TestResult {
    let index = index(24)?;
    let resistance = field_id(&index, "Resistance")?;
    // Exact name, unique prefix and a unit alias all reach the field.
    for text in [
        "resistance>=10k",
        "res>=10k",
        "R>=10kΩ",
        "Ω>=10000",
        "kΩ>=10",
        "ohm>=10kohm",
    ] {
        let query = index.parse(text)?;
        assert_eq!(reading(&query.terms[0], resistance), Some(10e3), "{text}");
    }
    // A unit key refuses the bare multiplier, as for volts; `R` is a unit
    // alias of this data, so it is a unit key even though it also begins
    // the field name.
    for text in ["Ω>=10k", "R>=10k"] {
        let error = index.parse(text).err().ok_or("bare multiplier parsed")?;
        assert_eq!(error.message, "10 k has no unit of resistance", "{text}");
    }
    let error = index.parse("wattage>=1").err().ok_or("unknown key")?;
    assert!(
        error
            .message
            .starts_with("‘wattage’ is not a field or filter"),
        "{error}"
    );
    assert!(error.message.contains("Resistance"));
    assert!(error.message.contains("filters: pkg, has"), "{error}");
    // `kind:component` lists component entries; it is not the source filter.
    assert_eq!(index.parse("kind:component")?.terms, [Term::Components]);
    // `ma` is a prefix of one field only; `t` of none; `c`/`ca` only one.
    assert!(index.parse("ma>=10").is_ok());
    assert!(
        index
            .parse("tag:audio")
            .is_err_and(|e| e.message.contains("reserved"))
    );
    assert!(
        index
            .parse("R>10k..22k")
            .is_err_and(|e| e.message.contains("‘:’"))
    );
    assert!(
        index
            .parse("R>=")
            .is_err_and(|e| e.message.contains("needs a value"))
    );
    assert!(index.parse("\"unclosed").is_err());
    let long = "x ".repeat(MAX_TERMS + 1);
    assert!(
        index
            .parse(&long)
            .is_err_and(|e| e.message.contains("terms"))
    );
    Ok(())
}

/// The synthetic vocabulary with `Max voltage` renamed to `Voltage` and a
/// second field whose name also begins with `R`.
fn renamed_vocabulary(index: &Index) -> Result<Vocabulary> {
    let mut vocabulary = index.vocabulary().clone();
    let mut extra = vocabulary
        .fields
        .iter()
        .find(|f| f.name == "Max voltage")
        .cloned()
        .ok_or_else(|| Error::NotFound("Max voltage".into()))?;
    for field in &mut vocabulary.fields {
        if field.name == "Max voltage" {
            field.name = "Voltage".into();
        }
    }
    extra.name = "Rated power".into();
    extra.field = DefinitionId(u64::MAX);
    vocabulary.fields.push(extra);
    Ok(vocabulary)
}

#[test]
fn unit_keys_do_not_depend_on_which_other_fields_exist() -> TestResult {
    let index = index(24)?;
    let resistance = field_id(&index, "Resistance")?;
    let voltage = field_id(&index, "Max voltage")?;
    let renamed = renamed_vocabulary(&index)?;
    for vocabulary in [index.vocabulary(), &renamed] {
        // `V` and `R` are unit keys whatever the field names are: a field
        // named `Voltage` does not make `V>=10k` mean 10 kV, and `Rated
        // power` does not change how `R` reads.
        let error = parse_query("V>=10k", vocabulary)
            .err()
            .ok_or("V>=10k parsed")?;
        assert_eq!(error.to_string(), "V>=10k: 10 k has no unit of voltage");
        let error = parse_query("R>=10k", vocabulary)
            .err()
            .ok_or("R>=10k parsed")?;
        assert_eq!(error.to_string(), "R>=10k: 10 k has no unit of resistance");
        let query = parse_query("R>=10kΩ", vocabulary)?;
        assert_eq!(reading(&query.terms[0], resistance), Some(10e3));
        let query = parse_query("V>=25", vocabulary)?;
        assert_eq!(reading(&query.terms[0], voltage), Some(25.0));
        // A field-name key reads a bare multiplier in the default unit.
        let query = parse_query("res>=10k", vocabulary)?;
        assert_eq!(reading(&query.terms[0], resistance), Some(10e3));
    }
    // In the renamed data `volt` names the field and `r` is ambiguous.
    let query = parse_query("volt>=10k", &renamed)?;
    assert_eq!(reading(&query.terms[0], voltage), Some(10e3));
    let error = parse_query("r>=1", &renamed).err().ok_or("r>=1 parsed")?;
    assert!(
        error
            .message
            .contains("matches several fields: Rated power, Resistance"),
        "{error}"
    );
    Ok(())
}

#[test]
fn explicit_unit_keys_scale_directly_without_a_default_unit_intermediate() -> TestResult {
    let index = index(6)?;
    let resistance = field_id(&index, "Resistance")?;
    let mut vocabulary = index.vocabulary().clone();
    let kind = vocabulary
        .fields
        .iter_mut()
        .find(|f| f.field == resistance)
        .and_then(|f| f.kind.as_mut())
        .ok_or("no resistance quantity kind")?;
    kind.quantity.units.units = [("X", 1e20), ("Y", 1.0)]
        .into_iter()
        .map(|(symbol, scale)| crate::quantity::UnitOfMeasure {
            symbol: symbol.into(),
            aliases: vec![],
            scale,
            si_prefixes: true,
        })
        .collect();
    kind.quantity.default_unit = "X".into();
    kind.quantity.units.check()?;
    // 1e90 Y is in range; evaluating it first as X would make 1e110 SI.
    for text in ["Y>=1e90", "Y>=1e90Y", "kY>=1e87", "Y>=1Y0"] {
        let expected = if text == "Y>=1Y0" { 1.0 } else { 1e90 };
        let query = parse_query(text, &vocabulary)?;
        assert_eq!(
            reading(&query.terms[0], resistance),
            Some(expected),
            "{text}"
        );
    }
    for text in ["X>=1e90", "Y>=1e90X", "Y>=10k"] {
        assert!(parse_query(text, &vocabulary).is_err(), "{text}");
    }
    Ok(())
}

#[test]
fn ranges_and_tolerances_mean_meets_not_equals() -> TestResult {
    let index = index(24)?;
    let found = search(&index, "res:4k7..22k")?;
    let mut got = names(&found);
    got.sort_unstable();
    assert_eq!(
        got,
        ["EX-R0402-4K70-J", "EX-R0603-10K0-F", "EX-R0603-22K0-D"]
    );
    // ±0.5 % and ±1 % meet "tol ≤ 1 %"; ±5 % does not.
    // Eight resistors cycle ±1 %, ±5 %, ±0.5 %: three at 1 %, two at 0.5 %.
    let found = search(&index, "tol<=1%")?;
    assert_eq!(found.total, 5);
    for hit in &found.results {
        let tolerance = hit
            .entry
            .fields
            .iter()
            .find(|f| f.label == "Tolerance")
            .ok_or("no tolerance")?;
        assert!(
            matches!(tolerance.value, FieldValue::Tolerance { plus, .. } if plus.to_f64() <= 0.01)
        );
    }
    let tight = search(&index, "tol<1%")?;
    assert_eq!(tight.total, 2);
    assert!(search(&index, "tol:1..2").is_err());
    // Predicates and filters combine with text.
    let found = search(&index, "res>=10k pkg:0603")?;
    let mut got = names(&found);
    got.sort_unstable();
    // 100 kΩ is the only ≥ 10 kΩ part in 0402.
    assert_eq!(
        got,
        ["EX-R0603-10K0-F", "EX-R0603-1M00-J", "EX-R0603-22K0-D"]
    );
    assert_eq!(search(&index, "has:footprint")?.total, 24);
    assert_eq!(search(&index, "missing:mpn")?.total, 24);
    assert_eq!(search(&index, "has:mpn")?.total, 0);
    assert_eq!(search(&index, "src:project hdr")?.total, 8);
    Ok(())
}

#[test]
fn ranking_is_deterministic_and_follows_the_tiers() -> TestResult {
    let engine = engine(24)?;
    let project = engine.project();
    let mut entries = entries(project, engine.drafts());
    let vocabulary = Vocabulary::of(&project.library);
    let forward = Index::new(entries.clone(), vocabulary.clone())?;
    entries.reverse();
    entries.rotate_left(7);
    let shuffled = Index::new(entries, vocabulary)?;
    for text in [
        "ex",
        "ex-r",
        "10k",
        "0603",
        "res>=1k",
        "hdr",
        "capacitor",
        "",
    ] {
        let a = search(&forward, text)?;
        let b = search(&shuffled, text)?;
        assert_eq!(a, b, "{text}");
    }
    // Exact name, then name prefix, then a whole word, then a substring.
    let found = search(&forward, "ex-hdr-1x2-001")?;
    assert_eq!(found.results[0].tier, MatchTier::Exact);
    let found = search(&forward, "EX-HDR")?;
    assert!(found.results.iter().all(|h| h.tier == MatchTier::Prefix));
    assert_eq!(
        names(&found),
        [
            "EX-HDR-1X2-001",
            "EX-HDR-1X2-002",
            "EX-HDR-1X2-003",
            "EX-HDR-1X2-004",
            "EX-HDR-1X2-005",
            "EX-HDR-1X2-006",
            "EX-HDR-1X2-007",
            "EX-HDR-1X2-008"
        ]
    );
    let found = search(&forward, "hdr")?;
    assert!(found.results.iter().all(|h| h.tier == MatchTier::Word));
    let found = search(&forward, "ceramic")?;
    assert!(found.results.iter().all(|h| h.tier == MatchTier::Substring));
    // A name match outranks a field-only match of the same text.
    let found = search(&forward, "10k")?;
    assert_eq!(found.results[0].entry.name, "EX-R0603-10K0-F");
    assert_eq!(found.results[0].tier, MatchTier::Substring);
    // Boosts: used in the project, then favourite, then recent.
    let header = |n: usize| found_lineage(&forward, &format!("EX-HDR-1X2-{n:03}"));
    let context = CatalogContext::of_project(&[header(5)?], &[header(7)?, header(6)?]);
    let query = forward.parse("EX-HDR")?;
    let ranked = forward.search(&query, &context, SearchOptions::default());
    assert_eq!(
        names(&ranked)[..4],
        [
            "EX-HDR-1X2-005",
            "EX-HDR-1X2-007",
            "EX-HDR-1X2-006",
            "EX-HDR-1X2-001"
        ]
    );
    assert!(ranked.results[0].favourite);
    // The limit keeps the same leading order.
    let limited = forward.search(
        &query,
        &context,
        SearchOptions {
            limit: 2,
            offset: 0,
            include_drafts: false,
        },
    );
    assert_eq!(names(&limited), names(&ranked)[..2]);
    assert_eq!(limited.total, 8);
    Ok(())
}

fn found_lineage(index: &Index, name: &str) -> Result<DefinitionId> {
    index
        .entries()
        .iter()
        .find(|e| e.name == name)
        .map(|e| e.lineage)
        .ok_or_else(|| Error::NotFound(name.into()))
}

#[test]
fn facets_come_from_the_fields_the_results_carry() -> TestResult {
    let index = index(24)?;
    let all = index.facets(&index.parse("")?, false);
    assert_eq!(all.total, 24);
    let labels: Vec<&str> = all.fields.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(
        labels,
        ["Capacitance", "Max voltage", "Resistance", "Tolerance"]
    );
    let resistance = &all.fields[2];
    assert_eq!(resistance.entries, 8);
    assert_eq!(resistance.kind.as_deref(), Some("Resistance"));
    assert_eq!(resistance.min.as_deref(), Some("470 mΩ"));
    assert_eq!(resistance.max.as_deref(), Some("1 MΩ"));
    assert_eq!(resistance.values.len(), 8);
    // Equal counts list in text order.
    let packages: Vec<(&str, usize)> = all
        .packages
        .iter()
        .map(|p| (p.text.as_str(), p.count))
        .collect();
    assert_eq!(packages, [("0402", 8), ("0603", 8), ("TH 2.54", 8)]);
    assert_eq!(all.badges.symbol, 24);
    assert_eq!(all.badges.mpn, 0);
    // Headers carry no fields, so a header query offers no field facet.
    let headers = index.facets(&index.parse("hdr")?, false);
    assert_eq!(headers.total, 8);
    assert!(headers.fields.is_empty());
    assert_eq!(headers.packages[0].text, "TH 2.54");
    let resistors = index.facets(&index.parse("R>=1")?, false);
    let labels: Vec<&str> = resistors.fields.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(labels, ["Resistance", "Tolerance"]);
    Ok(())
}

#[test]
fn draft_only_lineages_are_counted_as_hidden_unless_asked_for() -> TestResult {
    use crate::{LibraryCommand, Operation, Outcome};
    let mut engine = engine(3)?;
    let outcome = engine.apply(
        engine.revision(),
        Operation::Library(LibraryCommand::NewDraft {
            meta: DefinitionMeta {
                name: "EX-DRAFT-PART".into(),
                ..Default::default()
            },
            definition: crate::library::Definition::empty(DefinitionKind::Component),
        }),
    )?;
    let Outcome::DraftCreated { id } = outcome else {
        return Err("no draft".into());
    };
    let search = |include_drafts| {
        engine.query(Query::Search {
            query: "ex".into(),
            limit: None,
            include_drafts,
        })
    };
    let QueryResult::Search(report) = search(false)? else {
        return Err("not a search report".into());
    };
    assert_eq!(report.total, 3);
    assert_eq!(
        report.hidden,
        [Hidden {
            reason: "unpublished draft".into(),
            count: 1
        }]
    );
    let QueryResult::Search(report) = search(true)? else {
        return Err("not a search report".into());
    };
    let draft = report
        .results
        .iter()
        .find(|h| h.entry.lineage == id)
        .ok_or("draft missing")?;
    assert!(draft.entry.draft && draft.entry.revision.is_none());
    assert!(report.hidden.is_empty());
    Ok(())
}

#[test]
fn inspect_reports_the_latest_revision_and_what_it_pins() -> TestResult {
    let engine = Engine::demo()?;
    let lineage = engine.project().instances[0].component.id;
    let QueryResult::Lineage(report) = engine.query(Query::InspectLineage {
        lineage,
        revision: None,
    })?
    else {
        return Err("not a lineage report".into());
    };
    assert!(report.latest);
    assert_eq!(report.entry.revision, Some(Revision::FIRST));
    assert_eq!(report.entry.used, 2);
    assert_eq!(report.revisions.len(), 1);
    assert_eq!(report.symbols[0].units, ["A"]);
    assert_eq!(report.implementations[0].footprints[0].pads, 2);
    assert!(report.entry.badges.symbol && report.entry.badges.footprint);
    assert!(report.entry.badges.model && !report.entry.badges.mpn);
    let missing = engine.query(Query::InspectLineage {
        lineage,
        revision: Some(Revision(9)),
    });
    assert!(missing.is_err_and(|e| e.code() == "not_found"));
    // Queries leave the document, revision and history unchanged.
    let before = engine.document().clone();
    let revision = engine.revision();
    engine.query(Query::Facets {
        query: "R>=1".into(),
        include_drafts: false,
    })?;
    assert_eq!(engine.document(), &before);
    assert_eq!(engine.revision(), revision);
    assert!(!engine.can_undo());
    Ok(())
}

/// Keystroke-to-results latency of core `search` on 10 k–50 k synthetic
/// entries. Run with
/// `cargo test --release -p sparksmith-core catalog::tests::bench -- --ignored --nocapture`.
/// Target (not a gate): ≤ 10 ms p95 at 50 k entries, warm.
#[test]
#[ignore = "benchmark; run explicitly in release mode"]
fn bench_search_on_synthetic_entries() -> TestResult {
    use std::time::Instant;
    const RUNS: usize = 60;
    let queries = [
        "10k",
        "res>=10k pkg:0603",
        "tol<=1%",
        "ex-c",
        "hdr",
        "4k7",
        "capacitor 50V",
    ];
    for count in [10_000, 50_000] {
        let built = Instant::now();
        let index = synthetic::index(count)?;
        let build = built.elapsed();
        let bytes: usize = index
            .entries
            .iter()
            .map(|e| {
                std::mem::size_of::<CatalogEntry>()
                    + e.name.capacity()
                    + e.description.capacity()
                    + e.packages.iter().map(|p| p.capacity() + 24).sum::<usize>()
                    + e.fields
                        .iter()
                        .map(|f| {
                            std::mem::size_of::<EntryField>()
                                + f.label.capacity()
                                + f.text.capacity()
                        })
                        .sum::<usize>()
            })
            .sum::<usize>()
            + index
                .folded
                .iter()
                .map(|f| {
                    std::mem::size_of::<Folded>()
                        + f.name.capacity()
                        + f.lineage.capacity()
                        + f.other.capacity()
                        + f.fields.capacity()
                        + f.words.iter().map(|w| w.capacity() + 24).sum::<usize>()
                })
                .sum::<usize>();
        println!(
            "entries {count}: index build {:.1} ms, approx. {:.1} MB",
            build.as_secs_f64() * 1e3,
            bytes as f64 / 1e6
        );
        for text in queries {
            let mut times = Vec::with_capacity(RUNS);
            let mut total = 0;
            for _ in 0..RUNS {
                let start = Instant::now();
                let query = index.parse(text)?;
                let report =
                    index.search(&query, &CatalogContext::default(), SearchOptions::default());
                times.push(start.elapsed().as_secs_f64() * 1e3);
                total = report.total;
            }
            times.sort_by(f64::total_cmp);
            let p50 = times[RUNS / 2];
            let p95 = times[RUNS * 95 / 100];
            println!("  {text:<18} matches {total:>6}  p50 {p50:>6.2} ms  p95 {p95:>6.2} ms");
        }
    }
    Ok(())
}

/// A Part entry's rated value under `meets`: decided by `fields::satisfies`,
/// so a rated range must cover the requirement and tolerances of another
/// basis compare as intervals around the Part's nominal.
#[test]
fn part_predicates_are_decided_by_satisfies() {
    use crate::fields::{RatedSpec, ToleranceBasis};
    use crate::ids::{FieldRef, QuantityKindRef};
    let kind = QuantityKindRef::new(DefinitionId(7), Revision::FIRST);
    let field = FieldRef::new(DefinitionId(9), Revision::FIRST);
    let rated = |value: FieldValue, value_type: ValueType, meaning, nominal| EntryField {
        field: field.id,
        label: "Rated".into(),
        text: String::new(),
        value: value.clone(),
        shown: false,
        meaning: Some(meaning),
        rated: Some(RatedReading {
            spec: RatedSpec {
                field,
                value,
                meaning,
            },
            value_type,
            nominal,
        }),
    };
    let test = |test: Test, value_type: ValueType| FieldTest {
        field: field.id,
        test,
        value_type: Some(value_type),
        display: String::new(),
        unitless: false,
    };
    // An operating range of −40 … 125 °C rated as a minimum.
    let range = ValueType::Range { kind };
    let operating = rated(
        FieldValue::Range {
            min: dec(-40.0),
            max: dec(125.0),
        },
        range.clone(),
        SpecMeaning::MinimumRating,
        None,
    );
    let point = |value| test(Test::Compare { op: Op::Eq, value }, range.clone());
    assert!(meets(&operating, &point(25.0)));
    // The literal `≥` rewrite accepted any value below the maximum.
    assert!(!meets(&operating, &point(-50.0)));
    assert!(!meets(&operating, &point(150.0)));
    assert!(meets(
        &operating,
        &test(
            Test::Between {
                min: -20.0,
                max: 85.0
            },
            range.clone()
        )
    ));
    assert!(!meets(
        &operating,
        &test(
            Test::Between {
                min: -55.0,
                max: 85.0
            },
            range.clone()
        )
    ));
    // Explicit comparisons keep their literal meaning.
    assert!(meets(
        &operating,
        &test(
            Test::Compare {
                op: Op::Ge,
                value: -50.0
            },
            range
        )
    ));

    // A ±50 Ω absolute rating on a 10 kΩ Part is ±0.5 %: it meets ±1 %
    // and not ±0.25 %. Compared literally, 50 > 0.01 would refuse it.
    let relative = ValueType::Tolerance {
        basis: ToleranceBasis::Relative,
    };
    let absolute = ValueType::Tolerance {
        basis: ToleranceBasis::Absolute { kind },
    };
    let ohms = |nominal| {
        rated(
            FieldValue::Tolerance {
                minus: dec(50.0),
                plus: dec(50.0),
            },
            absolute.clone(),
            SpecMeaning::MaximumTolerance,
            nominal,
        )
    };
    let percent = |p: f64| {
        test(
            Test::Tolerance {
                op: Op::Eq,
                minus: p,
                plus: p,
            },
            relative.clone(),
        )
    };
    assert!(meets(&ohms(Some(10e3)), &percent(0.01)));
    assert!(!meets(&ohms(Some(10e3)), &percent(0.0025)));
    // Without a nominal the bases cannot be compared: no hit.
    assert!(!meets(&ohms(None), &percent(0.01)));
    // A 25 V minimum rating meets 16 V written with its unit, but a bare
    // value without one names a value, not a rating: `100n` is not "at
    // least 100 nV".
    let quantity = ValueType::Quantity {
        kind,
        min: None,
        max: None,
    };
    let volts = rated(
        FieldValue::Quantity {
            si: dec(25.0),
            display_unit: None,
        },
        quantity.clone(),
        SpecMeaning::MinimumRating,
        None,
    );
    let written = |value, unitless| FieldTest {
        unitless,
        ..test(Test::Compare { op: Op::Eq, value }, quantity.clone())
    };
    assert!(meets(&volts, &written(16.0, false)));
    assert!(!meets(&volts, &written(1e-7, true)));
    // An exact nominal still meets a unitless value equal to it.
    let farads = rated(
        FieldValue::Quantity {
            si: dec(1e-7),
            display_unit: None,
        },
        quantity.clone(),
        SpecMeaning::ExactNominal,
        None,
    );
    assert!(meets(&farads, &written(1e-7, true)));
}

/// An exact decimal from a finite fixture literal.
#[allow(clippy::expect_used)]
pub fn dec(value: f64) -> crate::quantity::Decimal {
    crate::quantity::Decimal::from_f64(value).expect("a finite literal")
}

/// A page past the first [`MAX_RESULTS`] results holds the results ranked
/// there (review, offline registry): paging through every match lists
/// each once, in the order one ranked list gives.
#[test]
fn pages_past_the_result_limit_hold_the_results_ranked_there() -> TestResult {
    let index = index(700)?;
    let query = index.parse("")?;
    let page = |offset: usize, limit: usize| {
        index.search(
            &query,
            &CatalogContext::default(),
            SearchOptions {
                limit,
                offset,
                include_drafts: false,
            },
        )
    };
    let first = page(0, MAX_RESULTS);
    assert!(first.total > MAX_RESULTS, "{} matches", first.total);
    let mut listed: Vec<String> = Vec::new();
    let mut offset = 0;
    while offset < first.total {
        let report = page(offset, 100);
        assert_eq!(report.total, first.total);
        assert!(!report.results.is_empty(), "page at {offset} is empty");
        listed.extend(names(&report).into_iter().map(str::to_owned));
        offset += 100;
    }
    assert_eq!(listed.len(), first.total);
    assert_eq!(listed[..MAX_RESULTS], names(&first)[..]);
    let keys: std::collections::BTreeSet<_> = (0..first.total)
        .flat_map(|o| page(o, 1).results.into_iter().map(|h| h.entry.key()))
        .collect();
    assert_eq!(keys.len(), first.total, "every match once");
    Ok(())
}

/// Run 3, A3: without a unit, a multiplier is read only as the fields that
/// take it bare (the search vocabulary's value slots): "100n" is a
/// capacitance, never 100 nΩ or 100 nW, and "10k" still a resistance.
#[test]
fn a_bare_multiplier_reads_as_the_kinds_that_take_it() -> TestResult {
    let _context = crate::library::standards::Context::fixture()?.enter();
    let index = index(24)?;
    let one = |text: &str| -> std::result::Result<Term, Box<dyn std::error::Error>> {
        index
            .parse(text)?
            .terms
            .into_iter()
            .next()
            .ok_or_else(|| "no term".into())
    };
    let resistance = field_id(&index, "Resistance")?;
    let capacitance = field_id(&index, "Capacitance")?;
    let nano = one("100n")?;
    assert_eq!(reading(&nano, resistance), None, "{nano:?}");
    assert!(reading(&nano, capacitance).is_some_and(|v| (v - 100e-9).abs() < 1e-18));
    if let Ok(power) = field_id(&index, "Rated power") {
        assert_eq!(reading(&nano, power), None, "{nano:?}");
    }
    let kilo = one("10k")?;
    assert_eq!(reading(&kilo, resistance), Some(10e3));
    assert_eq!(reading(&kilo, capacitance), None);
    // With its unit a value is read as written.
    assert!(reading(&one("100nΩ")?, resistance).is_some());
    let found = search(&index, "100n 0603")?;
    assert!(
        names(&found).iter().all(|n| n.starts_with("EX-C")),
        "{:?}",
        names(&found)
    );
    Ok(())
}
