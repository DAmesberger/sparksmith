//! The one per-instance assembly decision (review, "Architecture": which
//! part will be fitted). Purchasing, population and placement are three
//! answers kept apart, each from the one place that decides it:
//!
//! - **population**: whether the part is fitted at assembly, under the
//!   active variant (`instances::variants::fitted`);
//! - **purchasing**: whether it is listed on the bill of materials (its
//!   component belongs there and the instance is not excluded from it), and
//!   the Part bought for it with its purchasing state: the variant's
//!   substitute, resolved only after its review, else the instance's own;
//! - **placement**: whether it has a pick-and-place row (fitted, on the
//!   board with a footprint, its component placed by machine and the
//!   instance not excluded from the position files).
//!
//! The BOM, the pick-and-place and BOM files, the firmware device identity
//! and the variant report read this; none decides it again.
use super::purchasing::{PurchasingReport, PurchasingState, purchasing};
use crate::instances::{self, variants::Fitted};
use crate::model::{ComponentInstance, Population, Project};

/// What the active variant does with one placed part.
#[derive(Clone)]
pub struct Assembly<'a> {
    /// The part fitted: the population, the Part, the component revision
    /// and the physical implementation.
    pub fitted: Fitted<'a>,
    /// Listed on the bill of materials, fitted or not.
    pub purchasing: bool,
    /// The Part bought for it, with its purchasing state.
    pub purchase: PurchasingReport,
    /// Its placement on the board, if it has one, is a pick-and-place row.
    pub placement: bool,
}

impl Assembly<'_> {
    pub fn population(&self) -> Population {
        self.fitted.population
    }

    pub fn populated(&self) -> bool {
        self.fitted.populated()
    }
}

/// The assembly decision for `instance` under the active variant.
pub fn assembly<'a>(project: &'a Project, instance: &ComponentInstance) -> Assembly<'a> {
    let library = &project.library;
    let fitted = instances::variants::fitted(project, instance);
    let purchase = if fitted.substituted {
        let part = fitted.part.and_then(|r| library.part(r));
        // Fitting the lands is not buying the right part: the substitute is
        // resolved only once its electrical equivalence is reviewed and the
        // instance's requirements are met or deliberately excepted.
        let findings = match (project.variants.active(), fitted.part) {
            (Some(variant), Some(r)) => {
                instances::variants::substitute_findings(library, variant, instance, r)
            }
            _ => Vec::new(),
        };
        PurchasingReport {
            state: if findings.is_empty() {
                PurchasingState::Resolved
            } else {
                PurchasingState::Unresolved
            },
            part: fitted.part,
            manufacturer: part.map(|p| p.definition.manufacturer.clone()),
            mpn: part.map(|p| p.definition.mpn.clone()),
            findings,
        }
    } else {
        purchasing(project, instance)
    };
    Assembly {
        purchasing: instances::on_bom(library, instance),
        placement: fitted.populated()
            && fitted.physical.is_some()
            && instances::in_placement(library, instance),
        fitted,
        purchase,
    }
}
