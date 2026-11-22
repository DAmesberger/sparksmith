//! A part's pin table: every generator's `pin_table` parameter.
//!
//! A generator makes a package, and names its terminals by their numbers
//! with no electrical kind, since a package does not say what each pin of
//! the chip in it does. The real part's datasheet does: its pin table
//! gives each pin a name and a type. The `pin_table` parameter carries that
//! table as data, one pin per item, `NUMBER=NAME:KIND`, the number a
//! package terminal of the generated part: a preset of a particular part
//! (a radio module) gives its datasheet's table, and the datasheet
//! workflow gives the one it reads. Each listed terminal takes the name
//! and kind; any other keeps the generator's.
//!
//! The SDK declares the parameter for every generator ([`declare`]) and
//! applies it around the generator's own `generate` ([`generate`]), as it
//! does the land pattern, so a generator neither declares nor reads it.
use crate::Generator;
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sparksmith_generator_abi::v1::{
    Description, Direction, GENERATE_PART, Generation, Param, ParamType, ParamValue, Problem,
    Request, limits,
};

/// The parameter's key.
pub const KEY: &str = "pin_table";

/// Most characters of one pin entry: a number, a name and a kind.
pub const ENTRY_CHARS: u32 = 128;

/// The kinds a pin entry may give, each a terminal's functions: the first
/// is its default, any other one it can be selected to. `io` is a GPIO,
/// an input that can be an output; the rest are fixed.
pub const KINDS: [(&str, Direction, &[Direction]); 7] = [
    ("input", Direction::Input, &[]),
    ("output", Direction::Output, &[]),
    ("io", Direction::Input, &[Direction::Output]),
    ("bidirectional", Direction::Bidirectional, &[]),
    ("passive", Direction::Passive, &[]),
    ("power_input", Direction::PowerInput, &[]),
    ("power_output", Direction::PowerOutput, &[]),
];

/// The `pin_table` parameter: at most [`limits::TERMINALS`] entries, none by
/// default (the generator's names, every terminal passive).
pub fn param() -> Param {
    Param {
        key: KEY.into(),
        label: "Pin table".into(),
        help: Some(
            "The part's pins from its datasheet's pin table, one NUMBER=NAME:KIND per entry: the package terminal's number as the footprint numbers it, the pin's name as the datasheet writes it, and its kind: power_input (supply and ground pins), power_output (a regulator's output), input, output, io (a GPIO: an input that can be an output), bidirectional or passive (analog, no-connect and anything else). A terminal not listed keeps the generator's number as its name and is passive. Empty: no pin table."
                .into(),
        ),
        ty: ParamType::List {
            of: Box::new(ParamType::Text {
                max_chars: ENTRY_CHARS,
            }),
            max_len: limits::TERMINALS as u32,
        },
        default: ParamValue::List(Vec::new()),
        group: Some("pin_table".into()),
        context: None,
    }
}

/// `description` with the `pin_table` parameter added to its part capability
/// (unless it declares one itself).
pub fn declare(mut description: Description) -> Description {
    if let Some(part) = description
        .capabilities
        .iter_mut()
        .find(|c| c.name == GENERATE_PART)
        && !part.params.iter().any(|p| p.key == KEY)
    {
        part.params.push(param());
    }
    description
}

fn problem(message: String) -> Problem {
    Problem {
        code: "param.pin_table".into(),
        field: Some(KEY.into()),
        message,
    }
}

/// Run `G` for `request` with the land pattern applied: the `pin_table`
/// parameter is taken out of the request (the generator's own description
/// does not declare it), the part is made, and the pin table, if any,
/// names and types its terminals.
pub fn generate<G: Generator>(request: &Request) -> Result<Generation, Vec<Problem>> {
    let declared_by_generator = G::describe().params().iter().any(|p| p.key == KEY);
    if declared_by_generator {
        return crate::land_pattern::generate::<G>(request);
    }
    let mut own = request.clone();
    let items = match own.params.remove(KEY) {
        None => Vec::new(),
        Some(ParamValue::List(items)) => items,
        Some(_) => return Err(alloc::vec![problem("The pin table is not a list.".into())]),
    };
    let mut generation = crate::land_pattern::generate::<G>(&own)?;
    if !items.is_empty() {
        apply(&items, &mut generation).map_err(|p| alloc::vec![p])?;
    }
    Ok(generation)
}

/// One pin entry, read: the package terminal's number, the name and the
/// kind's functions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    pub number: String,
    pub name: String,
    pub direction: Direction,
    pub alternates: Vec<Direction>,
}

/// Read entry `n` (`NUMBER=NAME:KIND`; the name may itself hold `=` or
/// `:`, since the number ends at the first `=` and the kind starts after
/// the last `:`).
pub fn read(item: &ParamValue, n: usize) -> Result<Pin, Problem> {
    let ParamValue::Text(entry) = item else {
        return Err(problem(format!("Pin entry {} is not text.", n + 1)));
    };
    let parts = entry.split_once('=').and_then(|(number, rest)| {
        rest.rsplit_once(':')
            .map(|(name, kind)| (number.trim(), name.trim(), kind.trim()))
    });
    let Some((number, name, kind)) =
        parts.filter(|(number, name, _)| !number.is_empty() && !name.is_empty())
    else {
        return Err(problem(format!(
            "\"{entry}\" is not a pin entry: write the pin's number, \"=\", its name, \":\" and its kind, as in 3=EN:input."
        )));
    };
    if name.chars().any(char::is_control) {
        return Err(problem(format!(
            "Pin {number}'s name holds a control character."
        )));
    }
    let Some((_, direction, alternates)) = KINDS.iter().find(|(key, _, _)| *key == kind) else {
        let keys: Vec<&str> = KINDS.iter().map(|(key, _, _)| *key).collect();
        return Err(problem(format!(
            "Pin {number}'s kind \"{kind}\" is not one of: {}.",
            keys.join(", ")
        )));
    };
    Ok(Pin {
        number: number.into(),
        name: name.into(),
        direction: *direction,
        alternates: alternates.to_vec(),
    })
}

/// Name and type `generation`'s terminals by the pin table `items`. A
/// package-scope generation has no component and is left as it is; each
/// entry must still name a package terminal, once.
pub fn apply(items: &[ParamValue], generation: &mut Generation) -> Result<(), Problem> {
    let pins = items
        .iter()
        .enumerate()
        .map(|(n, item)| read(item, n))
        .collect::<Result<Vec<_>, _>>()?;
    let package: BTreeSet<&str> = generation
        .package
        .terminals
        .iter()
        .map(String::as_str)
        .collect();
    let mut seen = BTreeSet::new();
    for pin in &pins {
        if !package.contains(pin.number.as_str()) {
            return Err(problem(format!(
                "The pin table lists pin {}, which this package does not have.",
                pin.number
            )));
        }
        if !seen.insert(pin.number.as_str()) {
            return Err(problem(format!(
                "The pin table lists pin {} twice.",
                pin.number
            )));
        }
    }
    let Some(component) = generation.component.as_mut() else {
        return Ok(());
    };
    // Each package terminal's component terminal, through the binding.
    let bound: BTreeMap<&str, &str> = generation
        .binding
        .iter()
        .map(|b| (b.package_terminal.as_str(), b.terminal.as_str()))
        .collect();
    let mut given: BTreeMap<String, &Pin> = BTreeMap::new();
    for pin in &pins {
        let Some(terminal) = bound.get(pin.number.as_str()) else {
            return Err(problem(format!(
                "Pin {} is bound to no terminal of the part.",
                pin.number
            )));
        };
        if let Some(other) = given.get(*terminal)
            && (other.name != pin.name
                || other.direction != pin.direction
                || other.alternates != pin.alternates)
        {
            return Err(problem(format!(
                "Pins {} and {} are one terminal of the part, so they need the same name and kind.",
                other.number, pin.number
            )));
        }
        given.insert(String::from(*terminal), pin);
    }
    for terminal in &mut component.terminals {
        if let Some(pin) = given.get(&terminal.key) {
            terminal.name.clone_from(&pin.name);
            terminal.direction = pin.direction;
            terminal.alternates.clone_from(&pin.alternates);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use sparksmith_generator_abi::v1::{
        BindingEntry, Component, Footprint, Package, PackageGeometry, Terminal,
    };

    /// Three package terminals; 1 and 2 are one component terminal `a`
    /// (a tactile switch's joined pair), 3 is `b`.
    fn three() -> Generation {
        let terminal = |key: &str| Terminal {
            key: key.into(),
            name: key.into(),
            direction: Direction::Passive,
            alternates: Vec::new(),
        };
        let bind = |package: &str, terminal: &str| BindingEntry {
            package_terminal: package.into(),
            terminal: terminal.into(),
        };
        Generation {
            orderable: Vec::new(),
            component: Some(Component {
                terminals: vec![terminal("a"), terminal("b")],
                prefix: "U".into(),
                name: "Three".into(),
                fields: Vec::new(),
            }),
            symbol: None,
            package: Package {
                terminals: vec!["1".into(), "2".into(), "3".into()],
                geometry: PackageGeometry {
                    envelope_nm: [1, 1, 1],
                    seating_nm: 0,
                    leads: Vec::new(),
                    pin1: "1".into(),
                },
            },
            footprint: Footprint {
                pads: Vec::new(),
                courtyard: Vec::new(),
                silk: Vec::new(),
                assembly: Vec::new(),
                keepouts: Vec::new(),
            },
            binding: vec![bind("1", "a"), bind("2", "a"), bind("3", "b")],
            model: None,
            fields: Vec::new(),
            notes: Vec::new(),
            marking: None,
        }
    }

    fn table(entries: &[&str]) -> Vec<ParamValue> {
        entries.iter().map(|e| text(e)).collect()
    }

    #[test]
    fn names_and_types_terminals_through_the_binding() -> Result<(), Problem> {
        let mut g = three();
        apply(&table(&["3=GPIO:io", "1=GND:power_input"]), &mut g)?;
        let terminals = g.component.map(|c| c.terminals).unwrap_or_default();
        let named: Vec<(&str, Direction, &[Direction])> = terminals
            .iter()
            .map(|t| (t.name.as_str(), t.direction, t.alternates.as_slice()))
            .collect();
        assert_eq!(
            named,
            [
                ("GND", Direction::PowerInput, &[][..]),
                ("GPIO", Direction::Input, &[Direction::Output][..]),
            ]
        );
        Ok(())
    }

    #[test]
    fn refuses_a_table_that_does_not_fit_the_package() {
        let refused = |entries: &[&str]| {
            let mut g = three();
            apply(&table(entries), &mut g).err().map(|p| p.message)
        };
        assert!(refused(&["4=X:input"]).is_some_and(|m| m.contains("pin 4")));
        assert!(refused(&["1=X:input", "1=X:input"]).is_some_and(|m| m.contains("twice")));
        // One terminal, two names.
        assert!(refused(&["1=A:input", "2=B:input"]).is_some_and(|m| m.contains("one terminal")));
        assert_eq!(refused(&["1=A:input", "2=A:input"]), None);
    }

    #[test]
    fn a_package_keeps_its_table_checked_but_has_no_terminals_to_name() -> Result<(), Problem> {
        let mut g = three();
        g.component = None;
        g.binding.clear();
        apply(&table(&["1=GND:power_input"]), &mut g)?;
        let mut g = three();
        g.component = None;
        assert!(apply(&table(&["9=GND:power_input"]), &mut g).is_err());
        Ok(())
    }

    fn text(s: &str) -> ParamValue {
        ParamValue::Text(s.into())
    }

    #[test]
    fn reads_number_name_and_kind() -> Result<(), Problem> {
        assert_eq!(
            read(&text("3=EN:input"), 0)?,
            Pin {
                number: "3".into(),
                name: "EN".into(),
                direction: Direction::Input,
                alternates: Vec::new(),
            }
        );
        // A GPIO is an input that can be an output.
        let io = read(&text("27 = IO0 : io"), 0)?;
        assert_eq!(io.name, "IO0");
        assert_eq!(io.direction, Direction::Input);
        assert_eq!(io.alternates, [Direction::Output]);
        // The name may hold "=" and ":"; the kind is after the last ":".
        assert_eq!(read(&text("5=A=B:C:passive"), 0)?.name, "A=B:C");
        Ok(())
    }

    #[test]
    fn refuses_entries_it_cannot_read() {
        for bad in ["EN:input", "3=:input", "=EN:input", "3=EN", "3=EN:ground"] {
            let refused = read(&text(bad), 0);
            assert!(
                refused.as_ref().is_err_and(|p| p.code == "param.pin_table"),
                "{bad}: {refused:?}"
            );
        }
    }
}
