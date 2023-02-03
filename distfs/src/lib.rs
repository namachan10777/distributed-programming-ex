#![feature(map_many_mut)]
pub mod client;
pub mod server;
pub mod type_conv;
pub mod proto {
    pub mod fs {
        tonic::include_proto!("fs");
    }
}
