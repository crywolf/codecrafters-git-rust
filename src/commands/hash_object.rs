use std::path::Path;

use crate::object::ObjectFile;

/// git hash-object command
pub fn invoke(path: impl AsRef<Path>, write: bool) -> anyhow::Result<[u8; 20]> {
    let mut object = ObjectFile::from_file(path)?;

    let hash = if write {
        // compress and write to disk
        object.write(None)?
    } else {
        // just compute object hash
        object.hash()?
    };

    Ok(hash)
}
