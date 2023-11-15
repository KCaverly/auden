mod db;
mod embedding;
mod embedding_queue;
mod languages;
mod parsing;
mod semantic_index;

use crate::semantic_index::SemanticIndex;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    simple_logger::init_with_env().unwrap();

    if let Some(index) = SemanticIndex::new(PathBuf::from("data/db")).await.ok() {
        let _ = index
            .index_directory(PathBuf::from("/home/kcaverly/personal/blang"))
            .await;
    }
}
