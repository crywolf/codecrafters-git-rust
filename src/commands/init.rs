use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;

// https://blog.meain.io/2023/what-is-in-dot-git/

/// git init command
pub fn invoke() -> anyhow::Result<()> {
    create_git_dirs(None).context("creating git directories")?;
    println!("Initialized git directory");
    Ok(())
}

pub fn create_git_dirs(custom_dir: Option<&Path>) -> anyhow::Result<()> {
    let parent = match custom_dir {
        Some(custom_dir) => {
            fs::create_dir(custom_dir)?;
            PathBuf::from(custom_dir)
        }
        None => PathBuf::new(),
    };

    fs::create_dir(parent.join(".git"))?;
    fs::create_dir(parent.join(".git/objects"))?;
    fs::create_dir(parent.join(".git/refs"))?;
    fs::create_dir(parent.join(".git/refs/heads"))?;
    fs::create_dir(parent.join(".git/refs/tags"))?;
    fs::write(parent.join(".git/HEAD"), "ref: refs/heads/master\n")?;
    Ok(())
}
