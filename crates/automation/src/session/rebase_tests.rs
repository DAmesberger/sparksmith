//! An agent's edit at a revision the person has moved on from: applied on
//! top of their edits when the two touch different objects, refused naming
//! the objects when they touch the same, and as strict as before for undo
//! and for a revision the session never answered at.
use super::*;
use crate::front::{Call, CallCx, NoOne, Surface};
use crate::gate::{Grant, Mode, Principal};
use sparksmith_core::Engine;

type Fallible<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn call(entry: &str, params: Value, expected_revision: u64) -> Call {
    Call {
        entry: entry.to_owned(),
        params,
        project: None,
        expected_revision: Some(expected_revision),
        dry_run: false,
        confirm: None,
        turn: None,
        job: false,
    }
}

fn run(session: &mut Session, who: Principal<'_>, call: Call) -> Result<Value, Box<Error>> {
    session.call(
        CallCx {
            who,
            asker: &mut NoOne,
            verbose: false,
            window: None,
        },
        call,
    )
}

fn add_via(session: &Session, x: f64) -> Value {
    let net = session.engine().project().nets.first().map_or(0, |n| n.id);
    json!({"via": {"net": net, "position": [x, 1.0], "drill": 0.3, "diameter": 0.6,
        "span": "through", "tented": true}})
}

/// The id of the via a reply added.
fn added_via(reply: &Value) -> Fallible<u64> {
    reply
        .pointer("/changes/added/vias/0")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("no via added: {reply}").into())
}

fn failed(error: Box<Error>) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

#[test]
fn an_agents_stale_edit_goes_on_top_of_the_persons_unless_both_change_one_object() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let grant = Grant::new(Mode::AllowAll, []);
    let agent = Principal::LocalAgent { grant: &grant };
    // The agent adds a via and reads the revision its reply names.
    let start = session.engine().revision();
    let params = add_via(&session, 1.0);
    let first = run(&mut session, agent, call("layout.add_via", params, start)).map_err(failed)?;
    let seen = session.engine().revision();
    let mine = added_via(&first)?;
    // The person adds a via of their own meanwhile.
    let params = add_via(&session, 3.0);
    let theirs = run(
        &mut session,
        Principal::Person,
        call("layout.add_via", params, seen),
    )
    .map_err(failed)?;
    let theirs = added_via(&theirs)?;
    let after_person = session.engine().revision();
    assert!(after_person > seen);
    // The agent moves its own via, still at the revision it saw: nothing
    // of the person's is touched, so it applies on top.
    let moved = run(
        &mut session,
        agent,
        call(
            "layout.via_move",
            json!({"id": mine, "position": [2.0, 2.0]}),
            seen,
        ),
    )
    .map_err(failed)?;
    assert_eq!(
        moved["rebased"],
        json!({"from": seen, "onto": after_person}),
        "{moved}"
    );
    let project = session.engine().project();
    let via = |id: u64| project.vias.iter().find(|v| v.id == id);
    assert!(via(theirs).is_some(), "the person's via stays");
    assert_eq!(via(mine).map(|v| v.position), Some([2.0, 2.0]));
    // The person moves their via; the agent, at the revision before that,
    // moves the same via: both change it, so it is refused, naming it,
    // and nothing changes.
    let before_move = session.engine().revision();
    run(
        &mut session,
        Principal::Person,
        call(
            "layout.via_move",
            json!({"id": theirs, "position": [4.0, 4.0]}),
            before_move,
        ),
    )
    .map_err(failed)?;
    let now = session.engine().revision();
    let file = session.engine().to_file();
    let Err(conflict) = run(
        &mut session,
        agent,
        call(
            "layout.via_move",
            json!({"id": theirs, "position": [5.0, 5.0]}),
            before_move,
        ),
    ) else {
        return Err("a conflicting edit ran".into());
    };
    assert_eq!(conflict.code, "revision.conflict", "{conflict:?}");
    assert!(
        conflict.message.contains(&format!("vias {theirs}")),
        "{}",
        conflict.message
    );
    assert_eq!(conflict.expected_revision, Some(before_move));
    assert_eq!(conflict.actual_revision, Some(now));
    assert_eq!(session.engine().revision(), now);
    assert_eq!(session.engine().to_file(), file);
    Ok(())
}

/// The person's edit straight on the engine, as the window makes it: the
/// session never answers at its revision.
fn edit_directly(session: &mut Session, entry: &str, params: Value) -> Fallible {
    let Some(crate::session::discovery::Call::Operation(value)) =
        crate::session::discovery::call(entry, params)
    else {
        return Err(format!("{entry} is no operation").into());
    };
    let operation: sparksmith_core::Operation = serde_json::from_value(value)?;
    let revision = session.engine().revision();
    session.engine_mut().apply(revision, operation)?;
    Ok(())
}

#[test]
fn undo_and_unseen_revisions_stay_strict() -> Fallible {
    let mut session = Session::new(Engine::demo()?);
    let grant = Grant::new(Mode::AllowAll, []);
    let agent = Principal::LocalAgent { grant: &grant };
    let start = session.engine().revision();
    let params = add_via(&session, 1.0);
    run(&mut session, agent, call("layout.add_via", params, start)).map_err(failed)?;
    let seen = session.engine().revision();
    // The person makes two edits in the window.
    let params = add_via(&session, 3.0);
    edit_directly(&mut session, "layout.add_via", params)?;
    let unseen = session.engine().revision();
    let params = add_via(&session, 4.0);
    edit_directly(&mut session, "layout.add_via", params)?;
    // Undo at the agent's revision would undo the person's edits: refused.
    let Err(undo) = run(&mut session, agent, call("project.undo", json!({}), seen)) else {
        return Err("a stale undo ran".into());
    };
    assert_eq!(undo.code, "stale_revision", "{undo:?}");
    // A revision this session never answered at is refused as before.
    let params = add_via(&session, 5.0);
    let Err(refused) = run(&mut session, agent, call("layout.add_via", params, unseen)) else {
        return Err("an edit at an unseen revision ran".into());
    };
    assert_eq!(refused.code, "stale_revision", "{refused:?}");
    // The person's own calls are never rebased.
    let params = add_via(&session, 6.0);
    let Err(person) = run(
        &mut session,
        Principal::Person,
        call("layout.add_via", params, seen),
    ) else {
        return Err("the person's stale edit ran".into());
    };
    assert_eq!(person.code, "stale_revision", "{person:?}");
    // At the revision it saw, the agent's edit goes on top of both.
    let vias = session.engine().project().vias.len();
    let params = add_via(&session, 7.0);
    let rebased = run(&mut session, agent, call("layout.add_via", params, seen)).map_err(failed)?;
    assert_eq!(rebased["rebased"]["from"], json!(seen), "{rebased}");
    assert_eq!(session.engine().project().vias.len(), vias + 1);
    Ok(())
}
