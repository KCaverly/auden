use anyhow::anyhow;
use homedir::get_my_home;
use tonic::{transport::Server, Request, Response, Status};

use auden::semantic_index::IndexingStatus;
use auden::semantic_index::SemanticIndex;
use auden_grpc::auden_server::{Auden, AudenServer};
use auden_grpc::{
    IndexReply, IndexRequest, SearchReply, SearchRequest, SearchResultReply, StatusReply,
    StatusRequest,
};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

pub mod auden_grpc {
    tonic::include_proto!("auden_grpc");
}

pub struct AudenAgent {
    index: Arc<Mutex<SemanticIndex>>,
}

impl AudenAgent {
    pub async fn new() -> anyhow::Result<Self> {
        let database_dir = get_my_home()?
            .ok_or(anyhow!("cant find home directory"))?
            .as_path()
            .join(".auden")
            .join("db");

        let index = Arc::new(Mutex::new(SemanticIndex::new(database_dir).await?));
        anyhow::Ok(AudenAgent { index })
    }
}

#[tonic::async_trait]
impl Auden for AudenAgent {
    async fn index_directory(
        &self,
        request: Request<IndexRequest>,
    ) -> Result<Response<IndexReply>, Status> {
        let mut index = self.index.lock().await;

        let path = PathBuf::from(request.into_inner().path);
        let indexing = index.index_directory(path.clone()).await;
        let reply = match indexing {
            Ok(_) => IndexReply {
                code: 0,
                status: format!("Indexing {:?}", path).into(),
            },
            Err(err) => IndexReply {
                code: 1,
                status: format!("Failed to start Indexing: {:?}", err),
            },
        };

        Ok(Response::new(reply))
    }

    async fn indexing_status(
        &self,
        request: Request<StatusRequest>,
    ) -> Result<Response<StatusReply>, Status> {
        let index = self.index.lock().await;

        let path = PathBuf::from(request.into_inner().path);
        let status = index.get_status(path.clone()).await;

        let reply = match status {
            IndexingStatus::Indexing { jobs_outstanding } => StatusReply {
                status: status.to_string(),
                outstanding: jobs_outstanding as i32,
            },
            _ => StatusReply {
                status: status.to_string(),
                outstanding: 0,
            },
        };

        Ok(Response::new(reply))
    }

    async fn search_directory(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchReply>, Status> {
        let index = self.index.lock().await;
        let request = request.into_inner();
        let path = PathBuf::from(request.path);
        let n = request.n as usize;
        let search_query = request.query;

        let search_results = index.search_directory(path, n, search_query.as_str()).await;
        let reply = match search_results {
            Ok(results) => {
                let search_results = results
                    .iter()
                    .map(|result| SearchResultReply {
                        id: result.id.id.to_string(),
                        start_byte: result.start_byte as i32,
                        end_byte: result.end_byte as i32,
                        path: result.path.to_string_lossy().to_string(),
                    })
                    .collect::<Vec<SearchResultReply>>();

                SearchReply {
                    code: 0,
                    message: "Searched directory successfully: {:?}".to_string(),
                    result: search_results,
                }
            }
            Err(err) => SearchReply {
                code: 1,
                message: format!("Failed to search directory: {:?}", err),
                result: vec![],
            },
        };

        Ok(Response::new(reply))
    }
}

// #[tokio::main]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    simple_logger::init_with_env().unwrap();

    let addr = "[::1]:50051".parse()?;

    // This is required for surrealdb recursive queries
    // https://github.com/surrealdb/surrealdb/issues/2920
    let stack_size = 10 * 1024 * 1024;

    // Stack frames are generally larger in debug mode.
    #[cfg(debug_assertions)]
    let stack_size = stack_size * 2;

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(stack_size)
        .build()
        .unwrap()
        .block_on(async {
            if let Some(agent) = AudenAgent::new().await.ok() {
                let _ = Server::builder()
                    .add_service(AudenServer::new(agent))
                    .serve(addr)
                    .await;
            }

            loop {}
        });

    Ok(())
}
