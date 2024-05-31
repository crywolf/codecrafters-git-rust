use std::{
    fs,
    io::{self, prelude::*, BufReader},
    path::PathBuf,
};

use anyhow::Context;
use clap::{Parser, Subcommand};
use flate2::{bufread::ZlibEncoder, read::ZlibDecoder, Compression};
use sha1::{Digest, Sha1};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create an empty Git repository
    Init,

    /// Provide content or type and size information for repository objects
    CatFile {
        /// Pretty-print object's content
        #[arg(short)]
        pretty_print: bool,

        /// Show object type
        #[arg(short)]
        type_only: bool,

        /// Show object size
        #[arg(short)]
        size_only: bool,

        /// Object hash
        #[arg(id = "object")]
        hash: String,
    },

    /// Compute object ID and optionally create an object from a file
    HashObject {
        /// Actually write the object into the object database
        #[arg(short)]
        write: bool,

        #[arg(id = "file")]
        file: String,
    },
}

const OBJECTS_PATH: &str = ".git/objects";

fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    match args.command {
        Commands::Init => {
            fs::create_dir(".git").unwrap();
            fs::create_dir(".git/objects").unwrap();
            fs::create_dir(".git/refs").unwrap();
            fs::create_dir(".git/refs/heads").unwrap();
            fs::create_dir(".git/refs/tags").unwrap();
            fs::write(".git/HEAD", "ref: refs/heads/master\n").unwrap();
            println!("Initialized git directory");
            Ok(())
        }
        Commands::CatFile {
            pretty_print: _,
            type_only,
            size_only,
            hash,
        } => cat_file(&hash, type_only, size_only),
        Commands::HashObject { write, file } => hash_object(&file, write),
    }
}

pub fn cat_file(hash: &str, type_only: bool, size_only: bool) -> anyhow::Result<()> {
    let dir = &hash[..2];
    let file = &hash[2..];
    let path = format!("{OBJECTS_PATH}/{dir}/{file}");

    let f = fs::File::open(&path).with_context(|| format!("opening file {path}"))?;

    let z = ZlibDecoder::new(f);
    let mut z = BufReader::new(z);

    let mut buf = Vec::new();

    z.read_until(0, &mut buf)
        .with_context(|| format!("reading object header in file {path}"))?;

    let header = std::ffi::CStr::from_bytes_with_nul(&buf)
        .expect("should be null terminated string")
        .to_str()
        .context("file header is not valid UTF-8")?;

    let (typ, size) = header
        .split_once(' ')
        .with_context(|| format!("parsing object header {header}"))?;

    anyhow::ensure!(
        ["blob", "commit", "tree"].contains(&typ),
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

    let size = size.parse::<usize>().context("parsing object size")?;

    buf.clear();
    buf.reserve_exact(size);
    buf.resize(size, 0);

    z.read_exact(&mut buf).context("reading object data")?;

    let n = z
        .read(&mut [0])
        .context("ensuring that object was completely read")?;

    anyhow::ensure!(
        n == 0,
        "object size is {n} bytes larger than stated in object header"
    );

    io::stdout()
        .write_all(&buf)
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
