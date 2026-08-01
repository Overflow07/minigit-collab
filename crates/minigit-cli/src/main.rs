use anyhow::Result;
use clap::{Parser, Subcommand};
use minigit_core::Repository;
use std::env;

#[derive(Debug, Parser)]
#[command(name = "minigit")]
#[command(about = "A tiny Git-like version control system for learning")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init,
    Add {
        file: String,
    },
    Commit {
        #[arg(short, long)]
        message: String,
    },
    Log,
    Status,
    Checkout {
        commit: String,
    },
    Branch {
        name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init => {
            Repository::init(env::current_dir()?)?;
            println!("Initialized empty MiniGit repository in .minigit/");
        }
        Command::Add { file } => {
            let repo = Repository::discover(env::current_dir()?)?;
            let hash = repo.add(&file)?;
            println!("added {file} as {hash}");
        }
        Command::Commit { message } => {
            let repo = Repository::discover(env::current_dir()?)?;
            let hash = repo.commit(message)?;
            println!("committed {hash}");
        }
        Command::Log => {
            let repo = Repository::discover(env::current_dir()?)?;
            let commits = repo.log()?;

            for (hash, commit) in commits {
                println!("commit {hash}");
                println!("    {}", commit.message);
                println!();
            }
        }

        Command::Status => {
            let repo = Repository::discover(env::current_dir()?)?;
            let status = repo.status()?;

            println!("Staged files:");
            for path in status.staged {
                println!("  {path}");
            }

            println!("Modified files:");
            for path in status.modified {
                println!("  {path}");
            }

            println!("Untracked files:");
            for path in status.untracked {
                println!("  {path}");
            }
        }

        Command::Checkout { commit } => {
            let repo = Repository::discover(env::current_dir()?)?;
            repo.checkout(&commit)?;
            println!("checked out {commit}");
        }

        Command::Branch { name } => {
            let repo = Repository::discover(env::current_dir()?)?;
            repo.create_branch(&name)?;
            println!("created branch {name}");
        }
    }

    Ok(())
}
