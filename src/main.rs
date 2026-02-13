use clap::Parser;
use std::path::PathBuf;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

mod config;
mod server;
mod state;
mod types;
mod topic;
mod store;
mod util;
mod file_storage;
mod auth;
mod email;
mod cron;
mod limits;
mod cmd;
mod webpush;
mod firebase;

use clap::Subcommand;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the configuration file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Listen interface and port (e.g. :80)
    #[arg(short, long)]
    listen_http: Option<String>,

    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// User management commands
    User(cmd::user::UserArgs),
    /// Run integration tests
    Test(cmd::test::TestArgs),
    /// Run the server (default)
    Serve,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize logging
    let log_level = if args.debug { Level::DEBUG } else { Level::INFO };
    let subscriber = FmtSubscriber::builder()
        .with_max_level(log_level)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("setting default subscriber failed");

    // Load configuration
    // TODO: Pass config file from args if present
    let config = config::Config::load()?;

    info!("Starting ntfy server...");
    
    match args.command {
        Some(Commands::User(user_args)) => {
            cmd::user::handle_user_command(user_args, config).await?;
        },
        Some(Commands::Test(test_args)) => {
            cmd::test::handle_test_command(test_args, config).await?;
        },
        Some(Commands::Serve) | None => {
             // Start server
             server::run(config).await?;
        }
    }

    Ok(())
}
