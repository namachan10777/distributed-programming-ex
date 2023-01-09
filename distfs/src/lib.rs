pub mod client;
pub mod server;
pub mod proto {
    pub mod fs {
        use tonic::include_proto;
        include_proto!("fs");
    }
}
