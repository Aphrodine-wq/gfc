use std::path::PathBuf;
use std::process::Command;

use clap::{Parser, Subcommand};
use gfc_cache::Cache;
use gfc_config::{Config, Paths};
use gfc_daemon::{is_running, Daemon};
use gfc_git::scan_inventory;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "gfc", about = "Git Forge Cockpit — local-first Git inventory TUI")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan repositories and print inventory JSON
    Scan {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        roots: Vec<PathBuf>,
    },
    /// Background daemon
    Daemon {
        #[command(subcommand)]
        action: DaemonCmd,
    },
    /// Show or initialize configuration
    Config {
        #[arg(long)]
        init: bool,
        #[arg(long)]
        path: bool,
    },
}

#[derive(Subcommand)]
enum DaemonCmd {
    /// Run the daemon in the foreground
    Run,
    /// Install a systemd --user unit
    Install,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("gfc=info".parse()?))
        .init();
    let cli = Cli::parse();
    match cli.command {
        None => {
            maybe_start_daemon()?;
            gfc_tui::run().map_err(|e| anyhow::anyhow!(e))?;
            Ok(())
        }
        Some(Commands::Scan { json, roots }) => {
            let paths = Paths::resolve()?;
            let mut config = Config::load_or_default(&paths.config_file)?;
            if !roots.is_empty() {
                config.roots = roots;
            }
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            let inventory = rt.block_on(scan_inventory(&config, &[]));
            if !json {
                eprintln!("gfc scan always emits JSON on stdout");
            }
            println!("{}", serde_json::to_string_pretty(&inventory)?);
            Ok(())
        }
        Some(Commands::Daemon { action }) => match action {
            DaemonCmd::Run => run_daemon(),
            DaemonCmd::Install => {
                let exe = std::env::current_exe()?;
                let path = gfc_daemon::systemd::install_user_unit(&exe)?;
                println!("wrote {}", path.display());
                println!("enable with: systemctl --user enable --now gfc.service");
                Ok(())
            }
        },
        Some(Commands::Config { init, path }) => {
            let paths = Paths::resolve()?;
            if path {
                println!("{}", paths.config_file.display());
            }
            if init {
                paths.ensure_dirs()?;
                if !paths.config_file.exists() {
                    Config::default().save(&paths.config_file)?;
                    println!("wrote {}", paths.config_file.display());
                } else {
                    println!("exists {}", paths.config_file.display());
                }
            }
            if !init && !path {
                let cfg = Config::load_or_default(&paths.config_file)?;
                print!("{}", toml_pretty(&cfg)?);
            }
            Ok(())
        }
    }
}

fn toml_pretty(cfg: &Config) -> anyhow::Result<String> {
    Ok(toml::to_string_pretty(cfg)?)
}

fn run_daemon() -> anyhow::Result<()> {
    let paths = Paths::resolve()?;
    paths.ensure_dirs()?;
    let config = Config::load_or_default(&paths.config_file)?;
    let cache = Cache::open(&paths.cache_file)?;
    let socket = paths.socket_file.clone();
    let daemon = Daemon::new(paths, config, cache);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(daemon.run(socket))?;
    Ok(())
}

fn maybe_start_daemon() -> anyhow::Result<()> {
    let paths = Paths::resolve()?;
    paths.ensure_dirs()?;
    if is_running(&paths.socket_file) {
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    Command::new(exe).args(["daemon", "run"]).spawn()?;
    std::thread::sleep(std::time::Duration::from_millis(80));
    Ok(())
}
