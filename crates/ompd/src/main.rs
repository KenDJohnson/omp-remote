use std::{
    net::SocketAddr,
    num::NonZeroU64,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};

use clap::{Args, Parser, Subcommand, ValueEnum};
use omp_control_protocol::DeviceScopes;
use omp_runtime::RuntimeConfig;
use ompd::{
    DaemonServer, ServerSessionConfig, TlsMode, TransportConfig, persistence::Store,
    request_pairing, serve_admin_socket, unix_time_ms,
};
use parking_lot::Mutex;

#[derive(Debug, Parser)]
#[command(name = "ompd", version, about = "Secure remote control daemon for OMP")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the network service and owner-only local administration socket.
    Serve(ServeArgs),
    /// Create a one-time pairing QR code through the running daemon.
    Pair(PairArgs),
}

#[derive(Debug, Args)]
struct ServeArgs {
    /// SQLite state database.
    #[arg(long, env = "OMPD_DATABASE")]
    database: PathBuf,
    /// Owner-only Unix socket used by local administration commands.
    #[arg(long, env = "OMPD_ADMIN_SOCKET")]
    admin_socket: PathBuf,
    /// OMP executable to supervise.
    #[arg(long, env = "OMPD_OMP", default_value = "omp")]
    omp: PathBuf,
    /// TCP listener address.
    #[arg(long, env = "OMPD_BIND")]
    bind: SocketAddr,
    /// Client-visible WebSocket URL ending in /control.
    #[arg(long, env = "OMPD_PUBLIC_ENDPOINT")]
    public_endpoint: String,
    /// TLS deployment model. Plaintext requires an explicit development mode.
    #[arg(long, env = "OMPD_TLS_MODE", value_enum)]
    tls_mode: TlsModeArg,
    /// PEM certificate chain for direct TLS modes.
    #[arg(long, env = "OMPD_TLS_CERT")]
    tls_certificate: Option<PathBuf>,
    /// PEM private key for direct TLS modes.
    #[arg(long, env = "OMPD_TLS_KEY")]
    tls_private_key: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum TlsModeArg {
    Certificate,
    TrustedReverseProxy,
    PinnedSelfSigned,
    DevelopmentPlaintext,
}

#[derive(Debug, Args)]
struct PairArgs {
    /// Owner-only Unix socket exposed by the running daemon.
    #[arg(long, env = "OMPD_ADMIN_SOCKET")]
    admin_socket: PathBuf,
    /// Human-readable name assigned to the new device.
    #[arg(long)]
    name: String,
    /// Lifetime such as 30s, 10m, or 1h.
    #[arg(long, default_value = "10m", value_parser = parse_duration_ms)]
    expires: NonZeroU64,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Serve(args) => serve(args).await,
        Command::Pair(args) => {
            let links = request_pairing(
                args.admin_socket,
                args.name,
                args.expires,
                DeviceScopes::all(),
            )
            .await?;
            print!("{}", links.human_output());
            Ok(())
        }
    }
}

async fn serve(args: ServeArgs) -> Result<(), Box<dyn std::error::Error>> {
    create_parent(&args.database)?;
    create_parent(&args.admin_socket)?;
    let tls_mode = build_tls_mode(&args)?;
    let transport = TransportConfig {
        bind_address: args.bind,
        public_endpoint: args.public_endpoint,
        tls_mode,
    };
    transport.validate()?;
    let store = Arc::new(Mutex::new(Store::open_at(&args.database, unix_time_ms())?));
    let server = DaemonServer::new(
        transport.clone(),
        ServerSessionConfig::default(),
        Arc::clone(&store),
        RuntimeConfig::new(args.omp),
    )?;
    let controller = server.controller().clone();
    let service_result: Result<(), Box<dyn std::error::Error>> = tokio::select! {
        result = server.serve() => result.map_err(Into::into),
        result = serve_admin_socket(args.admin_socket, store, transport) => result.map_err(Into::into),
        signal = tokio::signal::ctrl_c() => signal.map_err(Into::into),
    };
    let shutdown_result = controller.shutdown().await;
    service_result?;
    shutdown_result?;
    Ok(())
}

fn build_tls_mode(args: &ServeArgs) -> Result<TlsMode, String> {
    let certificate_files = || {
        let certificate = args
            .tls_certificate
            .clone()
            .ok_or_else(|| "--tls-certificate is required for direct TLS modes".to_owned())?;
        let private_key = args
            .tls_private_key
            .clone()
            .ok_or_else(|| "--tls-private-key is required for direct TLS modes".to_owned())?;
        Ok::<_, String>((certificate, private_key))
    };
    let reject_certificate_files = || {
        if args.tls_certificate.is_some() || args.tls_private_key.is_some() {
            Err("--tls-certificate and --tls-private-key apply only to direct TLS modes".to_owned())
        } else {
            Ok(())
        }
    };
    match args.tls_mode {
        TlsModeArg::Certificate => {
            let (certificate, private_key) = certificate_files()?;
            Ok(TlsMode::CertificateFiles {
                certificate,
                private_key,
            })
        }
        TlsModeArg::PinnedSelfSigned => {
            let (certificate, private_key) = certificate_files()?;
            Ok(TlsMode::PinnedSelfSigned {
                certificate,
                private_key,
            })
        }
        TlsModeArg::TrustedReverseProxy => {
            reject_certificate_files()?;
            Ok(TlsMode::TrustedReverseProxy {
                local_endpoint: args.bind,
            })
        }
        TlsModeArg::DevelopmentPlaintext => {
            reject_certificate_files()?;
            Ok(TlsMode::DevelopmentPlaintext {
                local_endpoint: args.bind,
            })
        }
    }
}

fn parse_duration_ms(value: &str) -> Result<NonZeroU64, String> {
    let (number, multiplier) = if let Some(value) = value.strip_suffix("ms") {
        (value, 1)
    } else if let Some(value) = value.strip_suffix('s') {
        (value, 1_000)
    } else if let Some(value) = value.strip_suffix('m') {
        (value, 60_000)
    } else if let Some(value) = value.strip_suffix('h') {
        (value, 3_600_000)
    } else {
        return Err("duration must end in ms, s, m, or h (for example 10m)".into());
    };
    let number: u64 = number
        .parse()
        .map_err(|_| "duration must contain a positive integer".to_owned())?;
    let milliseconds = number
        .checked_mul(multiplier)
        .ok_or_else(|| "duration is too large".to_owned())?;
    NonZeroU64::new(milliseconds).ok_or_else(|| "duration must be greater than zero".to_owned())
}

fn create_parent(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}
