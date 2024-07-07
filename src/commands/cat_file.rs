use anyhow::Context;

use crate::object::ObjectFile;

/// git cat-file command
pub fn invoke(hash: &str, type_only: bool, size_only: bool) -> anyhow::Result<()> {
    let mut object = ObjectFile::read(hash)?;

    let object_type = object.header.typ;
    let size = object.header.size;

    if type_only {
        println!("{object_type}");
        return Ok(());
    }

    if size_only {
        println!("{size}");
        return Ok(());
    }

    let mut stdout = std::io::stdout().lock();

    std::io::copy(&mut object.reader, &mut stdout).context("streaming file content to stdin")?;

    Ok(())
}
