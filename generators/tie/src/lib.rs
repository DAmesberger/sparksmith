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
    let params = vec![
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
        length("gap", "Gap between lands", Some("project.copper.pad_gap")),
        length("solder_height", "Illustrative solder height", None),
    ];
    Description {
        abi: ABI,
        id: "tie".into(),
        version: 1,
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

fn zero_ohm(size: &str) -> Result<Generation, Vec<Problem>> {
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
            return zero_ohm(text("chip_size")?);
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
            leads.push(Lead {
                key: k.clone(),
                contact: [x(n), 0, 0],
            });
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
            vec![Bridge {
                key: "neck".into(),
                side: Side::Top,
                pads: ends.clone(),
                outline: vec![
                    [a, -neck / 2],
                    [b, -neck / 2],
                    [b, neck - neck / 2],
                    [a, neck - neck / 2],
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
                tie: Some(Tie { form, ends }),
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
                        ((count - 1) * pitch + length).max(1),
                        width.max(1),
                        height.max(1),
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
    fn solder_and_trace_join_only_the_selected_pair() -> Result<(), Vec<Problem>> {
        for key in ["solder", "trace"] {
            let mut r = request(key, 3);
            r.params
                .insert("pair".into(), ParamValue::Text("last".into()));
            let g = TieGenerator::generate(&r)?;
            assert_eq!(g.footprint.pads.len(), 3);
            assert_eq!(
                g.component.and_then(|c| c.tie).map(|t| t.ends),
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
}
