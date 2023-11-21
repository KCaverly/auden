use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use surrealdb::engine::local::RocksDb;
use surrealdb::sql::Thing;
use surrealdb::Surreal;

#[derive(Debug, Serialize)]
struct Span {
    start_byte: usize,
    end_byte: usize,
    sha: Vec<u8>,
    embedding: Vec<f32>,
}

#[derive(Debug, Serialize)]
struct FileSystemObject {
    id: PathBuf,
    spans: Vec<Span>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    #[allow(dead_code)]
    id: Thing,
}

pub(crate) struct VectorDatabase {
    db: Surreal<surrealdb::engine::local::Db>,
}

impl VectorDatabase {
    pub(crate) async fn initialize() -> anyhow::Result<Self> {
        // To start, this is just going to delete the file db
        let file = PathBuf::from(file!());
        let db_location = fs::canonicalize(file.parent().unwrap())
            .unwrap()
            .join("temp.db");
        tokio::fs::remove_dir_all(&db_location).await?;

        let db = Surreal::new::<RocksDb>(db_location).await?;
        db.use_ns("yars").use_db("yars").await?;

        anyhow::Ok(VectorDatabase { db })

        // // let row: Vec<Record> = db
        // //     .create("spans")
        // //     .content(Span {
        // //         start_byte: 0,
        // //         end_byte: 1,
        // //         sha: vec![],
        // //         embedding: vec![0.1, 0.8, 0.1],
        // //     })
        // //     .await
        // //     .unwrap();
        // //
        // // let row: Vec<Record> = db
        // //     .create("spans")
        // //     .content(Span {
        // //         start_byte: 3,
        // //         end_byte: 5,
        // //         sha: vec![],
        // //         embedding: vec![0.8, 0.05, 0.1],
        // //     })
        // //     .await
        // //     .unwrap();
        // //
        // // let rows = db
        // //     .query("SELECT id, vector::similarity::cosine(embedding, [0.9, 0.05, 0.1]) as similarity FROM spans ORDER BY similarity DESC LIMIT $n").bind(("n", 1))
        // //     .await;
        //
        // // dbg!(rows);
    }
}
