use std::fmt::Write;
use std::fs;

use anyhow::Context;

use crate::object::{Header, ObjectFile, ObjectType};

/// git hash-object command
pub fn invoke(
    tree_hash: &str,
    message: &str,
    parent_hash: Option<String>,
) -> anyhow::Result<[u8; 20]> {
    // check tree existence
    let tree_path = ObjectFile::hash_to_path(tree_hash);
    fs::metadata(&tree_path)
        .with_context(|| format!("tree object does not exist: {}", tree_path.display()))?;

    let mut commit = String::new();
    writeln!(commit, "tree {tree_hash}")?;

    if let Some(parent_hash) = parent_hash {
        writeln!(commit, "parent {parent_hash}")?;
    }

    let time = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .context("current system time is before UNIX epoch")?;

    let name = "crywolf";
    let email = "cry.wolf@centrum.cz";

    writeln!(commit, "author: {name} <{email}> {} +0000", time.as_secs())?;

    writeln!(
        commit,
        "committer {name} <{email}> {} +0000",
        time.as_secs()
    )?;

    writeln!(commit, "\n{message}")?;

    let mut object = ObjectFile {
        header: Header {
            typ: ObjectType::Commit,
            size: commit.len(),
        },
        reader: std::io::Cursor::new(commit),
    };

    object.write(None)
}
