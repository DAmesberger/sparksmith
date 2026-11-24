//! `stackup list|show`: the fabricators' stackups that ship
//! (`sparksmith_io::fab_stackups`). Taking one is the typed
//! `layout.use_stackup` request (`stdio`/`mcp`).
use super::{Result, open, print};
use serde_json::{Value, json};
use sparksmith_core::layout::fab_stackup::{self, FabStackup, PickOptions, StackupFilter};
use sparksmith_core::product::SHORT_NAME as CLI;
use sparksmith_io::fab_stackups;

/// The usage line(s).
pub const USAGE: &str = "stackup list [FILE.sparksmith]
                                   list the fabricators' stackups: every one, or those
                                   for a project's copper layers and fab profile
  stackup show ID [FILE.sparksmith]
                                   one stackup as published, with its sources; with a
                                   project, what taking it would change";

/// Run the subcommand with the arguments after its name.
pub fn run(args: &[String]) -> Result<()> {
    print(&reply(&fab_stackups::shipped()?, args)?)
}

/// The subcommand's JSON reply over `library`.
fn reply(library: &[FabStackup], args: &[String]) -> Result<Value> {
    match args {
        [command, rest @ ..] if command == "list" && rest.len() <= 1 => {
            let file = rest.first();
            let session = open(file)?;
            let filter = StackupFilter {
                all: file.is_none(),
                ..StackupFilter::default()
            };
            let listed = fab_stackup::list(session.engine().project(), library, &filter);
            Ok(serde_json::to_value(listed)?)
        }
        [command, id] if command == "show" => Ok(json!(fab_stackups::find(library, id)?)),
        [command, id, file] if command == "show" => {
            let session = open(Some(file))?;
            let report = fab_stackup::report(
                session.engine().project(),
                fab_stackups::find(library, id)?,
                &PickOptions::default(),
            )?;
            Ok(serde_json::to_value(report)?)
        }
        _ => Err(format!("usage: {CLI} {USAGE}").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn every_stackup_is_listed_and_one_is_shown_with_its_sources() -> Result<()> {
        // As `run` in main enters it for the whole program.
        let _context = sparksmith_core::library::standards::Context::fixture().map(|c| c.enter());
        let library = fab_stackups::shipped()?;
        let listed = reply(&library, &args(&["list"]))?;
        let rows = listed["stackups"].as_array().map_or(0, Vec::len);
        assert_eq!(rows, library.len());
        assert_eq!(listed["left_out"], 0);
        let first = library.first().ok_or("no stackup ships")?;
        let shown = reply(&library, &args(&["show", &first.id]))?;
        assert_eq!(shown["source"]["url"], json!(first.source.url));
        assert!(shown["disagreements"].is_array());
        assert!(reply(&library, &args(&["show", "none"])).is_err());
        assert!(reply(&library, &args(&["remove", &first.id])).is_err());
        Ok(())
    }
}
