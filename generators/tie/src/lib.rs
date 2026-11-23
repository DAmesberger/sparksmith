//! Intentional connections between independently routed nets.
#![cfg_attr(target_arch = "wasm32", no_std)]
extern crate alloc;
use alloc::{format, string::String, vec, vec::Vec};
use sparksmith_generator_sdk::{Generator, export_generator, v1::*};
use sparksmith_generator_sdk::{
    args::{checked, problem},
    draw, land,
};
#[path = "../../chip/src/family.rs"]
mod chip;
include!(concat!(env!("OUT_DIR"), "/solder.rs"));

pub struct TieGenerator;
export_generator!(TieGenerator);

fn choice(key: &str, label: &str, options: &[(&str, &str)], default: &str) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        ty: ParamType::Enum {
            choices: options
                .iter()
                .map(|&(k, l)| Choice {
                    key: k.into(),
                    label: l.into(),
                })
                .collect(),
        },
        default: ParamValue::Text(default.into()),
        help: None,
        group: None,
        context: None,
    }
}
fn length(key: &str, label: &str, context: Option<&str>) -> Param {
    Param {
        key: key.into(),
        label: label.into(),
        ty: ParamType::Length {
            min_nm: 0,
            max_nm: limits::COORDINATE_NM / 16,
            step_nm: None,
        },
        default: ParamValue::Int(0),
        help: Some("Enter the dimension for this board.".into()),
        group: None,
        context: context.map(String::from),
    }
}
fn describe() -> Description {
    let chip = chip::description();
    let chip_sizes = chip
        .presets()
        .iter()
        .filter(|p| p.params.get("part") == Some(&ParamValue::Text("resistor".into())))
        .map(|p| (p.key.as_str(), p.name.as_str()))
        .collect::<Vec<_>>();
    let mut presets =
        sparksmith_generator_sdk::presets(include_str!("../presets.json")).unwrap_or_default();
    for p in chip
        .presets()
        .iter()
        .filter(|p| chip_sizes.iter().any(|(k, _)| *k == p.key))
    {
        let mut row = p.clone();
        row.params = Params::from([
            ("form".into(), ParamValue::Text("zero_ohm".into())),
            ("chip_size".into(), ParamValue::Text(p.key.clone())),
        ]);
        row.key = format!("zero-ohm-{}", p.key);
        row.name = format!("0 Ω link · {}", p.name);
        row.aliases = vec![];
        presets.push(row);
    }
    let mut params = vec![
        choice(
            "form",
            "Form",
            &[
                ("kelvin", "Kelvin tie"),
                ("copper", "Copper tie"),
                ("solder", "Solder jumper"),
                ("trace", "Cuttable trace jumper"),
                ("zero_ohm", "0 Ω link"),
            ],
            "kelvin",
        ),
        Param {
            key: "ends".into(),
            label: "Number of lands or Kelvin ends".into(),
            ty: ParamType::Count { min: 2, max: 8 },
            default: ParamValue::Int(2),
            group: None,
            help: None,
            context: None,
        },
        choice(
            "state",
            "Default jumper state",
            &[("open", "Open"), ("closed", "Closed")],
            "open",
        ),
        choice(
            "pair",
            "Jumper lands",
            &[("first", "1–2"), ("last", "2–3")],
            "first",
        ),
        choice(
            "chip_size",
            "Resistor package",
            &chip_sizes,
            "resistor-0603",
        ),
        length("pad_length", "Pad length", None),
        length("pad_width", "Pad width", None),
        length("bridge_width", "Bridge width", None),
        length(
            "bridge_length",
            "Copper bridge length (0 joins land centres)",
            None,
        ),
        length("gap", "Gap between lands", Some("project.copper.pad_gap")),
        length("solder_height", "Illustrative solder height", None),
    ];
    params.extend(
        chip.params()
            .iter()
            .filter(|p| {
                matches!(
                    p.key.as_str(),
                    "resistor_style" | "land_gap_min" | BOARD_MASK_EXPANSION
                )
            })
            .cloned(),
    );
    Description {
        abi: ABI,
        id: "tie".into(),
        version: 2,
        title: "Net ties and jumpers".into(),
        licence: "MIT OR Apache-2.0".into(),
        deterministic: true,
        needs: vec![],
        capabilities: vec![Capability {
            name: GENERATE_PART.into(),
            effect: Effect::None,
            title: Some("Make a tie".into()),
            params,
            rules: vec![],
            presets,
            part: Some(PartOffer {
                families: vec!["tie".into()],
                scopes: vec![Scope::Component],
                grammar: vec![],
                instance_fields: vec![],
            }),
        }],
    }
}

fn zero_ohm(size: &str, request: &Request) -> Result<Generation, Vec<Problem>> {
    let desc = chip::description();
    let p = desc
        .presets()
        .iter()
        .find(|p| p.key == size)
        .ok_or_else(|| {
            vec![problem(
                "param.out_of_range",
                "chip_size",
                "Choose a resistor package.".into(),
            )]
        })?;
    let mut params: Params = desc
        .params()
        .iter()
        .map(|p| (p.key.clone(), p.default.clone()))
        .collect();
    params.extend(p.params.clone());
    for key in ["resistor_style", "land_gap_min", BOARD_MASK_EXPANSION] {
        if let Some(value) = request.params.get(key) {
            params.insert(key.into(), value.clone());
        }
    }
    params.insert("part".into(), ParamValue::Text("zero_ohm".into()));
    chip::generate(&Request {
        abi: ABI,
        scope: Scope::Component,
        params,
    })
}

impl Generator for TieGenerator {
    fn describe() -> Description {
        describe()
    }
    fn generate(request: &Request) -> Result<Generation, Vec<Problem>> {
        let description = describe();
        let args = checked(&description, &request.params)?;
        let one = |p| vec![p];
        let text = |k| args.text(k).map_err(one);
        let number = |k| args.int(k).map_err(one);
        let key = text("form")?;
        if key == "zero_ohm" {
            return zero_ohm(text("chip_size")?, request);
        }
        let landless = key == "kelvin";
        let jumper = key == "solder" || key == "trace";
        let form = if jumper {
            TieForm::SolderJumper {
                default: if text("state")? == "open" {
                    JumperState::Open
                } else {
                    JumperState::Closed
                },
                bridge: if key == "solder" {
                    JumperBridge::Solder
                } else {
                    JumperBridge::Trace
                },
            }
        } else {
            TieForm::Copper
        };
        let count = number("ends")?;
        if jumper && count > 3 {
            return Err(vec![problem(
                "param.inconsistent",
                "ends",
                "Choose two or three jumper lands.".into(),
            )]);
        }
        let last = text("pair")? == "last";
        if jumper && count == 2 && last {
            return Err(vec![problem(
                "param.inconsistent",
                "pair",
                "Lands 2–3 require a three-land jumper.".into(),
            )]);
        }
        let length = number("pad_length")?;
        let width = number("pad_width")?;
        let gap = number("gap")?;
        let neck = number("bridge_width")?;
        let height = number("solder_height")?;
        if !landless && (length <= 0 || width <= 0 || gap <= 0) {
            return Err(vec![problem(
                "param.inconsistent",
                "gap",
                "Supply positive pad dimensions and the board's land gap.".into(),
            )]);
        }
        if !landless && (neck <= 0 || neck > width || (key == "solder" && height <= 0)) {
            return Err(vec![problem("param.inconsistent","bridge_width","Choose a positive bridge width that fits the lands. A solder bridge also needs a positive model height.".into())]);
        }
        let pitch = length + gap;
        // The native bead rounds the top edge to half its height. Leave
        // enough metal on each side of that fillet for a valid solid.
        if key == "solder" && height >= neck.min(pitch) {
            return Err(vec![problem(
                "param.inconsistent",
                "solder_height",
                "The solder height must fit within both the bridge width and the span between lands.".into(),
            )]);
        }
        let x = |n: i64| (2 * n - count - 1) * pitch / 2;
        let first = if jumper && last { 2 } else { 1 };
        let end = if jumper { first + 1 } else { count };
        let ends: Vec<String> = (first..=end).map(|i| format!("{i}")).collect();
        let mut terminals = Vec::new();
        let mut binding = Vec::new();
        let mut keys = Vec::new();
        let mut leads = Vec::new();
        let mut pads = Vec::new();
        let mut drawing = draw::Drawing {
            numbered: true,
            ..Default::default()
        };
        for n in 1..=count {
            let k = format!("{n}");
            terminals.push(Terminal {
                key: k.clone(),
                name: k.clone(),
                direction: Direction::Passive,
                alternates: vec![],
                drive: None,
                unconnected: None,
            });
            let y = (count + 1 - 2 * n) * draw::MODULE_NM;
            drawing.pin(&k, [-2 * draw::MODULE_NM, y], draw::RIGHT, draw::MODULE_NM);
            if n >= first && n <= end {
                drawing.line([-draw::MODULE_NM, y], [0, y]);
            }
            if landless {
                continue;
            }
            keys.push(k.clone());
            if key == "solder" && (first..=end).contains(&n) {
                leads.push(Lead {
                    key: k.clone(),
                    contact: [x(n), 0, 0],
                });
            }
            binding.push(BindingEntry {
                package_terminal: k.clone(),
                terminal: k.clone(),
            });
            pads.push(Pad {
                key: k.clone(),
                terminal: k,
                kind: None,
                number: None,
                shape: PadShape::Rect {
                    size: [length, width],
                },
                at: [x(n), 0],
                rot_mdeg: 0,
                side: Side::Top,
                hole: None,
                mask: Opening::Default,
                paste: if jumper {
                    Opening::Default
                } else {
                    Opening::None
                },
            });
        }
        drawing.line(
            [0, (count + 1 - 2 * first) * draw::MODULE_NM],
            [0, (count + 1 - 2 * end) * draw::MODULE_NM],
        );
        drawing.labels(count * draw::MODULE_NM, -count * draw::MODULE_NM);
        let a = x(first);
        let b = x(end);
        let bridges = if !landless && form.needs_bridge() {
            let span = match number("bridge_length")? {
                0 => b - a,
                value => value,
            };
            let left = a + (b - a - span) / 2;
            let right = left + span;
            if left >= a + length / 2
                || right <= b - length / 2
                || left < a - length / 2
                || right > b + length / 2
            {
                return Err(vec![problem(
                    "param.inconsistent",
                    "bridge_length",
                    "Both ends of the copper bridge must land on their pads.".into(),
                )]);
            }
            vec![Bridge {
                key: "neck".into(),
                side: Side::Top,
                pads: ends.clone(),
                outline: vec![
                    [left, -neck / 2],
                    [right, -neck / 2],
                    [right, neck - neck / 2],
                    [left, neck - neck / 2],
                ],
            }]
        } else {
            vec![]
        };
        let model = if key == "solder" {
            Model::PartFile {
                asset: SOLDER_BRIDGE.into(),
                values: Params::from([
                    ("length".into(), ParamValue::Int(b - a)),
                    ("width".into(), ParamValue::Int(neck)),
                    ("height".into(), ParamValue::Int(height)),
                    ("shift".into(), ParamValue::Int((a + b) / 2)),
                ]),
            }
        } else {
            Model::NoBody
        };
        let name = match key {
            "kelvin" => "Kelvin tie",
            "solder" => "Solder jumper",
            "trace" => "Trace jumper",
            _ => "Copper tie",
        };
        Ok(Generation {
            component: Some(Component {
                ties: vec![Tie { id: 1, form, ends }],
                assembly: form.assembly(),
                name: name.into(),
                prefix: if jumper { "JP" } else { "NT" }.into(),
                terminals,
                fields: vec![],
            }),
            symbol: Some(Symbol {
                units: vec![drawing.unit("A")],
            }),
            package: Package {
                geometry: PackageGeometry {
                    envelope_nm: [
                        ((count - 1) * pitch + length).max(10_000),
                        width.max(10_000),
                        height.max(10_000),
                    ],
                    seating_nm: 0,
                    leads,
                    pin1: keys.first().cloned().unwrap_or_default(),
                },
                terminals: keys,
            },
            footprint: Footprint {
                bridges,
                courtyard: land::courtyard(&pads, None).unwrap_or_default(),
                pads,
                silk: vec![],
                assembly: vec![],
                keepouts: vec![],
            },
            binding,
            model: Some(model),
            fields: vec![],
            orderable: vec![],
            marking: None,
            notes: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(form: &str, ends: i64) -> Request {
        let mut params: Params = describe()
            .params()
            .iter()
            .map(|p| (p.key.clone(), p.default.clone()))
            .collect();
        for (key, value) in [
            ("pad_width", 900000),
            ("pad_length", 600000),
            ("gap", 250000),
            ("bridge_width", 150000),
            ("solder_height", 100000),
            ("ends", ends),
        ] {
            params.insert(key.into(), ParamValue::Int(value));
        }
        params.insert("form".into(), ParamValue::Text(form.into()));
        Request {
            abi: ABI,
            scope: Scope::Component,
            params,
        }
    }

    #[test]
    fn every_copper_end_has_a_declared_land() -> Result<(), Vec<Problem>> {
        for n in 2..=8 {
            let g = TieGenerator::generate(&request("copper", n))?;
            let bridge = &g.footprint.bridges[0];
            assert_eq!(bridge.pads.len(), n as usize);
            assert_eq!(bridge.pads, g.package.terminals);
            assert_eq!(g.model, Some(Model::NoBody));
        }
        Ok(())
    }
    #[test]
    fn an_explicit_copper_length_must_overlap_the_end_lands() -> Result<(), Vec<Problem>> {
        let mut r = request("copper", 2);
        r.params
            .insert("bridge_length".into(), ParamValue::Int(450_000));
        let g = TieGenerator::generate(&r)?;
        let outline = &g.footprint.bridges[0].outline;
        assert_eq!(outline[1][0] - outline[0][0], 450_000);
        r.params
            .insert("bridge_length".into(), ParamValue::Int(250_000));
        assert!(TieGenerator::generate(&r).is_err());
        Ok(())
    }

    #[test]
    fn solder_and_trace_join_only_the_selected_pair() -> Result<(), Vec<Problem>> {
        for key in ["solder", "trace"] {
            let mut r = request(key, 3);
            r.params
                .insert("pair".into(), ParamValue::Text("last".into()));
            let g = TieGenerator::generate(&r)?;
            assert_eq!(g.footprint.pads.len(), 3);
            let contacted: Vec<_> = g
                .package
                .geometry
                .leads
                .iter()
                .map(|lead| lead.key.as_str())
                .collect();
            let expected = if key == "solder" {
                &["2", "3"][..]
            } else {
                &[]
            };
            assert_eq!(contacted, expected);
            assert!(sparksmith_generator_sdk::golden::terminals_without_contacts(&g).is_empty());
            assert_eq!(
                g.component
                    .and_then(|c| c.ties.into_iter().next())
                    .map(|t| t.ends),
                Some(vec!["2".into(), "3".into()])
            );
            assert_eq!(g.footprint.bridges.len(), usize::from(key == "trace"));
            assert_eq!(
                matches!(g.model, Some(Model::PartFile { .. })),
                key == "solder"
            );
        }
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_defaults_and_every_preset_generate() -> Result<(), Vec<Problem>> {
        let description = sparksmith_generator_sdk::described::<TieGenerator>();
        for case in sparksmith_generator_sdk::abi::parity::cases(&description) {
            let part = sparksmith_generator_sdk::generate::<TieGenerator>(&case.request)?;
            assert_eq!(
                part.component.as_ref().map(|c| c.ties.len()),
                Some(1),
                "{}",
                case.name
            );
        }
        Ok(())
    }
    #[test]
    fn a_zero_ohm_link_uses_the_chip_body_footprint_and_project_gap() -> Result<(), Vec<Problem>> {
        let mut r = request("zero_ohm", 2);
        r.params
            .insert("land_gap_min".into(), ParamValue::Int(500_000));
        let link = TieGenerator::generate(&r)?;
        let desc = chip::description();
        let mut params: Params = desc
            .params()
            .iter()
            .map(|p| (p.key.clone(), p.default.clone()))
            .collect();
        let preset = desc
            .presets()
            .iter()
            .find(|p| p.key == "resistor-0603")
            .ok_or_else(|| {
                vec![problem(
                    "test.preset",
                    "chip_size",
                    "Missing resistor preset.".into(),
                )]
            })?;
        params.extend(preset.params.clone());
        params.insert("land_gap_min".into(), ParamValue::Int(500_000));
        let resistor = chip::generate(&Request {
            abi: ABI,
            scope: Scope::Component,
            params,
        })?;
        assert_eq!(
            include_bytes!("../parts/chip-resistor.solidcraft").as_slice(),
            include_bytes!("../../chip/parts/chip-resistor.solidcraft").as_slice(),
        );
        assert_eq!(link.model, resistor.model);
        assert_eq!(link.package, resistor.package);
        assert_eq!(link.footprint, resistor.footprint);
        assert!(
            link.component
                .is_some_and(|c| c.ties.len() == 1 && c.ties[0].form == TieForm::ZeroOhm)
        );
        Ok(())
    }
    #[test]
    fn golden_hashes_match_natively() -> Result<(), String> {
        let count = sparksmith_generator_sdk::golden::check::<TieGenerator>(include_str!(
            "../golden.json"
        ))?;
        assert_eq!(count, 5);
        Ok(())
    }
}
