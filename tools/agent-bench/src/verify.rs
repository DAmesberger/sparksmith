//! Task success is determined from the saved file, reopened independently of
//! the model session. The starting file pins the unchanged topology of an ECO.
use crate::Result;
use serde::Deserialize;
use sparksmith_core::{Engine, Project, Query, QueryResult};
use std::path::Path;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    outline_mm: [f64; 2],
    width_mm: f64,
    net_names: Vec<String>,
    tracks: usize,
    blockers: usize,
    warnings: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    resistor_value: String,
    led_colour: String,
    chip_size_mm: [f64; 2],
    errors: usize,
}

pub enum Expected {
    Change(Change),
    Build(Build),
}

fn require(ok: bool, reason: &str) -> Result<()> {
    if !ok {
        return Err(reason.to_owned().into());
    }
    Ok(())
}

impl Expected {
    pub fn verify(&self, start: Option<&Path>, saved: &Path) -> Result<()> {
        let file = sparksmith_io::read_project(saved)?;
        let defaults = sparksmith_io::new_project::shipped()?;
        let mut engine = Engine::from_file_with(file, &defaults.presets.glass_dk)?;
        engine.set_context(sparksmith_io::standards::shipped_context()?);
        engine.refill_stale()?;
        let project = engine.project();
        for symbol in project.occurrences.iter() {
            let sheet = project
                .sheet(symbol.sheet)
                .ok_or("symbol sheet is missing")?;
            let [left, top] = sheet.top_left();
            let [width, height] = sheet.size_mm();
            let drawn = sparksmith_core::schematic::placed_unit(project, symbol.id)?;
            let bounds = drawn
                .bounds(Default::default())
                .ok_or("a schematic symbol is empty")?;
            require(
                bounds.min[0] >= left
                    && bounds.max[0] <= left + width
                    && bounds.min[1] >= top - height
                    && bounds.max[1] <= top,
                "a schematic symbol extends outside its printed sheet",
            )?;
        }
        let pdf = sparksmith_io::schematic_pdf::schematic_pdf(project, &[])?;
        require(
            pdf.starts_with(b"%PDF-"),
            "the saved schematic did not render",
        )?;
        match self {
            Self::Change(spec) => {
                let before = sparksmith_io::read_project(start.ok_or("missing starting board")?)?;
                spec.topology(&before.project, project)?;
                let QueryResult::Preflight(report) =
                    engine.query(Query::Preflight { profile: None })?
                else {
                    return Err("preflight did not return a production report".into());
                };
                require(
                    report.blocking == spec.blockers
                        && report.warnings == spec.warnings
                        && report.waived == 0,
                    "production findings differ from the task",
                )?;
            }
            Self::Build(spec) => {
                spec.circuit(project)?;
                let QueryResult::Issues(report) = engine.query(Query::Issues {
                    scope: sparksmith_core::issues::IssueScope::All,
                })?
                else {
                    return Err("issues did not return a check report".into());
                };
                require(
                    report.rows.iter().all(|row| {
                        std::iter::once(&row.finding)
                            .chain(&row.also)
                            .all(|finding| finding.code != "board.unrouted")
                    }),
                    "unrouted connections remain",
                )?;
                require(
                    report.counts.error == spec.errors,
                    "board findings differ from the task",
                )?;
            }
        }
        Ok(())
    }
}

impl Build {
    /// The same series circuit permits either header pin order and either
    /// resistor orientation, with the resistor on the supply or return side.
    fn circuit(&self, project: &Project) -> Result<()> {
        use std::collections::{BTreeMap, BTreeSet};
        let ids: BTreeSet<_> = project.instances.iter().map(|i| i.id).collect();
        let placed: BTreeSet<_> = project.placements.iter().map(|p| p.instance).collect();
        let drawn: BTreeSet<_> = project.occurrences.iter().map(|o| o.instance).collect();
        require(
            ids.len() == 3 && placed == ids && drawn == ids,
            "each of the three parts must be placed on both the board and schematic",
        )?;
        let bom = sparksmith_core::completeness::bom(project)?;
        let match_value = |value: &str| -> Result<_> {
            let found: Vec<_> = bom
                .lines
                .iter()
                .filter(|line| line.fitted && line.values.iter().any(|v| v.text == value))
                .flat_map(|line| line.instances.iter().copied())
                .collect();
            match found.as_slice() {
                [id] => Ok(*id),
                _ => {
                    Err(format!("expected exactly one fitted component with value {value}").into())
                }
            }
        };
        let resistor = match_value(&self.resistor_value)?;
        let diode = match_value(&self.led_colour)?;
        require(
            resistor != diode,
            "the resistor and LED must be separate parts",
        )?;
        let header = ids
            .iter()
            .find(|id| **id != resistor && **id != diode)
            .ok_or("missing header")?;
        let mut roles = BTreeMap::new();
        for part in project.instances.iter() {
            let resolved = sparksmith_core::instances::resolve(&project.library, part)?;
            require(
                resolved.component.definition.terminals.len() == 2,
                "each part must have two terminals",
            )?;
            let role = if part.id == *header {
                require(
                    resolved
                        .component
                        .meta
                        .name
                        .to_lowercase()
                        .contains("header"),
                    "the connector is not a header",
                )?;
                "header"
            } else {
                let physical = resolved
                    .physical
                    .ok_or("a chip has no physical implementation")?;
                let [x, y, _] = physical.package.definition.envelope_nm;
                let size = [x.to_mm_f64(), y.to_mm_f64()];
                require(
                    size == self.chip_size_mm || [size[1], size[0]] == self.chip_size_mm,
                    "a package is not the requested chip size",
                )?;
                if part.id == diode {
                    "diode"
                } else {
                    "resistor"
                }
            };
            roles.insert(part.id, role);
        }
        let mut edges = BTreeMap::new();
        let mut cathode_net = None;
        for net in project.nets.iter() {
            let [a, b] = net.terminals.as_slice() else {
                return Err("each series net must connect exactly two terminals".into());
            };
            let mut pair = [
                *roles.get(&a.instance).ok_or("missing first component")?,
                *roles.get(&b.instance).ok_or("missing second component")?,
            ];
            pair.sort();
            require(
                pair[0] != pair[1] && edges.insert(pair, net.id).is_none(),
                "the circuit does not form one series loop",
            )?;
            for end in [a, b].into_iter().filter(|t| t.instance == diode) {
                let part = project.instance(diode).ok_or("missing LED")?;
                let resolved = sparksmith_core::instances::resolve(&project.library, part)?;
                let pin = resolved
                    .component
                    .definition
                    .terminal(end.terminal)
                    .ok_or("missing LED terminal")?;
                if pin.name.eq_ignore_ascii_case("K") || pin.name.eq_ignore_ascii_case("cathode") {
                    cathode_net = Some(net.id);
                }
            }
        }
        let led_header = edges
            .get(&["diode", "header"])
            .ok_or("missing LED-header connection")?;
        let led_resistor = edges
            .get(&["diode", "resistor"])
            .ok_or("missing LED-resistor connection")?;
        let resistor_header = edges
            .get(&["header", "resistor"])
            .ok_or("missing resistor-header connection")?;
        let ground = match cathode_net {
            Some(net) if net == *led_header => *led_header,
            Some(net) if net == *led_resistor => *resistor_header,
            _ => return Err("missing LED cathode in series circuit".into()),
        };
        require(
            project.fills.iter().any(|fill| {
                fill.net == Some(ground)
                    && fill.layer == sparksmith_core::model::CopperLayer::Bottom
                    && fill
                        .computed
                        .as_ref()
                        .is_some_and(|c| !c.regions.is_empty())
            }),
            "missing bottom copper pour on the supply return",
        )?;
        Ok(())
    }
}

impl Change {
    fn topology(&self, before: &Project, after: &Project) -> Result<()> {
        let mut names: Vec<_> = after.nets.iter().map(|net| net.name.clone()).collect();
        names.sort();
        require(
            names == self.net_names && before.nets == after.nets,
            "named net membership changed",
        )?;
        require(
            before.placements == after.placements
                && before.instances == after.instances
                && before.occurrences == after.occurrences
                && before.wires == after.wires,
            "placement or wiring changed",
        )?;
        require(
            after.board_size == self.outline_mm && after.board_shape == before.board_shape,
            "incorrect board outline",
        )?;
        require(
            before.tracks.len() == self.tracks && after.tracks.len() == self.tracks,
            "incorrect track count",
        )?;
        for original in before.tracks.iter() {
            let mut expected = original.clone();
            expected.width = self.width_mm;
            require(
                after.tracks.contains(&expected),
                "track geometry, ID, net, layer or width differs",
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_correct_outline_and_width_cannot_hide_a_changed_endpoint() -> Result<()> {
        let folder =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/agent-tasks/two-header");
        let spec: Change = serde_json::from_slice(&std::fs::read(folder.join("expected.json"))?)?;
        let before = sparksmith_io::read_project(&folder.join("start.sparksmith"))?.project;
        let mut revised = before.clone();
        revised.board_size = spec.outline_mm;
        for track in revised.tracks.iter_mut() {
            track.width = spec.width_mm;
        }
        spec.topology(&before, &revised)?;
        revised.tracks.first_mut().ok_or("fixture track")?.end[0] += 1.0;
        assert!(spec.topology(&before, &revised).is_err());
        Ok(())
    }
}
