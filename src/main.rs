use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "picoclaw")]
#[command(about = "Ultra-lightweight personal AI assistant", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize configuration and workspace
    Onboard,
    /// Start interactive agent mode
    Agent {
        /// Direct message to process
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Start multi-channel gateway
    Gateway,
    /// Manage authentication
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
    /// Show version information
    Version,
}

#[derive(Subcommand)]
enum AuthAction {
    Login,
    Logout,
    Status,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Version) | None => {
            println!("picoclaw {}", env!("CARGO_PKG_VERSION"));
        }
        Some(Commands::Onboard) => {
            println!("Onboard not yet implemented");
        }
        Some(Commands::Agent { message }) => {
            println!("Agent mode not yet implemented. Message: {:?}", message);
        }
        Some(Commands::Gateway) => {
            println!("Gateway not yet implemented");
        }
        Some(Commands::Auth { action }) => {
            match action {
                AuthAction::Login => println!("Login not yet implemented"),
                AuthAction::Logout => println!("Logout not yet implemented"),
                AuthAction::Status => println!("Status not yet implemented"),
            }
        }
    }

    Ok(())
}
