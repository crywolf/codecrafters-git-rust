use std::{
    fs,
    io::{self, prelude::*, BufReader},
    path::PathBuf,
};

use anyhow::{bail, Context};
use flate2::{bufread::ZlibEncoder, Compression};
use hex::ToHex;
use sha1::{Digest, Sha1};

use crate::object;

const OBJECTS_PATH: &str = ".git/objects";

pub fn init() -> Result<(), anyhow::Error> {
    fs::create_dir(".git").unwrap();
    fs::create_dir(".git/objects").unwrap();
    fs::create_dir(".git/refs").unwrap();
    fs::create_dir(".git/refs/heads").unwrap();
    fs::create_dir(".git/refs/tags").unwrap();
    fs::write(".git/HEAD", "ref: refs/heads/master\n").unwrap();
    println!("Initialized git directory");
    Ok(())
}

pub fn cat_file(hash: &str, type_only: bool, size_only: bool) -> anyhow::Result<()> {
    let mut f = object::ObjectFile::new(hash)?;
    f.read_header()?;

    let Some(typ) = f.typ.clone() else {
        bail!("File type was not read")
    };
    let Some(size) = f.size else {
        bail!("File size was not read")
    };

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

    f.read_content()?;

    io::stdout()
        .write_all(f.as_bytes())
        .context("writing object data to stdout")?;

    Ok(())
}

pub fn hash_object(file: &str, write: bool) -> anyhow::Result<()> {
    let f = fs::File::open(file).with_context(|| format!("opening file {file}"))?;
    let mut f = BufReader::new(f);

    let mut content: Vec<u8> = Vec::new();
    let n = f
        .read_to_end(&mut content)
        .with_context(|| format!("reading file {file}"))?;

    let header = format!("blob {n}\0");
    let content_with_header = [header.as_bytes(), &content].concat();

    let digest = Sha1::digest(&content_with_header);

    let mut z = ZlibEncoder::new(
        std::io::Cursor::new(content_with_header),
        Compression::fast(),
    );

    let mut compressed = Vec::new();

    z.read_to_end(&mut compressed)
        .context("compressing the file")?;

    let digest = format!("{:x}", digest);

    println!("{digest}");

    if !write {
        return Ok(());
    }

    let mut path = PathBuf::new();

    let dir = &digest[..2]; // first 2 chars of the digest
    let filename = &digest[2..]; // rest of the digest

    path.push(OBJECTS_PATH);
    path.push(dir);

    fs::create_dir_all(&path).with_context(|| format!("creating directory {}", path.display()))?;

    path.push(filename);

    let mut f =
        fs::File::create(&path).with_context(|| format!("creating file {}", path.display()))?;
    f.write_all(&compressed)?;

    Ok(())
}

pub fn ls_tree(hash: &str, name_only: bool) -> Result<(), anyhow::Error> {
    let mut f = object::ObjectFile::new(hash)?;
    f.read_header()?;

    let Some(typ) = &f.typ else {
        bail!("File type was not read")
    };

    anyhow::ensure!(typ == "tree", "incorrect object type '{typ}'");

    f.read_content()?;

    let mut content = BufReader::new(f.as_bytes());

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
            println!(
                "{:06} {} {}    {}",
                mode,
                kind,
                hash.encode_hex::<String>(),
                name,
            );
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
