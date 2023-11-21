use crate::embedding::Embedding;
use crate::parsing::FileContext;
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use surrealdb::engine::local::RocksDb;
use surrealdb::sql::Thing;
use surrealdb::Surreal;
use tokio::sync::{mpsc, Mutex};

pub(crate) enum DatabaseJob {
    WriteFileAndSpans { context: Arc<Mutex<FileContext>> },
}

impl fmt::Debug for DatabaseJob {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DatabaseJob::WriteFileAndSpans { .. } => {
                write!(f, "DatabaseJob::WriteFileAndSpans",)
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchResult {
    pub id: usize,
    pub path: PathBuf,
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Debug, Serialize)]
struct Span {
    start_byte: usize,
    end_byte: usize,
    sha: Vec<u8>,
    embedding: Vec<f32>,
}

#[derive(Debug, Serialize)]
struct Directory {
    path: PathBuf,
}

#[derive(Debug, Serialize)]
struct Path {
    id: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct DirectoryId {
    tb: Thing,
    #[allow(dead_code)]
    id: Thing,
}

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    #[allow(dead_code)]
    id: Thing,
}

#[derive(Clone)]
pub(crate) struct VectorDatabase {
    db: Surreal<surrealdb::engine::local::Db>,
    executor: mpsc::Sender<DatabaseJob>,
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

        // Create Table for Directory
        db.query(
            "
            DEFINE TABLE directory SCHEMAFULL;
            DEFINE FIELD path ON TABLE directory TYPE string;

        ",
        )
        .await
        .unwrap();

        // Create Table for File
        db.query(
            "
            DEFINE TABLE file SCHEMAFULL;
            DEFINE FIELD path ON TABLE file TYPE string;
        ",
        )
        .await
        .unwrap();

        // Create Table for Span
        db.query(
            "
            DEFINE TABLE span SCHEMAFULL;            
            DEFINE FIELD start_byte ON TABLE span TYPE int; 
            DEFINE FIELD end_byte ON TABLE span TYPE int;
            DEFINE FIELD embedding ON TABLE span TYPE array<float>;
            DEFINE FIELD sha ON TABLE span TYPE array<int>;
            ",
        )
        .await
        .unwrap();

        let (executor, mut receiver) = mpsc::channel::<DatabaseJob>(10000);
        tokio::spawn({
            async move {
                while let Some(job) = receiver.recv().await {
                    log::debug!("receiver db job: {:?}", job);
                    match job {
                        DatabaseJob::WriteFileAndSpans { context } => {
                            let file_context = context.lock().await;
                            let file_id = VectorDatabase::get_or_create_file(
                                &file_context.details.path,
                                file_context.details.directory_state.id.clone(),
                            )
                            .await;

                            if let Ok(file_id) = file_id {
                                let mut data = Vec::new();
                                for (embedding, document) in
                                    file_context.embeddings.iter().zip(&file_context.documents)
                                {
                                    data.push((
                                        document.start_byte,
                                        document.end_byte,
                                        document.sha.clone(),
                                        embedding,
                                    ));
                                }

                                let _ = VectorDatabase::get_or_create_spans(file_id.clone(), data)
                                    .await;
                                log::debug!("wrote file {:?}", file_id);
                            }
                        }
                    }
                }
            }
        });

        anyhow::Ok(VectorDatabase { db, executor })

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

    pub(crate) async fn get_or_create_directory(&self, path: &PathBuf) -> anyhow::Result<String> {
        let existing_paths = self
            .db
            .query("SELECT id FROM directory WHERE path = $path")
            .bind(("path", path))
            .await;

        match existing_paths {
            Ok(mut paths) => {
                let id: Vec<Thing> = paths.take("id").unwrap();
                if id.len() == 1 {
                    return anyhow::Ok(id[0].id.to_raw());
                } else {
                    let row: Vec<Record> = self
                        .db
                        .create("directory")
                        .content(Directory { path: path.clone() })
                        .await?;

                    return anyhow::Ok(
                        row.get(0)
                            .ok_or(anyhow!("not returning a created directory"))?
                            .id
                            .to_raw(),
                    );
                }
            }
            Err(err) => {
                return Err(anyhow!(err));
            }
        }
    }

    pub(crate) async fn queue(&self, database_job: DatabaseJob) -> anyhow::Result<()> {
        log::debug!("sending database job for execution: {:?}", database_job);
        anyhow::Ok(self.executor.send(database_job).await?)
    }

    pub(crate) async fn get_embeddings_for_directory(
        &self,
        path: &PathBuf,
    ) -> anyhow::Result<HashMap<Vec<u8>, Embedding>> {
        anyhow::Ok(HashMap::new())
    }

    pub(crate) async fn get_top_neighbours(
        &self,
        directory: PathBuf,
        embedding: &Embedding,
        n: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        todo!();
    }

    async fn get_or_create_file(path: &PathBuf, directory_id: String) -> anyhow::Result<String> {
        todo!();
    }

    async fn get_or_create_spans(
        path_id: String,
        data: Vec<(usize, usize, Vec<u8>, &Embedding)>,
    ) -> anyhow::Result<()> {
        todo!();
    }
}
