//! Success replies for attached documents, pinout work, workflows, window views and turns.
use super::{Builder, object};
use crate::workflow::{
    ask::AskReply,
    entries::{OfferReply, StepResultReply, WorkflowsReply},
};
use serde_json::{Value, json};
use sparksmith_core::{
    diagnostics::Stage,
    functions::rules::RuleCheck,
    library::pinout::{ChangeKind, PlanBase, ReviewItem},
};
use std::collections::BTreeMap;

pub(super) fn document_text(cx: &mut Builder) -> Value {
    cx.typed::<super::super::assist::DocumentTextReply>()
}

pub(super) fn document_page(cx: &mut Builder) -> Value {
    cx.typed::<super::super::assist::DocumentPageReply>()
}

pub(super) fn pinout_edit(cx: &mut Builder) -> Value {
    cx.typed::<super::super::assist::PinoutEditReply>()
}

pub(super) fn working_preview(cx: &mut Builder) -> Value {
    compact_plan(cx)
}

pub(super) fn working_apply(cx: &mut Builder) -> Value {
    cx.edit()
}

pub(super) fn workflows(cx: &mut Builder) -> Value {
    cx.typed::<WorkflowsReply>()
}

pub(super) fn step_result(cx: &mut Builder) -> Value {
    cx.typed::<StepResultReply>()
}

pub(super) fn offer(cx: &mut Builder) -> Value {
    cx.typed::<OfferReply>()
}

pub(super) fn ask_person(cx: &mut Builder) -> Value {
    cx.typed::<AskReply>()
}

pub(super) fn view_context(cx: &mut Builder) -> Value {
    cx.typed::<crate::view::Context>()
}

pub(super) fn view_select(cx: &mut Builder) -> Value {
    cx.typed::<crate::view::Selection>()
}

pub(super) fn turn_begin(cx: &mut Builder) -> Value {
    cx.typed::<super::super::turns::Turn>()
}

/// The compact plan projects each change and finding down to the fields shown to the agent.
fn compact_plan(cx: &mut Builder) -> Value {
    let text = cx.typed::<String>();
    let finding = object(
        &[
            ("code", text.clone()),
            ("message", text.clone()),
            ("blocks", cx.typed::<Option<Stage>>()),
        ],
        &[],
    );
    let change = object(
        &[
            ("kind", cx.typed::<ChangeKind>()),
            ("object", text.clone()),
            ("row", cx.typed::<Option<String>>()),
            ("summary", text.clone()),
        ],
        &[],
    );
    object(
        &[
            ("handle", text),
            ("base", cx.typed::<PlanBase>()),
            ("change_counts", cx.typed::<BTreeMap<String, usize>>()),
            ("findings", json!({"type":"array", "items":finding})),
            ("review", cx.typed::<Vec<ReviewItem>>()),
            ("rules", cx.typed::<Vec<RuleCheck>>()),
        ],
        &[
            ("changes", json!({"type":"array", "items":change})),
            ("findings_not_listed", cx.typed::<usize>()),
            ("review_not_listed", cx.typed::<usize>()),
        ],
    )
}
