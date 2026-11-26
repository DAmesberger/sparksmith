//! The product's name, in one place. Every name a person sees (window and
//! menu titles, About, messages, the CLI's usage, the MCP server, generated
//! file headers, the per-user folders) comes from here, so a rename is the
//! one line in [`product_name!`] and [`product_short_name!`].
//!
//! The macros exist for text built at compile time (`concat!`, `const`
//! tables); everything else uses the constants. `apps/sparksmith/macos/Info.plist`
//! cannot read Rust, so a test checks its names against these.

/// The product name as people read it: "Sparksmith".
#[macro_export]
macro_rules! product_name {
    () => {
        "Sparksmith"
    };
}

/// The lower-case short name: the CLI binary, the MCP server name and the
/// per-user config, cache and state folders.
#[macro_export]
macro_rules! product_short_name {
    () => {
        "sparksmith"
    };
}

/// The product name: "Sparksmith".
pub const NAME: &str = product_name!();
/// The lower-case short name (CLI binary, MCP server, folders).
pub const SHORT_NAME: &str = product_short_name!();
/// The reverse-DNS prefix of bundle and type identifiers.
pub const BUNDLE_ID_PREFIX: &str = "io.sparksmith";
/// The macOS application bundle identifier.
pub const BUNDLE_ID: &str = "io.sparksmith.app";
/// The product's website.
pub const WEBSITE: &str = "https://sparksmith.io";
/// The version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The commit a package was made of (`cargo xtask package` sets it); a build
/// from a checkout has none.
pub const COMMIT: Option<&str> = option_env!("SPARKSMITH_COMMIT");

/// The version as people read it: "0.1.0 (1a2b3c4d5e)".
pub fn version() -> String {
    version_from(None)
}

/// The version, naming `source` (the checkout's commit a host read when it
/// was built) when no package commit is set: "0.1.0 (built from source at
/// 1a2b3c4d5e)". Only the commit's hash, nothing of the machine.
pub fn version_from(source: Option<&str>) -> String {
    match (COMMIT, source.filter(|s| is_commit(s))) {
        (Some(commit), _) => format!("{VERSION} ({commit})"),
        (None, Some(source)) => format!("{VERSION} (built from source at {source})"),
        (None, None) => format!("{VERSION} (built from source)"),
    }
}

/// A short hexadecimal commit hash, and nothing else.
fn is_commit(text: &str) -> bool {
    (7..=40).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_agree() {
        assert_eq!(NAME, "Sparksmith");
        assert_eq!(SHORT_NAME, NAME.to_ascii_lowercase());
        assert!(BUNDLE_ID.starts_with(BUNDLE_ID_PREFIX));
        assert!(WEBSITE.starts_with("https://"));
    }

    /// Run 4, B2: a build from a checkout names its commit when the host
    /// read one; anything but a hash is not shown.
    #[test]
    fn a_source_build_names_its_commit() {
        if COMMIT.is_some() {
            return;
        }
        assert_eq!(
            version_from(Some("c939b0f7ab")),
            format!("{VERSION} (built from source at c939b0f7ab)")
        );
        assert_eq!(version_from(None), format!("{VERSION} (built from source)"));
        assert_eq!(
            version_from(Some("/home/someone")),
            format!("{VERSION} (built from source)")
        );
    }
}
