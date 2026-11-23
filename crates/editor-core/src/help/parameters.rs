//! Human-readable parameter ranges and defaults shared by both manual renderers.
pub fn parameter_type(ty: &sparksmith_generator_abi::v1::ParamType) -> String {
    use sparksmith_generator_abi::v1::ParamType::*;
    match ty {
        Length { min_nm, max_nm, .. } => format!("Length, {min_nm}–{max_nm} nm"),
        Angle { min_mdeg, max_mdeg } => format!("Angle, {min_mdeg}–{max_mdeg} m°"),
        Count { min, max } => format!("Count, {min}–{max}"),
        Enum { choices } => choices
            .iter()
            .map(|c| c.label.as_str())
            .collect::<Vec<_>>()
            .join(" / "),
        Bool => "On or off".into(),
        Text { max_chars } => format!("Text, up to {max_chars} characters"),
        Quantity {
            kind,
            unit,
            min,
            max,
        } => {
            let mut s = format!("{kind} ({unit})");
            if let Some(min) = min {
                s.push_str(&format!(", minimum {min}"));
            }
            if let Some(max) = max {
                s.push_str(&format!(", maximum {max}"));
            }
            s
        }
        List { of, max_len } => format!("Up to {max_len} items · {}", parameter_type(of)),
    }
}
pub fn parameter_value(
    ty: &sparksmith_generator_abi::v1::ParamType,
    value: &sparksmith_generator_abi::v1::ParamValue,
) -> serde_json::Result<String> {
    use sparksmith_generator_abi::v1::{ParamType, ParamValue};
    Ok(match (ty, value) {
        (ParamType::Enum { choices }, ParamValue::Text(key)) => choices
            .iter()
            .find(|c| c.key == *key)
            .map_or(key.as_str(), |c| c.label.as_str())
            .into(),
        (ParamType::Length { .. }, ParamValue::Int(n)) => format!("{n} nm"),
        (ParamType::Angle { .. }, ParamValue::Int(n)) => format!("{n} m°"),
        (_, ParamValue::Text(s)) => s.clone(),
        _ => serde_json::to_string(value)?,
    })
}
