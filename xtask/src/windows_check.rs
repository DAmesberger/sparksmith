//! `cargo xtask windows-check [-p PACKAGE]...`: strict Clippy of the whole
//! workspace, tests included, for Windows, on a machine that is not one.
//!
//! Code under `cfg(windows)` and `cfg(not(unix))`, and what only the rest of
//! the code uses, is compiled nowhere else before hosted CI. Clippy
//! type-checks without linking, so no Windows library is needed but the C
//! compiler some dependencies' build scripts call: `nix develop .#windows`
//! has it and the target. No test runs. `cfg(target_env = "msvc")` is not
//! seen, since the target is the GNU one.
use crate::{Result, cargo};
use std::path::Path;
use std::process::Command;

const TARGET: &str = "x86_64-pc-windows-gnu";

pub(crate) fn run(packages: &[String]) -> Result<()> {
    let libraries = Command::new("rustc")
        .args(["--print", "target-libdir", "--target", TARGET])
        .output()?;
    let folder = String::from_utf8(libraries.stdout)?;
    if !Path::new(folder.trim()).is_dir() {
        return Err(format!(
            "this toolchain has no {TARGET}; run: nix develop .#windows -c cargo xtask windows-check"
        )
        .into());
    }
    cargo(&arguments(packages)?)
}

fn arguments(packages: &[String]) -> Result<Vec<&str>> {
    let mut call = vec!["clippy", "--locked", "--profile", "ci", "--target", TARGET];
    match packages {
        [] => call.push("--workspace"),
        chosen if chosen.len() % 2 == 0 && chosen.iter().step_by(2).all(|a| a == "-p") => {
            call.extend(chosen.iter().map(String::as_str));
        }
        _ => return Err("usage: cargo xtask windows-check [-p PACKAGE]...".into()),
    }
    // Every crate's findings in one run, not the first one's.
    call.extend(["--all-targets", "--keep-going", "--", "-D", "warnings"]);
    Ok(call)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workspace_or_the_chosen_packages_are_checked() -> Result<()> {
        assert!(arguments(&[])?.contains(&"--workspace"));
        let chosen = ["-p".to_owned(), "sparksmith-io".to_owned()];
        let call = arguments(&chosen)?;
        assert!(call.contains(&"sparksmith-io") && !call.contains(&"--workspace"));
        assert!(arguments(&["sparksmith-io".to_owned()]).is_err());
        assert!(arguments(&["-p".to_owned()]).is_err());
        Ok(())
    }
}
