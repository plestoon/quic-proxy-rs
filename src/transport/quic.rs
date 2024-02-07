use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::Result;
use pin_project_lite::pin_project;
use quinn::{
    ClientConfig, Connecting, Connection, Endpoint, RecvStream, SendStream, ServerConfig,
    TransportConfig,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::lookup_host;
use tokio::select;
use tokio_util::sync::CancellationToken;

use crate::stream_handler::StreamHandler;
use crate::transport::{TransportClient, TransportServer};

pin_project! {
    pub struct QuicTransport {
        #[pin]
        send_stream: SendStream,
        #[pin]
        recv_stream: RecvStream,
    }
}

impl QuicTransport {
    fn new(send_stream: SendStream, recv_stream: RecvStream) -> Self {
        QuicTransport {
            send_stream,
            recv_stream,
        }
    }
}

impl AsyncRead for QuicTransport {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        AsyncRead::poll_read(self.project().recv_stream, cx, buf)
    }
}

impl AsyncWrite for QuicTransport {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        AsyncWrite::poll_write(self.project().send_stream, cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        AsyncWrite::poll_flush(self.project().send_stream, cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        AsyncWrite::poll_shutdown(self.project().send_stream, cx)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        AsyncWrite::poll_write_vectored(self.project().send_stream, cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        AsyncWrite::is_write_vectored(&self.send_stream)
    }
}

pub struct QuicServer {
    cancellation_token: CancellationToken,
}

impl QuicServer {
    pub async fn start(
        listen_addr: &str,
        cert_path: &str,
        key_path: &str,
        stream_handler: impl StreamHandler,
    ) -> Result<Self> {
        let cancellation_token = CancellationToken::new();
        {
            let listen_addr = listen_addr.to_string();
            let cert_path = cert_path.to_string();
            let key_path = key_path.to_string();
            let cancellation_token = cancellation_token.clone();
            tokio::spawn(async move {
                Self::serve(
                    listen_addr,
                    cert_path,
                    key_path,
                    stream_handler,
                    cancellation_token,
                )
                .await
                .unwrap()
            });
        }

        Ok(QuicServer { cancellation_token })
    }

    async fn serve(
        listen_addr: String,
        cert_path: String,
        key_path: String,
        stream_handler: impl StreamHandler,
        cancellation_token: CancellationToken,
    ) -> Result<()> {
        let mut reader = BufReader::new(File::open(cert_path)?);
        let certs = rustls_pemfile::certs(&mut reader)?
            .into_iter()
            .map(rustls::Certificate)
            .collect();
        let mut reader = BufReader::new(File::open(key_path)?);
        let mut keys = rustls_pemfile::pkcs8_private_keys(&mut reader)?;
        let key = rustls::PrivateKey(keys.remove(0));
        let config = ServerConfig::with_single_cert(certs, key)?;
        let endpoint = Endpoint::server(config, listen_addr.parse::<SocketAddr>().unwrap())?;

        loop {
            select! {
                _ = cancellation_token.cancelled() => {
                    break
                }
                result = endpoint.accept() => {
                    match result {
                        Some(conn) => {
                            let stream_handler = stream_handler.clone();
                            tokio::spawn(async move {
                                Self::handle_connection(conn, stream_handler).await.unwrap()
                            });
                        }
                        _ => {
                            println!("not accepting connections");

                            break
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_connection(conn: Connecting, stream_handler: impl StreamHandler) -> Result<()> {
        let conn = conn.await?;
        loop {
            match conn.accept_bi().await {
                Ok((send, recv)) => {
                    let transport = QuicTransport::new(send, recv);
                    let stream_handler = stream_handler.clone();
                    tokio::spawn(
                        async move { stream_handler.handle_stream(transport).await.unwrap() },
                    );
                }
                Err(_) => {
                    break;
                }
            }
        }

        Ok(())
    }
}

impl TransportServer for QuicServer {
    fn stop(&self) {
        self.cancellation_token.cancel();
    }
}

#[derive(Clone)]
pub struct QuicClient {
    connection: Connection,
}

impl QuicClient {
    pub async fn new(host: &str) -> Result<Self> {
        let addr = lookup_host(host)
            .await?
            .find(|addr| addr.is_ipv4())
            .unwrap();
        let mut endpoint = Endpoint::client("0.0.0.0:0".parse::<SocketAddr>().unwrap())?;
        let mut config = ClientConfig::with_native_roots();
        let mut transport_config = TransportConfig::default();
        transport_config.keep_alive_interval(Some(Duration::from_secs(5)));
        config.transport_config(Arc::new(transport_config));
        endpoint.set_default_client_config(config);
        let connection = endpoint.connect(addr, "localhost")?.await?;

        Ok(QuicClient { connection })
    }
}

impl TransportClient for QuicClient {
    #[allow(refining_impl_trait)]
    async fn new_transport(&self) -> Result<QuicTransport> {
        let (send, recv) = self.connection.open_bi().await?;
        Ok(QuicTransport::new(send, recv))
    }
}
