use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use anyhow::{anyhow, Result};
use pin_project_lite::pin_project;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use url::Url;

use crate::transport::quic::{QuicClient, QuicTransport};
use crate::transport::tcp::{TcpClient, TcpTransport};

pub mod quic;
pub mod tcp;

pub trait Transport: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> Transport for T {}

pin_project! {
    #[project = TransportDispatchProj]
    pub enum TransportDispatch {
        TcpTransport {#[pin] transport: TcpTransport},
        QuicTransport {#[pin] transport: QuicTransport}
    }
}

impl AsyncRead for TransportDispatch {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.project() {
            TransportDispatchProj::TcpTransport { transport } => {
                AsyncRead::poll_read(transport, cx, buf)
            }
            TransportDispatchProj::QuicTransport { transport } => {
                AsyncRead::poll_read(transport, cx, buf)
            }
        }
    }
}

impl AsyncWrite for TransportDispatch {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.project() {
            TransportDispatchProj::TcpTransport { transport } => {
                AsyncWrite::poll_write(transport, cx, buf)
            }
            TransportDispatchProj::QuicTransport { transport } => {
                AsyncWrite::poll_write(transport, cx, buf)
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.project() {
            TransportDispatchProj::TcpTransport { transport } => {
                AsyncWrite::poll_flush(transport, cx)
            }
            TransportDispatchProj::QuicTransport { transport } => {
                AsyncWrite::poll_flush(transport, cx)
            }
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        match self.project() {
            TransportDispatchProj::TcpTransport { transport } => {
                AsyncWrite::poll_shutdown(transport, cx)
            }
            TransportDispatchProj::QuicTransport { transport } => {
                AsyncWrite::poll_shutdown(transport, cx)
            }
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<Result<usize, std::io::Error>> {
        match self.project() {
            TransportDispatchProj::TcpTransport { transport } => {
                AsyncWrite::poll_write_vectored(transport, cx, bufs)
            }
            TransportDispatchProj::QuicTransport { transport } => {
                AsyncWrite::poll_write_vectored(transport, cx, bufs)
            }
        }
    }

    fn is_write_vectored(&self) -> bool {
        match self {
            Self::TcpTransport { transport } => AsyncWrite::is_write_vectored(transport),
            Self::QuicTransport { transport } => AsyncWrite::is_write_vectored(transport),
        }
    }
}

pub trait TransportServer {
    fn stop(&self);
}

pub trait TransportClient: Send + Sync + Clone + 'static {
    fn new_transport(&self) -> impl Future<Output = Result<impl Transport>> + Send;
}

#[derive(Clone)]
pub enum TransportClientDispatch {
    TcpClient(TcpClient),
    QuicClient(QuicClient),
}

impl TransportClientDispatch {
    pub async fn new(url: &str) -> Result<Self> {
        let url = Url::parse(url)?;
        let scheme = url.scheme();
        let host = url.host_str().unwrap();
        match scheme {
            "tcp" => Ok(Self::TcpClient(TcpClient::new(host))),
            "quic" => Ok(Self::QuicClient(QuicClient::new(host).await?)),
            _ => Err(anyhow!("unknown scheme: {}", scheme)),
        }
    }
}

impl TransportClient for TransportClientDispatch {
    async fn new_transport(&self) -> Result<impl Transport> {
        match self {
            Self::TcpClient(client) => Ok(TransportDispatch::TcpTransport {
                transport: client.new_transport().await?,
            }),
            Self::QuicClient(client) => Ok(TransportDispatch::QuicTransport {
                transport: client.new_transport().await?,
            }),
        }
    }
}
