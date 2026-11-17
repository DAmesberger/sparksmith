//! Original synthetic project JSON, refused before any rules can be truncated.
mod support;

#[test]
fn import_refuses_more_than_65536_project_patterns() -> Result<(), Box<dyn std::error::Error>> {
    let _context = sparksmith_core::library::standards::Context::fixture()?.enter();
    let board = support::Board {
        copper: 2,
        origin: None,
        setup: String::new(),
        footprints: Vec::new(),
        items: String::new(),
    };
    let project = serde_json::to_vec(&serde_json::json!({
        "net_settings": {"netclass_patterns": vec![serde_json::Value::Null; 65_537]}
    }))?;
    let result = support::import_with_project(board.text().as_bytes(), Some(&project));
    let Err(error) = result else {
        return Err("an oversized pattern list was silently imported".into());
    };
    assert!(error.to_string().contains("65536"), "{error}");
    assert!(error.to_string().contains("patterns"), "{error}");
    Ok(())
}

fn collection(section: &str, count: usize) -> serde_json::Value {
    use serde_json::{Value, json};
    match section {
        "classes" => {
            json!({"net_settings": {"classes": vec![json!({"name": "Synthetic"}); count]}})
        }
        "patterns" => {
            json!({"net_settings": {"netclass_patterns": vec![json!({"pattern": "N*", "netclass": "Synthetic"}); count]}})
        }
        "assignments" => {
            let entries: serde_json::Map<String, Value> = (0..count)
                .map(|n| (format!("N{n}"), json!("Synthetic")))
                .collect();
            json!({"net_settings": {"netclass_assignments": entries}})
        }
        _ => {
            let entries: serde_json::Map<String, Value> = (0..count)
                .map(|n| (format!("rule{n}"), json!(0.2)))
                .collect();
            json!({"board": {"design_settings": {"rules": entries}}})
        }
    }
}

#[test]
fn project_collection_limits_accept_the_boundary_and_refuse_the_first_excess()
-> Result<(), Box<dyn std::error::Error>> {
    use sparksmith_kicad::project::{ReadError, read};
    for section in ["classes", "patterns", "assignments", "rules"] {
        let accepted = read(&serde_json::to_vec(&collection(section, 65_536))?)?;
        match section {
            "classes" => assert_eq!(
                accepted.classes.len() + usize::from(accepted.default.is_some()),
                65_536
            ),
            "patterns" => assert_eq!(accepted.patterns.len(), 65_536),
            "assignments" => assert_eq!(accepted.assignments.len(), 65_536),
            _ => {}
        }
        let refused = read(&serde_json::to_vec(&collection(section, 65_537))?);
        assert_eq!(
            refused,
            Err(ReadError::Limit {
                section,
                limit: 65_536
            })
        );
    }
    Ok(())
}

#[test]
fn project_byte_limit_accepts_exactly_16_mib_and_refuses_more()
-> Result<(), Box<dyn std::error::Error>> {
    use sparksmith_kicad::project::{ReadError, read};
    let mut input = vec![b' '; 16 * 1024 * 1024];
    input[..2].copy_from_slice(b"{}");
    read(&input)?;
    input.push(b' ');
    assert_eq!(
        read(&input),
        Err(ReadError::Limit {
            section: "bytes",
            limit: 16 * 1024 * 1024
        })
    );
    Ok(())
}
