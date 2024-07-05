use std::{
    fs,
    io::{prelude::*, BufReader},
    path::{Path, PathBuf},
};

use anyhow::Context;

use crate::object::{ObjectFile, ObjectType};

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
    let mut f = ObjectFile::read(hash)?;

    let header = f.get_header()?;
    let object_type = header.typ;
    let size = header.size;

    if type_only {
        println!("{object_type}");
        return Ok(());
    }

    if size_only {
        println!("{size}");
        return Ok(());
    }

    std::io::stdout()
        .write_all(f.get_content()?)
        .context("writing object data to stdout")?;

    Ok(())
}

/// git hash-object command
pub fn hash_object(path: &Path, write: bool) -> anyhow::Result<[u8; 20]> {
    let mut f = fs::File::open(path).with_context(|| format!("opening file {}", path.display()))?;

    let mut content: Vec<u8> = Vec::new();
    let size = f
        .read_to_end(&mut content)
        .with_context(|| format!("reading file {}", path.display()))?;

    let typ = ObjectType::Blob;
    let header = format!("{} {size}\0", &typ);
    let content_with_header = [header.as_bytes(), &content].concat();

    let r = std::io::Cursor::new(content_with_header);

    let hash = if write {
        // compress and write to disk
        ObjectFile::write(r)?
    } else {
        // just compute object hash
        ObjectFile::hash(r)?
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
    let mut f = ObjectFile::read(hash)?;
    let header = f.get_header()?;
    let typ = header.typ;

    anyhow::ensure!(typ == ObjectType::Tree, "incorrect object type '{typ}'");

    let mut content = BufReader::new(f.get_content()?);

    loop {
        let mut buf = Vec::new();
        let n = content
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
        content
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

    let n = content
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

    let typ = ObjectType::Tree;
    let header = format!("{} {}\0", &typ, tree.len());
    let content_with_header = [header.as_bytes(), &tree].concat();

    let r = std::io::Cursor::new(content_with_header);

    // compress and write to disk
    let hash = ObjectFile::write(r)?;

    Ok(Some(hash))
}
