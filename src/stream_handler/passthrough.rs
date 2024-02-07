use anyhow::Result;
use tokio::io::copy_bidirectional;

use crate::stream_handler::StreamHandler;
use crate::transport::{Transport, TransportClient};

#[derive(Debug, Clone)]
pub struct Passthrough<T: TransportClient> {
    transport_client: T,
}

impl<T: TransportClient> Passthrough<T> {
    pub fn new(transport_client: T) -> Self {
        Passthrough { transport_client }
    }
}

impl<T: TransportClient> StreamHandler for Passthrough<T> {
    async fn handle_stream(&self, mut inbound: impl Transport) -> Result<()> {
        let mut outbound = self.transport_client.new_transport().await?;
        copy_bidirectional(&mut inbound, &mut outbound).await?;

        Ok(())
    }
}
