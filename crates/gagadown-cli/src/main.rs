use anyhow::Result;
use clap::{Parser, Subcommand};
use gagadown_core::{AddRequest, Engine, Status};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "gagadown", version, about = "GagaDown command line")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Download one URL and exit
    Get {
        url: String,
        #[arg(short, long)]
        dir: Option<PathBuf>,
        #[arg(short = 'c', long, default_value_t = 64)]
        max_connections: usize,
        #[arg(long)]
        initial: Option<usize>,
        #[arg(long)]
        proxy: Vec<String>,
        #[arg(long)]
        direct_only: bool,
        #[arg(long)]
        sha256: Option<String>,
        /// Isolated data dir (default: temp, so CLI runs don't touch the GUI state)
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Run the engine headless with the browser API
    Serve {
        #[arg(long)]
        port: Option<u16>,
    },
    /// Probe local proxy ports
    Proxies,
}

fn human(b: f64) -> String {
    let u = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b;
    let mut i = 0;
    while v >= 1024.0 && i < u.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", u[i])
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Get { url, dir, max_connections, initial, proxy, direct_only, sha256, data_dir } => {
            let data_dir = data_dir.unwrap_or_else(|| std::env::temp_dir().join(format!("gagadown-cli-{}", std::process::id())));
            let engine = Engine::new(Some(data_dir.clone()))?;
            let mut s = engine.settings();
            s.max_connections_per_task = max_connections;
            if let Some(i) = initial {
                s.initial_connections = i;
            }
            s.proxy.proxies = proxy;
            if direct_only {
                s.proxy.mode = gagadown_core::ProxyMode::DirectOnly;
                s.proxy.auto_detect = false;
            }
            if let Some(d) = &dir {
                s.download_dir = d.clone();
            }
            engine.set_settings(s)?;
            let out = engine.add(AddRequest { url, sha256, ..Default::default() }, None)?;
            let t0 = Instant::now();
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let Some(v) = engine.views(0).into_iter().find(|v| v.id == out.id) else { break };
                let pct = v.size.map(|s| if s > 0 { v.downloaded as f64 * 100.0 / s as f64 } else { 100.0 }).unwrap_or(0.0);
                eprint!(
                    "\r{:>6.2}%  {:>10}/s  conns {:>3}/{:<3} splits {:<5} {:<12}",
                    pct,
                    human(v.speed),
                    v.connections,
                    v.target,
                    v.splits,
                    v.route.clone().unwrap_or_default()
                );
                match v.status {
                    Status::Completed => {
                        let secs = t0.elapsed().as_secs_f64();
                        eprintln!("\ndone {} in {:.2}s, avg {}/s -> {}", human(v.downloaded as f64), secs, human(v.downloaded as f64 / secs), v.path.map(|p| p.display().to_string()).unwrap_or_default());
                        break;
                    }
                    Status::Failed if v.next_retry_at.is_none() => {
                        let e = v.error.unwrap();
                        eprintln!("\nfailed: {} {}", e.kind.label(), e.message);
                        engine.shutdown().await;
                        let _ = std::fs::remove_dir_all(&data_dir);
                        std::process::exit(1);
                    }
                    _ => {}
                }
            }
            engine.shutdown().await;
            let _ = std::fs::remove_dir_all(&data_dir);
        }
        Cmd::Serve { port } => {
            let engine = Engine::new(None)?;
            let port = port.unwrap_or(engine.settings().api_port);
            eprintln!("api on 127.0.0.1:{port}");
            gagadown_core::api::serve(engine, port).await?;
        }
        Cmd::Proxies => {
            for p in gagadown_core::route::detect_local_proxies().await {
                println!("{p}");
            }
        }
    }
    Ok(())
}
