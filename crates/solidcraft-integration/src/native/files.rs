//! The host owns atomic publication; Solidcraft owns native serialization.
use crate::Result;

/// A host writer that stages native output and publishes it without replacing
/// an existing target, including a file created during `write`. It removes
/// staged data when `write` fails and propagates that native error unchanged.
pub trait NewFileWriter {
    fn write_new(&self, target: &str, write: &mut dyn FnMut(&str) -> Result<()>) -> Result<()>;
}

impl<F> NewFileWriter for F
where
    F: Fn(&str, &mut dyn FnMut(&str) -> Result<()>) -> Result<()>,
{
    fn write_new(&self, target: &str, write: &mut dyn FnMut(&str) -> Result<()>) -> Result<()> {
        self(target, write)
    }
}

/// A new native file and the host that will publish it.
#[derive(Clone, Copy)]
pub struct NewFile<'a> {
    path: &'a str,
    pub(crate) writer: &'a dyn NewFileWriter,
}

impl<'a> NewFile<'a> {
    pub fn new(path: &'a str, writer: &'a dyn NewFileWriter) -> Self {
        Self { path, writer }
    }

    pub fn path(self) -> &'a str {
        self.path
    }
}

/// A test host for adapter unit tests, which do not depend on the IO layer.
/// The production host and its race/durability tests live in automation.
#[cfg(test)]
pub(crate) fn test_file(path: &str) -> NewFile<'_> {
    use crate::Error;
    use std::path::Path;
    fn write_new(target: &str, write: &mut dyn FnMut(&str) -> Result<()>) -> Result<()> {
        let target = Path::new(target);
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let temporary = tempfile::Builder::new()
            .suffix(".solidcraft")
            .tempfile_in(parent)?
            .into_temp_path();
        std::fs::remove_file(&temporary)?;
        let staged = temporary
            .to_str()
            .ok_or_else(|| Error::Invalid("test path must be UTF-8".into()))?;
        write(staged)?;
        temporary
            .persist_noclobber(target)
            .map_err(|error| Error::Io(error.error))?;
        Ok(())
    }
    NewFile::new(path, &write_new)
}
