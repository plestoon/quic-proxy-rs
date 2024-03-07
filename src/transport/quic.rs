use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::{anyhow, Result};
use pin_project_lite::pin_project;
use quinn::{ClientConfig, Connecting, Connection, Endpoint, RecvStream, SendStream, ServerConfig, TransportConfig, VarInt};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::lookup_host;
use tokio::select;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::config::{QUIC_KEEP_ALIVE_INTERNAL, QUIC_MAX_IDLE_TIMEOUT};
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
        let mut config = ServerConfig::with_single_cert(certs, key)?;
        let mut transport_config = TransportConfig::default();
        transport_config.max_idle_timeout(Some(QUIC_MAX_IDLE_TIMEOUT.try_into()?));
        transport_config.keep_alive_interval(Some(QUIC_KEEP_ALIVE_INTERNAL));
        config.transport_config(Arc::new(transport_config));
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

        endpoint.close(VarInt::from(0_u32), &[]);
        endpoint.wait_idle().await;

        Ok(())
    }

    async fn handle_connection(conn: Connecting, stream_handler: impl StreamHandler) -> Result<()> {
        let conn = conn.await?;
        loop {
            match conn.accept_bi().await {
                Ok((send, recv)) => {
                    let transport = QuicTransport::new(send, recv);
                    let stream_handler = stream_handler.clone();
                    tokio::spawn(async move { stream_handler.handle_stream(transport).await });
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
    host: String,
    connection: Arc<Mutex<Option<Connection>>>,
    recovering: Arc<Mutex<bool>>,
}

impl QuicClient {
    pub async fn new(host: &str) -> Result<Self> {
        match Self::new_connection(host).await {
            Ok(connection) => Ok(QuicClient {
                host: host.to_string(),
                connection: Arc::new(Mutex::new(Some(connection))),
                recovering: Arc::new(Mutex::new(false)),
            }),
            _ => Ok(QuicClient {
                host: host.to_string(),
                connection: Arc::new(Mutex::new(None)),
                recovering: Arc::new(Mutex::new(false)),
            }),
        }
    }

    async fn new_connection(host: &str) -> Result<Connection> {
        let addr = lookup_host(host)
            .await?
            .find(|addr| addr.is_ipv4())
            .unwrap();
        let mut endpoint = Endpoint::client("0.0.0.0:0".parse::<SocketAddr>().unwrap())?;
        let mut config = ClientConfig::with_native_roots();
        let mut transport_config = TransportConfig::default();
        transport_config.max_idle_timeout(Some(QUIC_MAX_IDLE_TIMEOUT.try_into()?));
        transport_config.keep_alive_interval(Some(QUIC_KEEP_ALIVE_INTERNAL));
        config.transport_config(Arc::new(transport_config));
        endpoint.set_default_client_config(config);
        let (host, _) = host.split_once(':').unwrap();
        let connection = endpoint.connect(addr, host)?.await?;

        Ok(connection)
    }

    async fn recover_connection(&self) -> Result<()> {
        let mut recovering = self.recovering.lock().await;

        if !*recovering {
            *recovering = true;
            drop(recovering);

            let mut connection = self.connection.lock().await;

            match Self::new_connection(&self.host).await {
                Ok(new_connection) => {
                    let mut recovering = self.recovering.lock().await;
                    *recovering = false;

                    *connection = Some(new_connection);
                }
                Err(e) => {
                    let mut recovering = self.recovering.lock().await;
                    *recovering = false;

                    return Err(e);
                }
            }
        }

        Ok(())
    }

    async fn open_stream(&self) -> Result<QuicTransport> {
        let connection_lock = self.connection.lock().await;
        let connection = connection_lock.clone();
        drop(connection_lock);

        let connection = connection.ok_or(anyhow!("connection not established"))?;
        let (send, recv) = connection.open_bi().await?;

        Ok(QuicTransport::new(send, recv))
    }
}

impl TransportClient for QuicClient {
    #[allow(refining_impl_trait)]
    async fn new_transport(&self) -> Result<QuicTransport> {
        let transport = self.open_stream().await;

        match transport {
            Ok(transport) => Ok(transport),
            _ => {
                self.recover_connection().await?;

                self.open_stream().await
            }
        }
    }
}
