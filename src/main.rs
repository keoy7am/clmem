use clap::Parser;

mod cli;
mod daemon;
mod ipc;
mod models;
mod platform;
mod tui;

#[derive(Parser)]
#[command(name = "clmem", about = "Claude Code Memory Monitor", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Start the background monitoring daemon
    Daemon {
        /// Run in foreground (don't daemonize)
        #[arg(long)]
        foreground: bool,
    },
    /// Launch the interactive TUI dashboard
    Tui,
    /// Show current memory status (one-shot, no daemon required)
    Status {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Clean up orphaned/stale processes
    Cleanup {
        /// Dry run - show what would be cleaned
        #[arg(long)]
        dry_run: bool,
        /// Force cleanup of IDLE processes too
        #[arg(long)]
        force: bool,
        /// Clean ALL Claude processes (requires confirmation)
        #[arg(long)]
        all: bool,
        /// Specific PIDs to clean
        #[arg(long, value_delimiter = ',')]
        pids: Option<Vec<u32>>,
    },
    /// Show memory history
    History {
        /// Number of recent entries
        #[arg(short = 'n', long, default_value = "60")]
        count: usize,
        /// Export as CSV
        #[arg(long)]
        csv: bool,
    },
    /// Generate a diagnostic report
    Report {
        /// Output file path (default: stdout)
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
    },
    /// Manage configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(clap::Subcommand)]
pub enum ConfigAction {
    /// Show current configuration
    Show,
    /// Edit configuration (opens in editor)
    Edit,
    /// Reset to defaults
    Reset,
    /// Show config file path
    Path,
}

fn main() -> anyhow::Result<()> {
    // Load config early to use log_level as fallback
    let config = models::Config::load().unwrap_or_default();

    // Initialize tracing: RUST_LOG env var takes priority, then config.log_level
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.log_level)),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Daemon { foreground } => cli::daemon_cmd::run(foreground),
        Commands::Tui => cli::tui_cmd::run(),
        Commands::Status { json } => cli::status::run(json),
        Commands::Cleanup {
            dry_run,
            force,
            all,
            pids,
        } => cli::cleanup::run(dry_run, force, all, pids),
        Commands::History { count, csv } => cli::history::run(count, csv),
        Commands::Report { output } => cli::report::run(output),
        Commands::Config { action } => cli::config::run(action),
    }
}
