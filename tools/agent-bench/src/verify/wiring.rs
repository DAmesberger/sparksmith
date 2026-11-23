//! Independently resolve every saved net member through its placed component definition.
use crate::Result;
use serde::Deserialize;
use sparksmith_core::Project;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wiring {
    pub nets: BTreeMap<String, Vec<[String; 2]>>,
}

impl Wiring {
    pub fn verify(&self, project: &Project) -> Result<()> {
        let mut actual = BTreeMap::new();
        for net in project.nets.iter() {
            let mut members = BTreeSet::new();
            for endpoint in &net.terminals {
                let instance = project
                    .instance(endpoint.instance)
                    .ok_or("net refers to an unknown instance")?;
                let resolved = sparksmith_core::instances::resolve(&project.library, instance)?;
                let pin = resolved
                    .component
                    .definition
                    .terminal(endpoint.terminal)
                    .ok_or("net refers to an unknown terminal")?;
                if !members.insert([instance.designator.clone(), pin.name.clone()]) {
                    return Err(format!("duplicate saved member on {}", net.name).into());
                }
            }
            if actual.insert(net.name.clone(), members).is_some() {
                return Err(format!("duplicate saved net {}", net.name).into());
            }
        }
        let expected: BTreeMap<_, BTreeSet<_>> = self
            .nets
            .iter()
            .map(|(name, members)| (name.clone(), members.iter().cloned().collect()))
            .collect();
        if actual != expected {
            return Err(format!(
                "saved subcircuit wiring differs: expected {expected:?}, found {actual:?}"
            )
            .into());
        }
        Ok(())
    }
}
