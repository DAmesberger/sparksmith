use super::*;
#[test]
fn a_broken_topic_names_its_source() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    std::fs::write(root.join(crate::shipped::MARKER), "{}").expect("marker");
    std::fs::create_dir_all(root.join("help/generated")).expect("generated");
    std::fs::create_dir_all(root.join("help/topics")).expect("topics");
    std::fs::write(root.join("help/topics/broken.md"), "no frontmatter").expect("file");
    let err = load(&ShippedLibrary::at(root).expect("library")).expect_err("invalid");
    assert!(err.to_string().contains("broken.md"), "{err}");
}
