mod command;

use clap::{Parser, Subcommand};

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
        /// Show object type
        #[arg(short, conflicts_with = "size_only")]
        type_only: bool,

        /// Show object size
        #[arg(short, conflicts_with = "type_only")]
        size_only: bool,

        /// Pretty-print object's content
        #[arg(short, conflicts_with_all = ["size_only", "type_only"])]
        pretty_print: bool,

        /// Object hash
        #[arg(id = "object")]
        hash: String,
    },

    /// Compute object ID and optionally create an object from a file
    HashObject {
        /// Actually write the object into the object database
        #[arg(short)]
        write: bool,

        /// Object type
        #[arg(short, id = "type", default_value = "blob")]
        typ: String,

        #[arg(id = "file")]
        file: String,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    match args.command {
        Commands::Init => command::init(),
        Commands::CatFile {
            pretty_print: _,
            type_only,
            size_only,
            hash,
        } => command::cat_file(&hash, type_only, size_only),
        Commands::HashObject {
            write,
            file,
            typ: _,
        } => command::hash_object(&file, write),
    }
}
