mod bench;
mod compare;
mod decode;
mod demo;
mod encode;
mod entropy;
mod format;
mod predict;
#[cfg(test)]
mod tests;
mod tiles;
mod transform;
mod verify;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "pressel")]
#[command(author = "Petar Stanchev")]
#[command(version)]
#[command(about = "Experimental strictly lossless image codec and research platform")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Encode {
        input_image: PathBuf,
        output_prsl: PathBuf,
    },
    Decode {
        input_prsl: PathBuf,
        output_png: PathBuf,
    },
    Verify {
        input_image: PathBuf,
        input_prsl: PathBuf,
    },
    Compare {
        first_image: PathBuf,
        second_image: PathBuf,
    },
    Bench {
        folder: PathBuf,
    },
    MakeDemoImage {
        output_png: PathBuf,
        #[arg(long, default_value_t = 0)]
        seed: u64,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Encode {
            input_image,
            output_prsl,
        } => encode::run_encode(&input_image, &output_prsl),
        Command::Decode {
            input_prsl,
            output_png,
        } => decode::run_decode(&input_prsl, &output_png),
        Command::Verify {
            input_image,
            input_prsl,
        } => verify::run_verify(&input_image, &input_prsl).map(|_| ()),
        Command::Compare {
            first_image,
            second_image,
        } => compare::run_compare(&first_image, &second_image).map(|_| ()),
        Command::Bench { folder } => bench::run_bench(&folder),
        Command::MakeDemoImage { output_png, seed } => demo::run_make_demo_image(&output_png, seed),
    }
}
