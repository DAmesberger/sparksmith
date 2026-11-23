//! A designer's sourced current table. No curve or formula ships in the program.
use crate::diagnostics::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CurrentTable {
    pub edition: String,
    pub figure: String,
    pub source: String,
    pub rows: Vec<CurrentPoint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CurrentPoint {
    pub rise_c: f64,
    pub area_mm2: f64,
    pub amperes: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CurrentRule {
    pub rise_c: Option<f64>,
    pub table: Option<CurrentTable>,
}

impl CurrentRule {
    pub fn validate(&self) -> Result<()> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        if self.rise_c.is_some_and(|v| !positive(v)) {
            return Err(Error::Invalid(
                "Choose a positive allowed temperature rise.".into(),
            ));
        }
        if let Some(t) = &self.table {
            t.validate()?;
        }
        Ok(())
    }
    pub fn capacity(&self, area_mm2: f64) -> Option<f64> {
        self.table.as_ref()?.capacity(self.rise_c?, area_mm2)
    }
}
impl CurrentTable {
    pub fn validate(&self) -> Result<()> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        if [&self.edition, &self.figure, &self.source]
            .iter()
            .any(|s| s.trim().is_empty() || s.len() > super::MAX_PROFILE_NOTES)
        {
            return Err(Error::Invalid(
                "Cite the source, edition and figure of the current table.".into(),
            ));
        }
        if self.rows.is_empty() || self.rows.len() > 4096 {
            return Err(Error::Limit(
                "A current table needs between 1 and 4096 sourced points.".into(),
            ));
        }
        for p in &self.rows {
            if ![p.rise_c, p.area_mm2, p.amperes].into_iter().all(positive) {
                return Err(Error::Invalid(
                    "A current table needs positive finite values.".into(),
                ));
            }
        }
        let mut sorted = self.rows.clone();
        sorted.sort_by(|a, b| {
            a.rise_c
                .total_cmp(&b.rise_c)
                .then(a.area_mm2.total_cmp(&b.area_mm2))
        });
        if sorted.windows(2).any(|w| {
            w[0].rise_c == w[1].rise_c
                && (w[0].area_mm2 == w[1].area_mm2 || w[0].amperes > w[1].amperes)
        }) {
            return Err(Error::Invalid("At each temperature rise, areas must be distinct and current must increase or stay constant.".into()));
        }
        Ok(())
    }

    /// Use the highest tabulated cross section no larger than the bridge
    /// at the chosen rise. Never interpolate or extrapolate a current.
    pub fn capacity(&self, rise: f64, area_mm2: f64) -> Option<f64> {
        self.rows
            .iter()
            .filter(|p| p.rise_c == rise && p.area_mm2 <= area_mm2)
            .max_by(|a, b| a.area_mm2.total_cmp(&b.area_mm2))
            .map(|p| p.amperes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_private_table_is_required_and_only_its_points_are_used() -> Result<()> {
        let mut r = CurrentRule {
            rise_c: Some(7.0),
            table: None,
        };
        assert_eq!(r.capacity(0.2), None);
        r.table = Some(CurrentTable {
            source: "Synthetic test fixture".into(),
            edition: "test".into(),
            figure: "A".into(),
            rows: vec![
                CurrentPoint {
                    rise_c: 7.0,
                    area_mm2: 0.1,
                    amperes: 0.5,
                },
                CurrentPoint {
                    rise_c: 7.0,
                    area_mm2: 0.2,
                    amperes: 0.8,
                },
            ],
        });
        r.validate()?;
        assert_eq!(r.capacity(0.19), Some(0.5));
        assert_eq!(r.capacity(0.09), None);
        r.rise_c = Some(9.0);
        assert_eq!(r.capacity(0.2), None);
        Ok(())
    }
}
