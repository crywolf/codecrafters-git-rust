use std::{fs, path::Path};

use anyhow::Context;

use crate::object::{Header, ObjectFile, ObjectType};

use super::hash_object;

/// git write-tree command
pub fn invoke() -> anyhow::Result<()> {
    let Some(hash) = write_tree_for(Path::new(".")).context("construct root tree object")? else {
        anyhow::bail!("asked to make tree object for empty tree");
    };

    println!("{}", hex::encode(hash));

    Ok(())
}

fn write_tree_for(path: &Path) -> anyhow::Result<Option<[u8; 20]>> {
    let mut entries = Vec::new();
    let dir = fs::read_dir(path).context("opening a directory")?;

    // read entries in directory
    for entry in dir {
        let entry = entry.with_context(|| format!("bad directory entry in {}", path.display()))?;

        let file_name = entry.file_name();
        let metadata = entry.metadata().context("metadata for directory entry")?;

        //TODO: skip files defined in .gitignore
        if file_name == ".git" || file_name == "target" {
            continue;
        }

        entries.push((entry, file_name, metadata));
    }

    // sort entries
    entries.sort_unstable_by(|a, b| {
        let mut aname = a.1.as_encoded_bytes().to_vec();
        let mut bname = b.1.as_encoded_bytes().to_vec();
        if a.2.is_dir() {
            aname.push(b'/');
        }
        if b.2.is_dir() {
            bname.push(b'/');
        }
        aname.cmp(&bname)
    });

    let mut tree = Vec::new();
    for (entry, file_name, metadata) in entries {
        let mode: &str;
        if metadata.is_dir() {
            mode = "40000";
        } else if metadata.is_symlink() {
            mode = "120000";
        } else {
            mode = "100644";
        }
        //  TODO ?  100755 (executable file)

        let hash = if metadata.is_dir() {
            if let Some(hash) = write_tree_for(&entry.path())? {
                hash
            } else {
                // empty directory, skip it
                continue;
            }
        } else {
            hash_object::invoke(&entry.path(), false)?
        };

        // <mode> <name>\0<20_byte_sha>
        tree.extend(mode.as_bytes());
        tree.push(b' ');
        tree.extend(file_name.as_encoded_bytes());
        tree.push(0);
        tree.extend(hash);
    }

    if tree.is_empty() {
        return Ok(None);
    }

    let header = Header {
        typ: ObjectType::Tree,
        size: tree.len(),
    };

    let tree_object = ObjectFile {
        header,
        reader: std::io::Cursor::new(tree),
    };

    // compress and write to disk
    let hash = tree_object.write()?;

    Ok(Some(hash))
}
