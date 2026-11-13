mod cli;
mod commands;
mod config;
mod credentials;
mod exit;
mod output;

use clap::Parser;

use crate::cli::Cli;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let code = commands::dispatch(cli).await;
    std::process::exit(code);
}
