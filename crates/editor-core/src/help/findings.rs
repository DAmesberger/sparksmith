//! Declaration facts. Authored prose is attached by the help loader.
use super::{Facts, HelpEntry};
use crate::diagnostics::FindingCode;

pub fn finding_entries() -> Result<Vec<HelpEntry>, String> {
    FindingCode::ALL.iter().map(|code| {
        let title = code.group_title().to_owned();
        Ok(HelpEntry {
            id: format!("finding/{code}").parse()?, title: title.clone(), summary: title.clone(),
            since: "1.0".into(),
            facts: Facts::Finding { severity: code.severity(), blocks: code.blocks(), group: title,
                cause: String::new(), fix: String::new(), explanation: None, fixed_by: vec![], checks: None, rule_key: None },
            body: vec![], related: vec![], keywords: vec![],
        })
    }).collect()
}
