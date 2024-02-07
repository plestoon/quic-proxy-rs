use anyhow::{anyhow, Result};
use tokio::signal;

use crate::stream_handler::{StreamHandler, StreamHandlerDispatch};
use crate::transport::quic::QuicServer;
use crate::transport::tcp::TcpServer;
use crate::transport::TransportServer;

pub mod stream_handler;
pub mod transport;

pub async fn run(
    listen_addr: &str,
    transport: &str,
    passthrough_url: Option<String>,
    cert_path: Option<String>,
    key_path: Option<String>,
) -> Result<()> {
    let stream_handler = StreamHandlerDispatch::new(passthrough_url).await?;
    let server = start_server(listen_addr, transport, cert_path, key_path, stream_handler).await?;

    signal::ctrl_c().await?;
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
