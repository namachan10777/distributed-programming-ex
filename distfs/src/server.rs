use std::path::{Path, PathBuf};

use super::proto;
struct Server {
    root_path: PathBuf,
}

impl Server {
    fn real_path<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        let path = path.as_ref();
        self.root_path.join(path)
    }
}

#[async_trait::async_trait]
impl proto::fs::filesystem_server::Filesystem for Server {
    async fn lookup(
        &self,
        req: tonic::Request<proto::fs::LookupRequest>,
    ) -> Result<tonic::Response<proto::fs::LookupResponse>, tonic::Status> {
        let req = req.into_inner();
        let parent = req.parent;
        let name = req.name;
        unimplemented!()
    }
}
