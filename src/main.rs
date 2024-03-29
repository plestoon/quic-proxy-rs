use anyhow::Result;
use clap::Parser;

use quic_proxy::run;
use quic_proxy::stream_handler::http_proxy::AuthConfig;

/// An HTTP Proxy over QUIC
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(short, long)]
    listen_addr: String,

    #[arg(short, long)]
    transport: String,

    #[arg(short, long)]
    passthrough_url: Option<String>,

    #[arg(short, long)]
    cert_path: Option<String>,

    #[arg(short, long)]
    key_path: Option<String>,

    #[arg(short, long)]
    auth: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let auth_config = args.auth.map(|auth| {
        let (username, password) = auth.split_once(':').unwrap();
        AuthConfig::new(username.into(), password.into())
    });
    run(
        args.listen_addr.as_str(),
        args.transport.as_str(),
        args.passthrough_url,
        args.cert_path,
        args.key_path,
        auth_config,
    )
        .await
}
