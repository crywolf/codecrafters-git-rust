use std::{
    fmt::Write,
    fs,
    io::prelude::*,
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::object::{Header, ObjectFile, ObjectType};

pub fn init() -> anyhow::Result<()> {
    create_git_dirs().context("creating git directories")?;
    println!("Initialized git directory");
    Ok(())
}

fn create_git_dirs() -> anyhow::Result<()> {
    fs::create_dir(".git")?;
    fs::create_dir(".git/objects")?;
    fs::create_dir(".git/refs")?;
    fs::create_dir(".git/refs/heads")?;
    fs::create_dir(".git/refs/tags")?;
    fs::write(".git/HEAD", "ref: refs/heads/master\n")?;
    Ok(())
}

/// git cat-file command
pub fn cat_file(hash: &str, type_only: bool, size_only: bool) -> anyhow::Result<()> {
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

/// git hash-object command
pub fn hash_object(path: impl AsRef<Path>, write: bool) -> anyhow::Result<[u8; 20]> {
    let object = ObjectFile::from_file(path)?;

    let hash = if write {
        // compress and write to disk
        object.write()?
    } else {
        // just compute object hash
        object.hash()?
    };

    Ok(hash)
}

/// git ls-tree command
pub fn ls_tree(hash: &str, recurse: bool, name_only: bool) -> anyhow::Result<()> {
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

/// git write-tree command
pub fn write_tree() -> anyhow::Result<()> {
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
            hash_object(&entry.path(), false)?
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

/// git hash-object command
pub fn commit_tree(
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

    let object = ObjectFile {
        header: Header {
            typ: ObjectType::Commit,
            size: commit.len(),
        },
        reader: std::io::Cursor::new(commit),
    };

    object.write()
}
