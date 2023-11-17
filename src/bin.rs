use anyhow::anyhow;
use tonic::{transport::Server, Request, Response, Status};

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use yars::embedding::DummyEmbeddingProvider;
use yars::semantic_index::SemanticIndex;
use yars_grpc::yars_server::{Yars, YarsServer};
use yars_grpc::{IndexReply, IndexRequest};

pub mod yars_grpc {
    tonic::include_proto!("yars_grpc");
}

pub struct YarsAgent {
    index: Arc<Mutex<SemanticIndex>>,
}

impl YarsAgent {
    pub async fn new() -> anyhow::Result<Self> {
        let index = Arc::new(Mutex::new(
            SemanticIndex::new(
                PathBuf::from("data/db"),
                Arc::new(DummyEmbeddingProvider {}),
            )
            .await?,
        ));
        anyhow::Ok(YarsAgent { index })
    }
}

#[tonic::async_trait]
impl Yars for YarsAgent {
    async fn index_directory(
        &self,
        request: Request<IndexRequest>, // Accept request of type HelloRequest
    ) -> Result<Response<IndexReply>, Status> {
        let mut index = self.index.lock().await;

        let path = PathBuf::from(request.into_inner().path);
        let indexing = index.index_directory(path.clone()).await;
        let reply = match indexing {
            Ok(_) => IndexReply {
                status: format!("Indexing {:?}", path).into(),
            },
            Err(err) => IndexReply {
                status: format!("Failed to start Indexing: {:?}", err),
            },
        };

        Ok(Response::new(reply)) // Send back our formatted greeting
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    simple_logger::init_with_env().unwrap();

    let addr = "[::1]:50051".parse()?;
    let agent = YarsAgent::new().await?;

    Server::builder()
        .add_service(YarsServer::new(agent))
        .serve(addr)
        .await?;

    Ok(())
}
