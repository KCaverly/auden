mod db;
mod embedding;
mod embedding_queue;
mod languages;
mod parsing;
mod semantic_index;

use crate::semantic_index::SemanticIndex;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

use self::embedding::DummyEmbeddingProvider;

#[tokio::main]
async fn main() {
    simple_logger::init_with_env().unwrap();

    if let Some(index) = SemanticIndex::new(
        PathBuf::from("data/db"),
        Arc::new(DummyEmbeddingProvider {}),
    )
    .await
    .ok()
    {
        let _ = index
            .index_directory(PathBuf::from("/home/kcaverly/personal/blang"))
            .await;

        sleep(Duration::from_secs(10)).await;
        println!("SEARCHING...");

        let results = index
            .search_directory(
                PathBuf::from("/home/kcaverly/personal/blang"),
                10,
                "This is a test query",
            )
            .await;

        println!("RESULTS: {:?}", results);

        loop {}
    }
}
