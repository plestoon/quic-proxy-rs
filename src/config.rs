use std::time::Duration;

pub const QUIC_MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
pub const QUIC_KEEP_ALIVE_INTERNAL: Duration = Duration::from_secs(5);