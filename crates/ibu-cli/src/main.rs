use anyhow::Result;
use clap::{Parser, Subcommand};

mod cat;
mod sort;
mod utils;
mod view;

/// Command-line toolkit for IBU files
#[derive(Parser)]
#[command(name = "ibu", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// View the contents of an IBU file as plain text
    View(view::ArgsView),

    /// Concatenate the contents of multiple IBU files
    Cat(cat::ArgsCat),

    /// Sort the contents of an IBU file
    Sort(sort::ArgsSort),
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::View(args) => view::run(&args),
        Command::Cat(args) => cat::run(&args),
        Command::Sort(args) => sort::run(&args),
    }
}
