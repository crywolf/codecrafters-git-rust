use std::{
    fs,
    io::{self, prelude::*, BufReader},
};

use anyhow::Context;

use crate::object::ObjectFile;

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
    let typ = header.typ;
    let size = header.size;

    anyhow::ensure!(
        ["blob", "commit", "tree"].contains(&typ.as_str()),
        "unknown object type '{typ}'"
    );

    if type_only {
        println!("{typ}");
        return Ok(());
    }

    if size_only {
        println!("{size}");
        return Ok(());
    }

    io::stdout()
        .write_all(f.get_content()?)
        .context("writing object data to stdout")?;

    Ok(())
}

/// git hash-object command
pub fn hash_object(file: &str, write: bool) -> anyhow::Result<()> {
    let digest = ObjectFile::hash(file, write)?;
    println!("{digest}");
    Ok(())
}

/// git ls-tree command
pub fn ls_tree(hash: &str, name_only: bool) -> Result<(), anyhow::Error> {
    let mut f = ObjectFile::read(hash)?;
    let header = f.get_header()?;
    let typ = header.typ;

    anyhow::ensure!(typ == "tree", "incorrect object type '{typ}'");

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

        let mut kind = "blob";
        if mode.starts_with('4') {
            kind = "tree";
        }

        if name_only {
            println!("{name}");
        } else {
            println!("{:06} {} {}    {}", mode, kind, hex::encode(hash), name,);
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
