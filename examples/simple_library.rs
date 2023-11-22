use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use yars::semantic_index::SemanticIndex;

use yars::embedding::DummyEmbeddingProvider;

async fn run_example() {
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

fn main() {
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
        .block_on(run_example());
}
