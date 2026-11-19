//! Interpreter regressions: small modules whose results the pinned wasmi
//! once got wrong, run on the host's own engine configuration.
//!
//! wasmi 2.0.0 fused a `select` with a zero test of a local (`i32.eqz`,
//! `i32.eq`/`i32.ne` against 0) and then read a stale register as the
//! condition (wasmi issue 2046, fixed in 2.0.1). Rust's
//! `if key.starts_with("row") { "rows" } else { "single pads" }` compiles
//! to such a `select` of the string's address, so the module generator's
//! row lists came out of the shipped module grouped as "sing": the address
//! of "single pads" with the length of "rows".
use sparksmith_generators::config;
use wasmi::{Engine, Instance, Linker, Module, Store};

type TestResult = Result<(), String>;

fn instance(wat: &str) -> Result<(Store<()>, Instance), String> {
    let mut config = config();
    // The host meters fuel; a test call needs no limit beyond enough.
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let bytes = wat::parse_str(wat).map_err(|e| e.to_string())?;
    let module = Module::new(&engine, &bytes).map_err(|e| e.to_string())?;
    let mut store = Store::new(&engine, ());
    store.set_fuel(1_000_000).map_err(|e| e.to_string())?;
    let instance = Linker::new(&engine)
        .instantiate_and_start(&mut store, &module)
        .map_err(|e| e.to_string())?;
    Ok((store, instance))
}

/// `select(10, 20, cond(x))` for each zero test of the parameter, the
/// shape `starts_with` compiles to.
#[test]
fn select_on_a_zero_test_of_a_local_reads_the_local() -> TestResult {
    let (mut store, instance) = instance(
        r#"(module
            (func (export "eqz") (param i32) (result i32)
                i32.const 10 i32.const 20 local.get 0 i32.eqz select)
            (func (export "eq0") (param i32) (result i32)
                i32.const 10 i32.const 20 local.get 0 i32.const 0 i32.eq select)
            (func (export "ne0") (param i32) (result i32)
                i32.const 10 i32.const 20 local.get 0 i32.const 0 i32.ne select)
            (func (export "zero_ne") (param i32) (result i32)
                i32.const 10 i32.const 20 i32.const 0 local.get 0 i32.ne select))"#,
    )?;
    for x in [0_i32, 1, -1, 9] {
        let zero = x == 0;
        for (name, expected) in [
            ("eqz", if zero { 10 } else { 20 }),
            ("eq0", if zero { 10 } else { 20 }),
            ("ne0", if zero { 20 } else { 10 }),
            ("zero_ne", if zero { 20 } else { 10 }),
        ] {
            let func = instance
                .get_typed_func::<i32, i32>(&store, name)
                .map_err(|e| e.to_string())?;
            let got = func.call(&mut store, x).map_err(|e| e.to_string())?;
            if got != expected {
                return Err(format!("{name}({x}) gave {got}, expected {expected}"));
            }
        }
    }
    Ok(())
}
