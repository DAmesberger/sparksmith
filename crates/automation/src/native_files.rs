//! The one production host for native file publication, shared with the app.
use sparksmith_solidcraft::native::files::{NewFile, NewFileWriter};
use sparksmith_solidcraft::{Error, Result};
use std::path::Path;

pub struct Writer;

impl NewFileWriter for Writer {
    fn write_new(&self, target: &str, write: &mut dyn FnMut(&str) -> Result<()>) -> Result<()> {
        let staged =
            sparksmith_io::persistence::StagedNew::new(Path::new(target)).map_err(file_error)?;
        let path = staged
            .path()
            .to_str()
            .ok_or_else(|| Error::Invalid("the native output path must be UTF-8".into()))?;
        write(path)?;
        staged.persist().map_err(file_error)
    }
}

pub fn new_file(path: &str) -> NewFile<'_> {
    NewFile::new(path, &Writer)
}

fn file_error(error: sparksmith_io::Error) -> Error {
    match error {
        sparksmith_io::Error::Io(error) => Error::Io(error),
        sparksmith_io::Error::Unsynced(message) => Error::Unsynced(message),
        sparksmith_io::Error::Invalid(message) => Error::Invalid(message),
        other => Error::Io(std::io::Error::other(other)),
    }
}

#[cfg(test)]
mod tests;
