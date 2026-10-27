use grokbot_orb::{config::Config, daemon::Daemon};
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = match Config::from_args(std::env::args_os(), std::env::vars().collect()) {
        Ok(c) => c,
        Err(e) => {
            if let Some(c) = e.downcast_ref::<clap::Error>() {
                c.exit()
            }
            return Err(e);
        }
    };
    let level = match config.log_level.as_str() {
        "WARNING" => "warn",
        v => v,
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(level.to_lowercase())
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    let daemon = Daemon::new(config)?;
    let running = daemon.start().await?;
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    running.stop().await;
    Ok(())
}
