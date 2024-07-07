use anyhow::Context;

use crate::object::{ObjectFile, ObjectType};

/// git cat-file command
pub fn invoke(
    hash: &str,
    object_type: Option<String>,
    pretty_print: bool,
    type_only: bool,
    size_only: bool,
) -> anyhow::Result<()> {
    let mut object = ObjectFile::read(hash)?;

    let real_object_type = object.header.typ;
    let size = object.header.size;

    if let Some(object_type) = object_type {
        if object_type != real_object_type.to_string() {
            anyhow::bail!("File is not {}", object_type)
        }
    }

    if type_only {
        println!("{real_object_type}");
        return Ok(());
    }

    if size_only {
        println!("{size}");
        return Ok(());
    }

    if pretty_print && real_object_type == ObjectType::Tree {
        return super::ls_tree::invoke(hash, false, false);
    }

    let mut stdout = std::io::stdout().lock();

    std::io::copy(&mut object.reader, &mut stdout).context("streaming file content to stdin")?;

    Ok(())
}
