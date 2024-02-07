use std::future::Future;

use anyhow::Result;

use crate::stream_handler::http_proxy::HttpProxy;
use crate::stream_handler::passthrough::Passthrough;
use crate::transport::{Transport, TransportClientDispatch};

pub mod http_proxy;
pub mod passthrough;

pub trait StreamHandler: Send + Clone + 'static {
    fn handle_stream(&self, stream: impl Transport) -> impl Future<Output = Result<()>> + Send;
}

#[derive(Clone)]
pub enum StreamHandlerDispatch {
    HttpProxy(HttpProxy),
    Passthrough(Passthrough<TransportClientDispatch>),
}

impl StreamHandlerDispatch {
    pub async fn new(passthrough_url: Option<String>) -> Result<Self> {
        if let Some(passthrough_url) = passthrough_url {
            let transport_client = TransportClientDispatch::new(&passthrough_url).await?;
            let passthrough = Passthrough::new(transport_client);

            Ok(Self::Passthrough(passthrough))
        } else {
            Ok(Self::HttpProxy(HttpProxy::new()))
        }
    }
}

impl StreamHandler for StreamHandlerDispatch {
    async fn handle_stream(&self, stream: impl Transport) -> Result<()> {
        match self {
            Self::HttpProxy(http_proxy) => http_proxy.handle_stream(stream).await,
            Self::Passthrough(passthrough) => passthrough.handle_stream(stream).await,
        }
    }
}
