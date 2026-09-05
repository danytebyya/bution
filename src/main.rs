use anyhow::Result;
use bution::cluster::NodeRole;
use bution::models::ModelInfo;
use bution::tui::App;
use clap::{Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "bution",
    version,
    about = "Distributed Local AI Cluster powered by llama.cpp RPC"
)]
struct Cli {
    /// Select a local GGUF model.
    #[arg(long, value_name = "FILE")]
    model: Option<PathBuf>,

    /// Directory containing llama-server, rpc-server, and llama-bench.
    #[arg(long, value_name = "DIR")]
    llama_bin_dir: Option<PathBuf>,

    /// Override the persisted node role.
    #[arg(long, value_enum)]
    role: Option<RoleArg>,

    /// Check for updates and update BUTION to the latest version.
    #[arg(long, short = 'u')]
    update: bool,

    /// Skip checking for updates on startup.
    #[arg(long)]
    no_update_check: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum RoleArg {
    Automatic,
    Main,
    Worker,
}

impl From<RoleArg> for NodeRole {
    fn from(role: RoleArg) -> Self {
        match role {
            RoleArg::Automatic => Self::Automatic,
            RoleArg::Main => Self::Main,
            RoleArg::Worker => Self::Worker,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.update {
        return bution::update::run_cli_update().await;
    }
    if !cli.no_update_check && bution::update::auto_update_on_startup_if_needed().await? {
        return Ok(());
    }
    let mut app = App::load()?;
    let mut settings_changed = false;

    if let Some(model_path) = cli.model {
        let model = ModelInfo::inspect(&model_path)?;
        app.model = Some(model);
    }
    if let Some(directory) = cli.llama_bin_dir {
        app.settings.llama_bin_dir = Some(directory);
        settings_changed = true;
    }
    if let Some(role) = cli.role {
        app.settings.role = role.into();
        if let Some(local) = app.nodes.first_mut() {
            local.role = app.settings.role;
        }
        settings_changed = true;
    }
    if settings_changed {
        app.settings.save(&app.paths)?;
    }
    bution::tui::run(app).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn command_line_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
