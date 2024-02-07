use anyhow::Result;
use tokio::net::{TcpListener, TcpStream};
use tokio::select;
use tokio_util::sync::CancellationToken;

use crate::stream_handler::StreamHandler;

use super::{TransportClient, TransportServer};

pub type TcpTransport = TcpStream;

pub struct TcpServer {
    cancellation_token: CancellationToken,
}

impl TcpServer {
    pub async fn start(listen_addr: &str, stream_handler: impl StreamHandler) -> Result<TcpServer> {
        let cancellation_token = CancellationToken::new();
        {
            let listen_addr = listen_addr.to_string();
            let cancellation_token = cancellation_token.clone();
            tokio::spawn(async move {
                Self::serve(listen_addr, stream_handler, cancellation_token)
                    .await
                    .unwrap()
            });
        }

        Ok(TcpServer { cancellation_token })
    }

    async fn serve(
        listen_addr: String,
        stream_handler: impl StreamHandler,
        cancellation_token: CancellationToken,
    ) -> Result<()> {
        let listener = TcpListener::bind(listen_addr).await?;

        loop {
            select! {
                _ = cancellation_token.cancelled() => {
                    break
                }
                result = listener.accept() => {
                    let (socket, _) = result?;
                    let stream_handler = stream_handler.clone();
                    tokio::spawn(async move { stream_handler.handle_stream(socket).await });
                }
            }
        }

        Ok(())
    }
}

impl TransportServer for TcpServer {
    fn stop(&self) {
        self.cancellation_token.cancel();
    }
}

#[derive(Clone)]
pub struct TcpClient {
    pub remote_addr: String,
}

impl TcpClient {
    pub fn new(remote_addr: &str) -> TcpClient {
        TcpClient {
            remote_addr: remote_addr.to_string(),
        }
    }
}

impl TransportClient for TcpClient {
    #[allow(refining_impl_trait)]
    async fn new_transport(&self) -> Result<TcpTransport> {
        Ok(TcpStream::connect(&self.remote_addr).await?)
    }
}
