//! The macOS bundle's names and identifiers come from `sparksmith_core::product`;
//! `macos/Info.plist` cannot read Rust, so this checks it agrees.
use sparksmith_core::product;

const PLIST: &str = include_str!("../macos/Info.plist");

/// The `<string>` values right after `<key>{key}</key>`.
fn values(key: &str) -> Vec<&'static str> {
    let tag = format!("<key>{key}</key>");
    PLIST
        .match_indices(&tag)
        .filter_map(|(at, _)| {
            let rest = PLIST.get(at + tag.len()..)?;
            let start = rest.find("<string>")? + "<string>".len();
            let end = rest.find("</string>")?;
            rest.get(start..end)
        })
        .collect()
}

#[test]
fn the_bundle_takes_the_product_names() {
    assert_eq!(values("CFBundleName"), [product::NAME]);
    assert_eq!(values("CFBundleDisplayName"), [product::NAME]);
    assert_eq!(values("CFBundleIdentifier"), [product::BUNDLE_ID]);
    assert_eq!(
        values("UTTypeIdentifier"),
        [format!("{}.project", product::BUNDLE_ID_PREFIX)]
    );
    let project = format!("{} project", product::NAME);
    // Its one document type: KiCad files are not imported.
    assert_eq!(values("CFBundleTypeName"), [project.as_str()]);
    assert_eq!(values("UTTypeDescription"), [project.as_str()]);
    assert!(!PLIST.contains("kicad"));
    assert!(PLIST.contains(&format!(
        "<string>{}.project</string>",
        product::BUNDLE_ID_PREFIX
    )));
}

/// The bundle runs the desktop binary, opens the project extension and
/// names the MIME type.
#[test]
fn the_bundle_runs_this_binary_and_opens_the_project_extension() {
    assert_eq!(values("CFBundleExecutable"), [env!("CARGO_BIN_NAME")]);
    assert_eq!(env!("CARGO_BIN_NAME"), "sparksmith-desktop");
    let extensions = format!("<string>{}</string>", sparksmith_core::format::EXTENSION);
    assert_eq!(
        PLIST
            .matches(&format!("<array>{extensions}</array>"))
            .count(),
        2,
        "CFBundleTypeExtensions and public.filename-extension"
    );
    assert!(PLIST.contains("<array><string>application/x-sparksmith</string></array>"));
}
