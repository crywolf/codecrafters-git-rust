use std::{io::prelude::*, path::PathBuf};

use anyhow::Context;

use crate::object::{ObjectFile, ObjectType};

/// git ls-tree command
pub fn invoke(hash: &str, recurse: bool, name_only: bool) -> anyhow::Result<()> {
    list_tree(hash, recurse, name_only, None)
}

fn list_tree(
    hash: &str,
    recurse: bool,
    name_only: bool,
    path_prefix: Option<&str>,
) -> anyhow::Result<()> {
    let mut object = ObjectFile::read(hash)?;

    let typ = object.header.typ;
    anyhow::ensure!(typ == ObjectType::Tree, "incorrect object type '{typ}'");

    loop {
        let mut buf = Vec::new();
        let n = object
            .reader
            .read_until(0, &mut buf)
            .context("reading mode and name for tree item")?;
        if n == 0 {
            break;
        }

        let item = std::ffi::CStr::from_bytes_with_nul(&buf)
            .expect("should be null terminated string")
            .to_str()
            .context("mode and name in tree item is not valid UTF-8")?;

        let (mode, name) = item
            .split_once(' ')
            .with_context(|| format!("parsing object mode and name from {item}"))?;

        let mut hash = [0; 20];
        object
            .reader
            .read_exact(&mut hash)
            .context("reading sha hash of tree item")?;

        let mut kind = ObjectType::Blob;
        if mode.starts_with('4') {
            kind = ObjectType::Tree;
        }

        if recurse && kind == ObjectType::Tree {
            list_tree(hex::encode(hash).as_str(), recurse, name_only, Some(name))?;
        } else {
            let mut name = PathBuf::from(name);
            if let Some(prefix) = path_prefix {
                name = PathBuf::from(prefix).join(name);
            }
            if name_only {
                println!("{}", name.display());
            } else {
                println!(
                    "{:06} {} {}\t{}",
                    mode.parse::<u64>()
                        .context("incorrect file mode - not a number")?,
                    kind,
                    hex::encode(hash),
                    name.display()
                );
            }
        }
    }

    let n = object
        .reader
        .read(&mut [0])
        .context("ensuring that object was completely read")?;

    anyhow::ensure!(
        n == 0,
        "object size is {n} bytes larger than stated in object header"
    );

    Ok(())
}
