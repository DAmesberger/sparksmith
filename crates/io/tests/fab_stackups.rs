//! The fabricators' own stackups (`library/stackups/<fabricator>/*.json`):
//! each file loads, cites its source and the row of each value, belongs to a
//! shipped fab profile that covers its copper count, is mirror-symmetric
//! exactly where its flag says so, and states a thickness that equals the sum
//! of its layers. No code reads these files yet (see the stackups report), so
//! this test is what keeps them well-formed.
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

type Fallible<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The stored sums are rounded to 5 decimals (0.01 µm resolution), twice for
/// the difference from the nominal thickness.
const TOLERANCE_MM: f64 = 2e-5;

fn err(message: String) -> Box<dyn std::error::Error> {
    message.into()
}

fn library() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library")
}

struct Preset {
    path: PathBuf,
    body: Value,
}

impl Preset {
    fn label(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn get(&self, key: &str) -> Fallible<&Value> {
        self.body
            .get(key)
            .ok_or_else(|| err(format!("{}: no \"{key}\"", self.label())))
    }

    fn text(&self, key: &str) -> Fallible<&str> {
        self.get(key)?
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| err(format!("{}: \"{key}\" is not text", self.label())))
    }

    fn num(&self, key: &str) -> Fallible<f64> {
        self.get(key)?
            .as_f64()
            .ok_or_else(|| err(format!("{}: \"{key}\" is not a number", self.label())))
    }

    fn layers(&self) -> Fallible<&Vec<Value>> {
        self.get("layers")?
            .as_array()
            .ok_or_else(|| err(format!("{}: \"layers\" is not a list", self.label())))
    }

    fn copper_layers(&self) -> Fallible<usize> {
        let n = self
            .get("copper_layers")?
            .as_u64()
            .ok_or_else(|| err(format!("{}: copper_layers", self.label())))?;
        Ok(usize::try_from(n)?)
    }
}

fn layer_text<'a>(layer: &'a Value, key: &str, label: &str) -> Fallible<&'a str> {
    layer
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| err(format!("{label}: a layer has no \"{key}\"")))
}

fn layer_num(layer: &Value, key: &str, label: &str) -> Fallible<f64> {
    layer
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| err(format!("{label}: a layer has no number \"{key}\"")))
}

/// Every preset file under `library/stackups/<fabricator>/`.
fn presets() -> Fallible<Vec<Preset>> {
    let root = library().join("stackups");
    let mut out = Vec::new();
    for fabricator in std::fs::read_dir(&root)? {
        let fabricator = fabricator?;
        if !fabricator.file_type()?.is_dir() {
            continue;
        }
        for file in std::fs::read_dir(fabricator.path())? {
            let path = file?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let file: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
            let label = path.display().to_string();
            if file.get("format").and_then(Value::as_u64) != Some(1) {
                return Err(err(format!("{label}: format is not 1")));
            }
            let body = file
                .get("fab_stackup")
                .cloned()
                .ok_or_else(|| err(format!("{label}: no fab_stackup")))?;
            out.push(Preset { path, body });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn key_of(layer: &Value) -> String {
    let get = |k: &str| layer.get(k).cloned().unwrap_or(Value::Null);
    serde_json::json!([
        get("role"),
        get("thickness_mm"),
        get("after_lamination_mm"),
        get("dk"),
        get("glass")
    ])
    .to_string()
}

fn profile(id: &str) -> Fallible<Value> {
    let path = library().join("profiles").join(format!("{id}.json"));
    let file: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    file.get("fab_profile")
        .cloned()
        .ok_or_else(|| err(format!("{id}: no fab_profile")))
}

#[test]
fn every_preset_loads_cites_its_source_and_names_a_profile_that_covers_it() -> Fallible<()> {
    let all = presets()?;
    assert!(all.len() >= 100, "only {} presets found", all.len());
    let mut ids = BTreeSet::new();
    for p in &all {
        let label = p.label();
        let id = p.text("id")?;
        assert!(
            !id.is_empty()
                && id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !id.starts_with('-')
                && !id.ends_with('-'),
            "{label}: {id:?} is not a library id"
        );
        assert_eq!(
            Some(format!("{id}.json")),
            Some(label.clone()),
            "{label}: the file is named for its id"
        );
        assert!(ids.insert(id.to_owned()), "{label}: id {id} is used twice");
        p.text("name")?;
        let fabricator = p.text("fabricator")?;
        assert_eq!(
            p.path
                .parent()
                .and_then(Path::file_name)
                .and_then(|n| n.to_str()),
            Some(fabricator),
            "{label}: the folder is the fabricator's id"
        );

        // The source, as the profiles cite theirs.
        let source = p.get("source")?;
        for key in ["publisher", "document", "url", "date", "notes"] {
            let text = source.get(key).and_then(Value::as_str).unwrap_or("");
            assert!(!text.trim().is_empty(), "{label}: source has no {key}");
        }
        let url = source.get("url").and_then(Value::as_str).unwrap_or("");
        assert!(url.starts_with("https://"), "{label}: source url {url}");
        let date = source.get("date").and_then(Value::as_str).unwrap_or("");
        assert!(
            date.len() == 10 && date.as_bytes().get(4) == Some(&b'-'),
            "{label}: source date {date}"
        );

        // The profiles that go with it cover its copper count.
        let count = p.copper_layers()?;
        let named = p
            .get("fab_profiles")?
            .as_array()
            .ok_or_else(|| err(format!("{label}: fab_profiles")))?;
        assert!(!named.is_empty(), "{label}: no fab profile");
        for profile_id in named {
            let profile_id = profile_id
                .as_str()
                .ok_or_else(|| err(format!("{label}: fab_profiles holds a non-text")))?;
            let shipped = profile(profile_id)?;
            assert_eq!(
                shipped.get("fabricator").and_then(Value::as_str),
                Some(fabricator),
                "{label}: {profile_id} is another fabricator's"
            );
            let covered = shipped
                .pointer("/boards/copper_layers")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_u64).collect::<Vec<_>>())
                .unwrap_or_default();
            assert!(
                covered.contains(&u64::try_from(count)?),
                "{label}: {profile_id} does not cover {count} layers"
            );
        }

        // Each layer: a role, a positive thickness, a Dk in range where it
        // is a dielectric, and the row it was read from.
        let layers = p.layers()?;
        let mut coppers = 0;
        for layer in layers {
            let role = layer_text(layer, "role", &label)?;
            layer_text(layer, "source", &label)?;
            let thickness = layer_num(layer, "thickness_mm", &label)?;
            assert!(
                thickness.is_finite() && thickness > 0.0 && thickness < 5.0,
                "{label}: a {role} of {thickness} mm"
            );
            match role {
                "copper" => coppers += 1,
                "prepreg" | "core" => {
                    let dk = layer_num(layer, "dk", &label)?;
                    assert!((1.0..=20.0).contains(&dk), "{label}: Dk {dk}");
                }
                other => return Err(err(format!("{label}: role {other}"))),
            }
        }
        assert_eq!(coppers, count, "{label}: copper layers listed");
        let first = layers.first().and_then(|l| l.get("role")).and_then(Value::as_str);
        let last = layers.last().and_then(|l| l.get("role")).and_then(Value::as_str);
        assert_eq!(first, Some("copper"), "{label}: starts with copper");
        assert_eq!(last, Some("copper"), "{label}: ends with copper");
        // Between two copper layers there is a dielectric, never two coppers
        // in a row.
        for pair in layers.windows(2) {
            let a = pair[0].get("role").and_then(Value::as_str);
            let b = pair[1].get("role").and_then(Value::as_str);
            assert!(
                !(a == Some("copper") && b == Some("copper")),
                "{label}: two copper layers touch"
            );
        }

        // Disagreements are listed as text.
        for d in p
            .get("disagreements")?
            .as_array()
            .ok_or_else(|| err(format!("{label}: disagreements")))?
        {
            assert!(d.as_str().is_some_and(|s| !s.is_empty()), "{label}");
        }
    }
    Ok(())
}

#[test]
fn a_presets_thickness_is_its_layers_sum_and_its_differences_follow() -> Fallible<()> {
    for p in presets()? {
        let label = p.label();
        let sum: f64 = p
            .layers()?
            .iter()
            .map(|l| layer_num(l, "thickness_mm", &label))
            .sum::<Fallible<f64>>()?;
        let stated = p.num("layers_sum_mm")?;
        assert!(
            (sum - stated).abs() <= TOLERANCE_MM,
            "{label}: layers add to {sum}, the file says {stated}"
        );
        let nominal = p
            .get("nominal")?
            .get("board_mm")
            .and_then(Value::as_f64)
            .ok_or_else(|| err(format!("{label}: nominal.board_mm")))?;
        if let Some(diff) = p.body.get("layers_sum_minus_nominal_mm").and_then(Value::as_f64) {
            assert!(
                (sum - nominal - diff).abs() <= TOLERANCE_MM,
                "{label}: difference from nominal {diff}, computed {}",
                sum - nominal
            );
        } else {
            return Err(err(format!("{label}: no layers_sum_minus_nominal_mm")));
        }
        // JLCPCB's finished thickness, where its row states one.
        let finished = p.body.get("stated_finished").filter(|v| !v.is_null());
        if let (Some(finished), Some(diff)) = (
            finished,
            p.body
                .get("layers_sum_minus_stated_finished_mm")
                .and_then(Value::as_f64),
        ) {
            let value = finished
                .get("value_mm")
                .and_then(Value::as_f64)
                .ok_or_else(|| err(format!("{label}: stated_finished.value_mm")))?;
            assert!(
                (sum - value - diff).abs() <= TOLERANCE_MM,
                "{label}: difference from the stated finished thickness"
            );
        }
        // PCBWay's second sum: after lamination.
        if let Some(after) = p
            .body
            .get("layers_sum_after_lamination_mm")
            .and_then(Value::as_f64)
        {
            let layers = p.layers()?;
            let mut total = 0.0;
            let mut core_with_copper = None;
            for l in layers {
                match l.get("role").and_then(Value::as_str) {
                    Some("copper") => {
                        total += l
                            .get("after_lamination_mm")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0);
                    }
                    Some("prepreg") => total += layer_num(l, "after_lamination_mm", &label)?,
                    Some("core") => {
                        core_with_copper = l.get("listed_with_copper_mm").and_then(Value::as_f64);
                    }
                    _ => {}
                }
            }
            total += core_with_copper
                .ok_or_else(|| err(format!("{label}: no core listed with copper")))?;
            assert!(
                (total - after).abs() <= TOLERANCE_MM,
                "{label}: after lamination {total}, the file says {after}"
            );
        }
    }
    Ok(())
}

#[test]
fn a_preset_is_symmetric_exactly_where_it_says_so() -> Fallible<()> {
    let mut symmetric = 0;
    for p in presets()? {
        let label = p.label();
        let keys: Vec<String> = p.layers()?.iter().map(key_of).collect();
        let mirrored: Vec<String> = keys.iter().rev().cloned().collect();
        let flag = p
            .get("symmetric")?
            .as_bool()
            .ok_or_else(|| err(format!("{label}: symmetric is not true or false")))?;
        assert_eq!(flag, keys == mirrored, "{label}: the symmetric flag");
        if flag {
            symmetric += 1;
        }
        // PCBWay's tables are mirror images by construction.
        if p.text("fabricator")? == "pcbway" {
            assert!(flag, "{label}: PCBWay's stackup is not symmetric");
        }
    }
    assert!(symmetric >= 100, "only {symmetric} symmetric presets");
    Ok(())
}

#[test]
fn every_layer_count_and_thickness_the_fabricators_published_is_covered() -> Fallible<()> {
    let mut seen = BTreeSet::new();
    for p in presets()? {
        let nominal = p
            .get("nominal")?
            .get("board_mm")
            .and_then(Value::as_f64)
            .ok_or_else(|| err(format!("{}: nominal.board_mm", p.label())))?;
        seen.insert((
            p.text("fabricator")?.to_owned(),
            p.copper_layers()?,
            format!("{nominal:.1}"),
        ));
    }
    for layers in [2, 4, 6] {
        for thickness in ["0.8", "1.0", "1.2", "1.6"] {
            assert!(
                seen.contains(&("jlcpcb".to_owned(), layers, thickness.to_owned())),
                "no JLCPCB {layers}-layer {thickness} mm preset"
            );
        }
    }
    for thickness in ["0.8", "1.0", "1.2", "1.6"] {
        assert!(
            seen.contains(&("pcbway".to_owned(), 4, thickness.to_owned())),
            "no PCBWay 4-layer {thickness} mm preset"
        );
    }
    Ok(())
}
