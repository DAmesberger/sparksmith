//! Slice C7: the `library.models` query on the wire (D1).
use super::*;

type Fallible<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn the_model_query_answers_on_the_wire() -> Fallible<()> {
    let mut session = Session::new(Engine::demo()?);
    let reply = session.dispatch(json!({"command": "project.query",
        "params": {"query": "library.models", "params": {"query": "kind:model mount:smd"}}}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert!(reply["result"]["result"]["hits"].is_array(), "{reply}");
    let reply = session.dispatch(json!({"command": "project.query",
        "params": {"query": "library.models", "params": {"query": "kind:part"}}}));
    assert_eq!(reply["ok"], false, "{reply}");
    assert_eq!(reply["error"]["code"], "invalid_input", "{reply}");
    Ok(())
}
