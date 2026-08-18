use anyhow::Result;
use clap::{Parser, Subcommand};

mod cat;
mod sort;
mod umi;
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

    /// Correct UMI errors in an IBU file
    ///
    /// Expects a sorted IBU file as input
    Umi(umi::ArgsUmi),
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::View(args) => view::run(&args),
        Command::Cat(args) => cat::run(&args),
        Command::Sort(args) => sort::run(&args),
        Command::Umi(args) => umi::run(&args),
    }
}

fn main() {
    if let Err(err) = run() {
        // A downstream consumer closing the pipe (e.g. `ibu view | head`) is a
        // normal way for a pipeline to end, not an error
        let broken_pipe = err.chain().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
        });
        if broken_pipe {
            std::process::exit(0);
        }
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
