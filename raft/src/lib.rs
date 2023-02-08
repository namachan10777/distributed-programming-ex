pub mod server;
use std::io;

use server::types::Log;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("init raft server {0}")]
    Init(io::Error),
    #[error("save persistent state {0}")]
    SavePersistent(io::Error),
}

#[async_trait::async_trait]
pub trait LogConsumer {
    type Target;
    async fn recv(&self, log: Log<Self::Target>) -> Result<(), Error>;
}
