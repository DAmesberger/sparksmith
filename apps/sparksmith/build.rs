//! The checkout's commit, for About and the version a source build shows
//! (`SPARKSMITH_SOURCE_COMMIT`): the hash only, nothing of the machine. A
//! package sets `SPARKSMITH_COMMIT` for the core instead; without git or a
//! checkout nothing is set. No rerun rule is printed, so cargo runs this
//! again whenever a file of the app changes.
use std::process::Command;

fn main() {
    if std::env::var_os("SPARKSMITH_COMMIT").is_some() {
        return;
    }
    let Ok(out) = Command::new("git")
        .args(["rev-parse", "--short=10", "HEAD"])
        .output()
    else {
        return;
    };
    let Ok(text) = String::from_utf8(out.stdout) else {
        return;
    };
    let commit = text.trim();
    if out.status.success()
        && (7..=40).contains(&commit.len())
        && commit.bytes().all(|b| b.is_ascii_hexdigit())
    {
        println!("cargo:rustc-env=SPARKSMITH_SOURCE_COMMIT={commit}");
    }
}
