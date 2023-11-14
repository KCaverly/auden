mod db;

use crate::db::VectorDatabase;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    simple_logger::init_with_env().unwrap();

    // For now, lets just panic if the Vector Database is not initialized properly
    let db = VectorDatabase::initialize(PathBuf::from("data/db")).await;
    match db {
        Ok(db) => {
            let result = db
                .get_or_create_directory(PathBuf::from("/home/kcaverly/personal/blang"))
                .await
                .unwrap();
        }
        Err(err) => {
            panic!("{:?}", err);
        }
    }
}
