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
use tzu_config::load_config;
#[cfg(feature = "ssr")]
use tzu_gui::server::{GuiConfig, build_state, router};

#[cfg(feature = "ssr")]
#[derive(Debug, Parser)]
#[command(name = "tzu-gui")]
#[command(about = "Local Leptos GUI for the tzu planning harness")]
struct Cli {
    #[arg(long)]
    host: Option<IpAddr>,
    #[arg(long)]
    port: Option<u16>,
    #[arg(long)]
    project_root: Option<PathBuf>,
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
    let app_config = load_config()?;
    let config = GuiConfig {
        host: cli.host.unwrap_or_else(|| {
            app_config
                .gui
                .host
                .parse()
                .unwrap_or_else(|_| "127.0.0.1".parse().expect("default GUI host is valid"))
        }),
        port: cli.port.unwrap_or(app_config.gui.port),
        project_root: cli
            .project_root
            .or_else(|| app_config.projects_directory.clone())
            .unwrap_or_else(|| PathBuf::from(".")),
        database_url: cli.database_url,
    };
    let mut leptos_options = leptos_options()?;
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

#[cfg(feature = "ssr")]
fn leptos_options() -> Result<leptos::config::LeptosOptions> {
    if std::env::var_os("LEPTOS_OUTPUT_NAME").is_some() {
        return Ok(get_configuration(None)?.leptos_options);
    }

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    Ok(get_configuration(Some(
        manifest
            .to_str()
            .expect("GUI Cargo.toml path must be valid UTF-8"),
    ))?
    .leptos_options)
}
