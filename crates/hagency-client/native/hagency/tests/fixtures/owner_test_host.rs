//! Isolated subprocess for owner state, restart and HTTP API integration tests.
use clap::{Parser, Subcommand};
use std::{net::SocketAddr, path::PathBuf};
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Init {
        #[arg(long)]
        state_dir: PathBuf,
    },
    Serve {
        #[arg(long)]
        state_dir: PathBuf,
        #[arg(long)]
        listen: SocketAddr,
    },
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if let Some(result) = hagency::native_owner::run_guardian_if_requested() {
        return result.map_err(Into::into);
    }
    let cli = Cli::parse();
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async {
        match cli.command {
            Command::Init { state_dir } => {
                hagency::owner_host::initialize(&state_dir)?;
            }
            Command::Serve { state_dir, listen } => {
                let host = hagency::owner_host::OwnerHost::open(&state_dir, listen)?;
                let cancel = hagency_matrix::CancellationToken::new();
                let signal = cancel.clone();
                tokio::spawn(async move {
                    #[cfg(unix)]
                    {
                        let mut term = tokio::signal::unix::signal(
                            tokio::signal::unix::SignalKind::terminate(),
                        )
                        .unwrap();
                        tokio::select! { _=term.recv()=>{}, _=tokio::signal::ctrl_c()=>{} }
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = tokio::signal::ctrl_c().await;
                    }
                    signal.cancel();
                });
                host.serve(listen, &cancel).await?;
            }
        }
        Ok(())
    })
}
