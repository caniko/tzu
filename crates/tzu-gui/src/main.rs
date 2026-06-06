#[cfg(feature = "ssr")]
use std::net::IpAddr;
#[cfg(feature = "ssr")]
use std::path::PathBuf;

#[cfg(feature = "ssr")]
use anyhow::Result;
#[cfg(feature = "ssr")]
use clap::Parser;
#[cfg(feature = "ssr")]
use leptos::config::get_configuration;
#[cfg(feature = "ssr")]
use tzu_gui::server::{GuiConfig, build_state, router};

#[cfg(feature = "ssr")]
#[derive(Debug, Parser)]
#[command(name = "tzu-gui")]
#[command(about = "Local Leptos GUI for the tzu planning harness")]
struct Cli {
    #[arg(long, default_value = "127.0.0.1")]
    host: IpAddr,
    #[arg(long, default_value_t = 7070)]
    port: u16,
    #[arg(long, default_value = ".")]
    project_root: PathBuf,
    #[arg(long, env = "TZU_DATABASE_URL")]
    database_url: Option<String>,
}

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let config = GuiConfig {
        host: cli.host,
        port: cli.port,
        project_root: cli.project_root,
        database_url: cli.database_url,
    };
    let mut leptos_options = get_configuration(None)?.leptos_options;
    leptos_options.site_addr = config.addr();
    let state = build_state(&config).await?;
    let app = router(state, leptos_options);
    let listener = tokio::net::TcpListener::bind(config.addr()).await?;
    tracing::info!("serving tzu GUI at http://{}", config.addr());
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

#[cfg(not(feature = "ssr"))]
pub fn main() {}
