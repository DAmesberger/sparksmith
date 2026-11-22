//! A part's orderable part numbers: every generator's `part_numbers`
//! parameter.
//!
//! A generator makes a package and a generic part; which manufacturer part
//! number that part is, the maker's ordering information says. The
//! `part_numbers` parameter carries it as data, one `MANUFACTURER=MPN` per
//! entry: a preset of a particular part gives its datasheet's ordering
//! codes, and the datasheet workflow gives the part number it was asked
//! for. A complete part returns them as its `orderable` parts, which the
//! host publishes as the component's manufacturer Parts, so the BOM can
//! name the part once one is chosen. A package has none.
//!
//! The SDK declares the parameter for every generator ([`declare`]) and
//! applies it around the generator's own `generate` ([`generate`]), as it
//! does the land pattern and the pin table.
use crate::Generator;
use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use sparksmith_generator_abi::v1::{
    Description, GENERATE_PART, Generation, Orderable, Param, ParamType, ParamValue, Problem,
    Request, limits,
};

/// The parameter's key.
pub const KEY: &str = "part_numbers";

/// The `part_numbers` parameter: at most [`limits::ORDERABLE`] entries,
/// none by default.
pub fn param() -> Param {
    Param {
        key: KEY.into(),
        label: "Orderable part numbers".into(),
        help: Some(
            "The manufacturer part numbers this part is exactly, one MANUFACTURER=MPN per entry, as the maker's ordering information prints them (Texas Instruments=TLV75733PDBVR); each becomes a manufacturer Part of the component, for the BOM. Empty: a generic part."
                .into(),
        ),
        ty: ParamType::List {
            of: Box::new(ParamType::Text {
                max_chars: 2 * limits::ORDERABLE_BYTES as u32 + 1,
            }),
            max_len: limits::ORDERABLE as u32,
        },
        default: ParamValue::List(Vec::new()),
        group: Some("ordering".into()),
        context: None,
    }
}

/// `description` with the `part_numbers` parameter added to its part
/// capability (unless it declares one itself).
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
        code: "param.part_numbers".into(),
        field: Some(KEY.into()),
        message,
    }
}

/// Run `G` for `request` with the pin table and the land pattern applied:
/// the `part_numbers` parameter is taken out of the request, the part is
/// made, and a complete part returns the part numbers as its orderable
/// parts.
pub fn generate<G: Generator>(request: &Request) -> Result<Generation, Vec<Problem>> {
    let declared_by_generator = G::describe().params().iter().any(|p| p.key == KEY);
    if declared_by_generator {
        return crate::pins::generate::<G>(request);
    }
    let mut own = request.clone();
    let items = match own.params.remove(KEY) {
        None => Vec::new(),
        Some(ParamValue::List(items)) => items,
        Some(_) => {
            return Err(alloc::vec![problem(
                "The part numbers are not a list.".into()
            )]);
        }
    };
    let orderable = read_all(&items).map_err(|p| alloc::vec![p])?;
    let mut generation = crate::pins::generate::<G>(&own)?;
    if generation.component.is_some() {
        generation.orderable = orderable;
    }
    Ok(generation)
}

/// Read every entry (`MANUFACTURER=MPN`, both sides trimmed; the part
/// number ends the entry after the last `=`), each part number once.
pub fn read_all(items: &[ParamValue]) -> Result<Vec<Orderable>, Problem> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let ParamValue::Text(entry) = item else {
            return Err(problem("A part number entry is not text.".into()));
        };
        let pair = entry
            .rsplit_once('=')
            .map(|(maker, mpn)| (maker.trim(), mpn.trim()))
            .filter(|(maker, mpn)| !maker.is_empty() && !mpn.is_empty());
        let Some((manufacturer, mpn)) = pair else {
            return Err(problem(format!(
                "\"{entry}\" is not a part number entry: write the manufacturer, \"=\" and the complete part number, as in Texas Instruments=TLV75733PDBVR."
            )));
        };
        if manufacturer.len() > limits::ORDERABLE_BYTES || mpn.len() > limits::ORDERABLE_BYTES {
            return Err(problem(format!(
                "\"{entry}\" is longer than a manufacturer or part number may be ({} bytes each).",
                limits::ORDERABLE_BYTES
            )));
        }
        if manufacturer
            .chars()
            .chain(mpn.chars())
            .any(char::is_control)
        {
            return Err(problem(format!("\"{entry}\" holds a control character.")));
        }
        if !seen.insert(String::from(mpn)) {
            return Err(problem(format!("The part number {mpn} is listed twice.")));
        }
        out.push(Orderable {
            manufacturer: manufacturer.into(),
            mpn: mpn.into(),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(entries: &[&str]) -> Vec<ParamValue> {
        entries
            .iter()
            .map(|e| ParamValue::Text((*e).into()))
            .collect()
    }

    #[test]
    fn reads_manufacturer_and_part_number() -> Result<(), Problem> {
        let read = read_all(&table(&[
            " Texas Instruments = TLV75733PDBVR ",
            "Espressif Systems=ESP32-S3-WROOM-1-N16R8",
        ]))?;
        assert_eq!(
            read,
            [
                Orderable {
                    manufacturer: "Texas Instruments".into(),
                    mpn: "TLV75733PDBVR".into(),
                },
                Orderable {
                    manufacturer: "Espressif Systems".into(),
                    mpn: "ESP32-S3-WROOM-1-N16R8".into(),
                },
            ]
        );
        Ok(())
    }

    #[test]
    fn refuses_entries_it_cannot_read() {
        for bad in [
            &["TLV75733PDBVR"][..],
            &["=TLV75733PDBVR"],
            &["Texas Instruments="],
            &["A=X", "B=X"],
        ] {
            let refused = read_all(&table(bad));
            assert!(
                refused
                    .as_ref()
                    .is_err_and(|p| p.code == "param.part_numbers"),
                "{bad:?}: {refused:?}"
            );
        }
    }
}
