mod bench;
mod compare;
mod decode;
mod demo;
mod encode;
mod entropy;
mod format;
mod png_chunks;
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
        #[arg(long, default_value_t = 1)]
        cores: usize,
        #[arg(long)]
        preserve_png_metadata: bool,
        #[arg(long)]
        preserve_png_chunks: bool,
        #[arg(long)]
        preserve_source_file: bool,
    },
    Decode {
        input_prsl: PathBuf,
        output_png: Option<PathBuf>,
        #[arg(long)]
        export_png: Option<PathBuf>,
        #[arg(long)]
        extract_source_file: Option<PathBuf>,
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
        #[arg(long, default_value_t = 1)]
        cores: usize,
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
            cores,
            preserve_png_metadata,
            preserve_png_chunks,
            preserve_source_file,
        } => encode::run_encode(
            &input_image,
            &output_prsl,
            encode::EncodeOptions {
                cores,
                preserve_png_metadata,
                preserve_png_chunks,
                preserve_source_file,
            },
        ),
        Command::Decode {
            input_prsl,
            output_png,
            export_png,
            extract_source_file,
        } => decode::run_decode(
            &input_prsl,
            output_png.as_deref(),
            export_png.as_deref(),
            extract_source_file.as_deref(),
        ),
        Command::Verify {
            input_image,
            input_prsl,
        } => verify::run_verify(&input_image, &input_prsl).map(|_| ()),
        Command::Compare {
            first_image,
            second_image,
        } => compare::run_compare(&first_image, &second_image).map(|_| ()),
        Command::Bench { folder, cores } => bench::run_bench(&folder, cores),
        Command::MakeDemoImage { output_png, seed } => demo::run_make_demo_image(&output_png, seed),
    }
}
