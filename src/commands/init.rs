use std::fs;

use anyhow::Context;

/// git inot command
pub fn invoke() -> anyhow::Result<()> {
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
