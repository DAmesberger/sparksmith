//! Parameters checked against a description: the one check both the host
//! (before it calls and hashes) and the module (before it generates) run.
#[cfg(feature = "complete")]
use crate::runtime::Failure;
use crate::v1::{Description, Param, ParamType, ParamValue, Params, Problem};
#[cfg(feature = "complete")]
use crate::v1::{Request, Scope};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// A problem with `field`.
pub fn problem(code: &str, field: &str, message: String) -> Problem {
    Problem {
        code: code.into(),
        field: Some(field.into()),
        message,
    }
}

/// A length in mm for messages, e.g. `0.45 mm`, without floats.
pub fn mm_text(nm: i64) -> String {
    let sign = if nm < 0 { "-" } else { "" };
    let abs = nm.unsigned_abs();
    let whole = abs / 1_000_000;
    let frac = abs % 1_000_000;
    let mut digits = format!("{frac:06}");
    while digits.len() > 2 && digits.ends_with('0') {
        digits.pop();
    }
    format!("{sign}{whole}.{digits} mm")
}

/// The canonical request for `given` parameters in `scope`: every
/// parameter `given` leaves out takes the description's default, then the
/// whole is checked ([`check`]). Its canonical bytes are what a generated
/// part's params hash covers, so the host and the core both complete here
/// and a default left out or given makes the same part. A host fills
/// project-context parameters into `given` first. Refuses a scope the
/// generator does not offer, unknown parameters and values of the wrong
/// type or out of range, all at once. Host side only (feature `complete`):
/// with it in a module, the module's bytes change.
#[cfg(feature = "complete")]
pub fn complete(
    description: &Description,
    scope: Scope,
    given: &Params,
) -> Result<Request, Failure> {
    if !description.scopes().contains(&scope) {
        let what = match scope {
            Scope::Component => "complete parts",
            Scope::Package => "packages on their own",
        };
        return Err(Failure::one(
            "param.scope",
            format!("The {} generator does not make {what}.", description.title),
        ));
    }
    let mut params = given.clone();
    for param in description.params() {
        params
            .entry(param.key.clone())
            .or_insert_with(|| param.default.clone());
    }
    check(description, &params).map_err(|problems| Failure { problems })?;
    Ok(Request {
        abi: crate::v1::ABI,
        scope,
        params,
    })
}

/// Every declared parameter present, of its type and in its range, and
/// nothing else; otherwise every problem found, in plain words.
pub fn check(description: &Description, params: &Params) -> Result<(), Vec<Problem>> {
    let mut problems = Vec::new();
    for key in params.keys() {
        if !description.params().iter().any(|p| &p.key == key) {
            problems.push(problem(
                "param.unknown",
                key,
                format!("This generator has no parameter \"{key}\"."),
            ));
        }
    }
    for param in description.params() {
        match params.get(&param.key) {
            None => problems.push(problem(
                "param.missing",
                &param.key,
                format!(
                    "{} is missing; the request must give every parameter.",
                    param.label
                ),
            )),
            Some(value) => {
                if let Err(p) = check_value(param, &param.ty, value) {
                    problems.push(p);
                }
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

/// Whether `value` is of type `ty` and in its range.
pub fn fits(ty: &ParamType, value: &ParamValue) -> bool {
    let param = Param {
        key: String::new(),
        label: String::new(),
        help: None,
        ty: ty.clone(),
        default: value.clone(),
        group: None,
        context: None,
    };
    check_value(&param, ty, value).is_ok()
}

fn check_value(param: &Param, ty: &ParamType, value: &ParamValue) -> Result<(), Problem> {
    let label = &param.label;
    let key = &param.key;
    let wrong = |kind: &str| problem("param.type", key, format!("{label} must be {kind}."));
    match (ty, value) {
        (
            ParamType::Length {
                min_nm,
                max_nm,
                step_nm,
            },
            ParamValue::Int(v),
        ) => {
            if v < min_nm || v > max_nm {
                return Err(problem(
                    "param.out_of_range",
                    key,
                    format!(
                        "{label} {} is outside this generator's range, {} to {}.",
                        mm_text(*v),
                        mm_text(*min_nm),
                        mm_text(*max_nm)
                    ),
                ));
            }
            if let Some(step) = step_nm
                && *step > 0
                && v.checked_rem(*step) != Some(0)
            {
                return Err(problem(
                    "param.out_of_range",
                    key,
                    format!("{label} must be a multiple of {}.", mm_text(*step)),
                ));
            }
            Ok(())
        }
        (
            ParamType::Angle {
                min_mdeg: min,
                max_mdeg: max,
            },
            ParamValue::Int(v),
        )
        | (ParamType::Count { min, max }, ParamValue::Int(v)) => {
            if v < min || v > max {
                Err(problem(
                    "param.out_of_range",
                    key,
                    format!("{label} {v} is outside this generator's range, {min} to {max}."),
                ))
            } else {
                Ok(())
            }
        }
        (ParamType::Enum { choices }, ParamValue::Text(v)) => {
            if choices.iter().any(|c| &c.key == v) {
                Ok(())
            } else {
                let keys: Vec<&str> = choices.iter().map(|c| c.key.as_str()).collect();
                Err(problem(
                    "param.out_of_range",
                    key,
                    format!("{label} \"{v}\" is not one of: {}.", keys.join(", ")),
                ))
            }
        }
        (ParamType::Bool, ParamValue::Bool(_)) => Ok(()),
        (ParamType::Text { max_chars }, ParamValue::Text(v)) => {
            if v.chars().count() > *max_chars as usize {
                Err(problem(
                    "param.out_of_range",
                    key,
                    format!("{label} is longer than {max_chars} characters."),
                ))
            } else {
                Ok(())
            }
        }
        (ParamType::Quantity { .. }, ParamValue::Text(_)) => Ok(()),
        (ParamType::List { of, max_len }, ParamValue::List(items)) => {
            if items.len() > *max_len as usize {
                return Err(problem(
                    "param.out_of_range",
                    key,
                    format!("{label} has more than {max_len} entries."),
                ));
            }
            items
                .iter()
                .try_for_each(|item| check_value(param, of, item))
        }
        (ParamType::Length { .. }, _) => Err(wrong("a length in nm")),
        (ParamType::Angle { .. }, _) => Err(wrong("an angle in millidegrees")),
        (ParamType::Count { .. }, _) => Err(wrong("a whole number")),
        (ParamType::Enum { .. }, _) => Err(wrong("one of its listed choices")),
        (ParamType::Bool, _) => Err(wrong("true or false")),
        (ParamType::Text { .. }, _) => Err(wrong("text")),
        (ParamType::Quantity { .. }, _) => Err(wrong("a decimal quantity as text")),
        (ParamType::List { .. }, _) => Err(wrong("a list")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Completion is host side; the workspace's tests turn it on.
    #[cfg(feature = "complete")]
    mod completion {
        use super::*;
        use crate::v1::{ABI, Capability, Effect, GENERATE_PART, PartOffer, Scope};
        use crate::{codec, runtime::Failure};
        use alloc::vec;

        fn param(key: &str, ty: ParamType, default: ParamValue) -> Param {
            Param {
                key: key.into(),
                label: key.into(),
                help: None,
                ty,
                default,
                group: None,
                context: None,
            }
        }

        /// A part generator of two parameters that makes packages only.
        fn description() -> Description {
            Description {
                abi: ABI,
                id: "fixture".into(),
                version: 1,
                title: "Fixture".into(),
                licence: "MIT OR Apache-2.0".into(),
                deterministic: true,
                capabilities: vec![Capability {
                    name: GENERATE_PART.into(),
                    effect: Effect::None,
                    title: None,
                    params: vec![
                        param(
                            "length",
                            ParamType::Length {
                                min_nm: 100_000,
                                max_nm: 10_000_000,
                                step_nm: None,
                            },
                            ParamValue::Int(1_600_000),
                        ),
                        param("polarised", ParamType::Bool, ParamValue::Bool(false)),
                    ],
                    rules: Vec::new(),
                    presets: Vec::new(),
                    part: Some(PartOffer {
                        families: vec!["chip".into()],
                        scopes: vec![Scope::Package],
                        grammar: Vec::new(),
                        instance_fields: Vec::new(),
                    }),
                }],
                needs: Vec::new(),
            }
        }

        fn bytes(given: &[(&str, ParamValue)]) -> Result<Vec<u8>, Failure> {
            let given: Params = given
                .iter()
                .map(|(key, value)| ((*key).into(), value.clone()))
                .collect();
            let request = complete(&description(), Scope::Package, &given)?;
            codec::encode(&request).map_err(|e| Failure::one("codec", format!("{e:?}")))
        }

        /// The bytes that hash to a part's identity are the same whether a
        /// default is left out or given, in any order.
        #[test]
        fn implicit_and_explicit_defaults_complete_to_the_same_bytes() -> Result<(), Failure> {
            let implicit = bytes(&[])?;
            let explicit = bytes(&[
                ("polarised", ParamValue::Bool(false)),
                ("length", ParamValue::Int(1_600_000)),
            ])?;
            assert_eq!(implicit, explicit);
            let partial = bytes(&[("length", ParamValue::Int(1_600_000))])?;
            assert_eq!(implicit, partial);
            assert_eq!(
                core::str::from_utf8(&implicit).ok(),
                Some(
                    r#"{"abi":1,"params":{"length":1600000,"polarised":false},"scope":"package"}"#
                )
            );
            // A value that differs from the default is kept.
            assert_ne!(implicit, bytes(&[("length", ParamValue::Int(2_000_000))])?);
            Ok(())
        }

        #[test]
        fn completion_refuses_an_unoffered_scope_and_unknown_or_bad_values() {
            let code = |r: Result<Request, Failure>| r.err().and_then(|f| f.code().map(Into::into));
            let d = description();
            assert_eq!(
                code(complete(&d, Scope::Component, &Params::new())),
                Some(String::from("param.scope"))
            );
            let mut unknown = Params::new();
            unknown.insert("no_such".into(), ParamValue::Int(1));
            assert_eq!(
                code(complete(&d, Scope::Package, &unknown)),
                Some(String::from("param.unknown"))
            );
            let mut wide = Params::new();
            wide.insert("length".into(), ParamValue::Int(1));
            assert!(complete(&d, Scope::Package, &wide).is_err());
        }
    }

    #[test]
    fn writes_mm_without_floats() {
        assert_eq!(mm_text(450_000), "0.45 mm");
        assert_eq!(mm_text(1_600_000), "1.60 mm");
        assert_eq!(mm_text(-50_000), "-0.05 mm");
        assert_eq!(mm_text(1_234_567), "1.234567 mm");
    }
}
