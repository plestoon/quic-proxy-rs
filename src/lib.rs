use anyhow::{anyhow, Result};
use tokio::signal;

use crate::stream_handler::http_proxy::AuthConfig;
use crate::stream_handler::{StreamHandler, StreamHandlerDispatch};
use crate::transport::quic::QuicServer;
use crate::transport::tcp::TcpServer;
use crate::transport::TransportServer;

pub mod config;
pub mod stream_handler;
pub mod transport;

pub async fn run(
    listen_addr: &str,
    transport: &str,
    passthrough_url: Option<String>,
    cert_path: Option<String>,
    key_path: Option<String>,
    auth_config: Option<AuthConfig>,
) -> Result<()> {
    let stream_handler = StreamHandlerDispatch::new(passthrough_url, auth_config).await?;
    let server = start_server(listen_addr, transport, cert_path, key_path, stream_handler).await?;

    let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = signal::ctrl_c() => {}
        _ = sigterm.recv() => {}
    }

    server.stop();

    Ok(())
}

async fn start_server(
    listen_addr: &str,
    transport: &str,
    cert_path: Option<String>,
    key_path: Option<String>,
    stream_handler: impl StreamHandler,
) -> Result<Box<dyn TransportServer>> {
    match transport {
        "tcp" => Ok(Box::new(
            TcpServer::start(listen_addr, stream_handler).await?,
        )),
        "quic" => Ok(Box::new(
            QuicServer::start(
                listen_addr,
                cert_path.unwrap().as_str(),
                key_path.unwrap().as_str(),
                stream_handler,
            )
            .await?,
        )),
        _ => Err(anyhow!("unknown transport: {}", transport)),
    }
}
