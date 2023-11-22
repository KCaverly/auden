use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use yars::semantic_index::SemanticIndex;

use yars::embedding::DummyEmbeddingProvider;
#[tokio::main]
async fn main() {
    simple_logger::init_with_env().unwrap();

    let directory = "/home/kcaverly/personal/yars";

    if let Some(mut index) = SemanticIndex::new(
        fs::canonicalize(PathBuf::from("data/db")).unwrap(),
        Arc::new(DummyEmbeddingProvider {}),
    )
    .await
    .ok()
    {
        if let Some(indexing) = index.index_directory(PathBuf::from(directory)).await.ok() {
            indexing.notified().await;

            let results = index
                .search_directory(PathBuf::from(directory), 10, "This is a test query")
                .await;

            println!("RESULTS: {:?}", results);
        };
    }
}
