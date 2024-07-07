mod commands;
mod object;

use std::path::PathBuf;

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
    #[command(allow_missing_positional = true)]
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

        /// Object type. Typically this matches the real type of <object>.
        #[arg(
            id = "type",
            conflicts_with_all = ["pretty_print","size_only", "type_only"],
            required_unless_present_any = ["pretty_print", "size_only", "type_only"]
        )]
        object_type: Option<String>,

        /// Object hash
        #[arg(id = "object", required = true)]
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
        file: PathBuf,
    },

    /// List the contents of a tree object
    LsTree {
        /// Recurse into sub-trees
        #[arg(short)]
        recurse: bool,

        /// List only filenames (instead of the "long" output), one per line
        #[arg(long)]
        name_only: bool,

        /// Id of a tree-ish
        #[arg(id = "tree-ish")]
        hash: String,
    },

    /// Create a tree object
    WriteTree {},

    /// Create a new commit object
    CommitTree {
        /// Parent commit object
        #[arg(short, id = "parent")]
        parent_hash: Option<String>,

        /// Commit log message
        #[arg(short, id = "message")]
        message: String,

        /// An existing tree object
        #[arg(id = "tree")]
        tree_hash: String,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Cli::parse();
    match args.command {
        Commands::Init => commands::init::invoke(),
        Commands::CatFile {
            pretty_print,
            object_type,
            type_only,
            size_only,
            hash,
        } => commands::cat_file::invoke(&hash, object_type, pretty_print, type_only, size_only),
        Commands::HashObject {
            write,
            file,
            typ: _,
        } => {
            let hash = commands::hash_object::invoke(file, write)?;
            println!("{}", hex::encode(hash));
            Ok(())
        }
        Commands::LsTree {
            recurse,
            name_only,
            hash,
        } => commands::ls_tree::invoke(&hash, recurse, name_only),
        Commands::WriteTree {} => commands::write_tree::invoke(),
        Commands::CommitTree {
            parent_hash,
            message,
            tree_hash,
        } => {
            let hash = commands::commit_tree::invoke(&tree_hash, &message, parent_hash)?;
            println!("{}", hex::encode(hash));
            Ok(())
        }
    }
}
