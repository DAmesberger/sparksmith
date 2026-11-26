//! The version this build shows: a package's commit, or the checkout's
//! commit `build.rs` read (run 4, B2: About said only "built from source").

/// "0.1.0 (1a2b3c4d5e)" for a package, "0.1.0 (built from source at
/// 1a2b3c4d5e)" for a build from a checkout.
pub fn version() -> String {
    sparksmith_core::product::version_from(option_env!("SPARKSMITH_SOURCE_COMMIT"))
}
