use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
};

use anyhow::Context;
use clap::{Parser, Subcommand};
use flate2::read::ZlibDecoder;

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

    /// Provide content for repository objects
    CatFile {
        /// pretty-print object's content
        #[arg(short)]
        pretty_print: bool,

        /// show object type
        #[arg(short)]
        type_only: bool,

        /// show object size
        #[arg(short)]
        size_only: bool,
        hash: String,
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
