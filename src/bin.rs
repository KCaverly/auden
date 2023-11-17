use std::path::PathBuf;
use std::sync::Arc;
use yars::semantic_index::SemanticIndex;

use yars::embedding::DummyEmbeddingProvider;

#[tokio::main]
async fn main() {
    simple_logger::init_with_env().unwrap();

    if let Some(mut index) = SemanticIndex::new(
        PathBuf::from("data/db"),
        Arc::new(DummyEmbeddingProvider {}),
    )
    .await
    .ok()
    {
        if let Some(indexing) = index
            .index_directory(PathBuf::from("/home/kcaverly/personal/blang"))
            .await
            .ok()
        {
            indexing.notified().await;

            let status = index
                .get_status(PathBuf::from("/home/kcaverly/personal/blang"))
                .await;

            let results = index
                .search_directory(
                    PathBuf::from("/home/kcaverly/personal/blang"),
                    10,
                    "This is a test query",
                )
                .await;

            println!("RESULTS: {:?}", results);
        };

        loop {}
    }
}
