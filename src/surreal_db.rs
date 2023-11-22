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
use tokio::sync::oneshot;
use tokio::sync::{mpsc, Mutex};

pub(crate) enum DatabaseJob {
    GetOrCreateDirectory {
        path: PathBuf,
        sender: oneshot::Sender<String>,
    },
    CreateFileAndSpans {
        context: Arc<Mutex<FileContext>>,
    },
}

impl fmt::Debug for DatabaseJob {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DatabaseJob::GetOrCreateDirectory { .. } => {
                write!(f, "DatabaseJob::CreateDirectory",)
            }
            DatabaseJob::CreateFileAndSpans { .. } => {
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
struct File {
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
    executor: mpsc::Sender<DatabaseJob>,
}

impl VectorDatabase {
    pub(crate) async fn initialize(database_dir: PathBuf) -> anyhow::Result<Self> {
        const DATABASE_NAME: &str = "yars";

        let (executor, mut receiver) = mpsc::channel::<DatabaseJob>(10000);
        tokio::spawn({
            async move {
                let location = database_dir.join("temp.db");
                log::debug!("initializing surrealdb at {:?}", location.clone());

                match Surreal::new::<RocksDb>(location).await {
                    Ok(db) => {
                        db.use_ns(DATABASE_NAME).use_db(DATABASE_NAME).await;

                        // Create Tables
                        db.query(
                            "
                            DEFINE TABLE directory SCHEMAFULL;
                            DEFINE FIELD path ON TABLE directory TYPE string;
                            ",
                        )
                        .await
                        .unwrap();

                        db.query(
                            "
                            DEFINE TABLE file SCHEMAFULL;
                            DEFINE FIELD path ON TABLE file TYPE string;
                        ",
                        )
                        .await
                        .unwrap();

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

                        while let Some(job) = receiver.recv().await {
                            match job {
                                DatabaseJob::GetOrCreateDirectory { path, sender } => {
                                    if let Ok(mut paths) = db
                                        .query("SELECT id FROM directory WHERE path = $path")
                                        .bind(("path", path.clone()))
                                        .await
                                    {
                                        let id: Vec<Thing> = paths.take("id").unwrap();
                                        if id.len() == 1 {
                                            sender.send(id[0].id.to_raw());
                                        } else {
                                            let row: Result<Vec<Record>, surrealdb::Error> = db
                                                .create("directory")
                                                .content(Directory { path: path.clone() })
                                                .await;

                                            match row {
                                                Ok(row) => {
                                                    row.get(0).and_then(|r| {
                                                        sender.send(r.id.to_raw()).ok()
                                                    });
                                                }
                                                Err(err) => {
                                                    log::error!("{:?}", err);
                                                }
                                            }
                                        }
                                    }
                                }
                                DatabaseJob::CreateFileAndSpans { context } => {
                                    let file_context = context.lock().await;
                                    let path = file_context.details.path.clone();

                                    let mut data = Vec::new();
                                    for (embedding, document) in
                                        file_context.embeddings.iter().zip(&file_context.documents)
                                    {
                                        data.push(Span {
                                            start_byte: document.start_byte,
                                            end_byte: document.end_byte,
                                            sha: document.sha.clone(),
                                            embedding: embedding.clone(),
                                        });
                                    }

                                    // Create File
                                    let row: Result<Vec<Record>, surrealdb::Error> =
                                        db.create("file").content(File { path }).await;

                                    match row {
                                        Ok(row) => {
                                            if let Some(id) = row.get(0) {
                                                for span in data {
                                                    let span_row: Result<
                                                        Vec<Record>,
                                                        surrealdb::Error,
                                                    > = db.create("span").content(span).await;
                                                    match span_row {
                                                        Ok(span_id) => {
                                                            if let Some(span_id) = span_id.get(0) {
                                                                let query = format!(
                                                                    "RELATE {:?}->contains->{:?};",
                                                                    id.id.to_raw(),
                                                                    span_id.id.to_raw()
                                                                );
                                                                let _ = db.query(query).await;
                                                            }
                                                        }
                                                        Err(err) => log::error!("{:?}", err),
                                                    }
                                                }
                                            }
                                        }
                                        Err(err) => {
                                            log::error!("{:?}", err);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Err(err) => {
                        panic!("{:?}", err);
                    }
                }
            }
        });

        anyhow::Ok(VectorDatabase { executor })
    }

    pub(crate) async fn get_or_create_directory(&self, path: &PathBuf) -> anyhow::Result<String> {
        let (sender, receiver) = oneshot::channel();
        let job = DatabaseJob::GetOrCreateDirectory {
            path: path.clone(),
            sender,
        };

        // Send Job Over
        self.queue(job).await?;

        anyhow::Ok(receiver.await?)

        // let existing_paths = self
        //     .db
        //     .query("SELECT id FROM directory WHERE path = $path")
        //     .bind(("path", path))
        //     .await;
        //
        // match existing_paths {
        //     Ok(mut paths) => {
        //         let id: Vec<Thing> = paths.take("id").unwrap();
        //         if id.len() == 1 {
        //             return anyhow::Ok(id[0].id.to_raw());
        //         } else {
        //             let row: Vec<Record> = self
        //                 .db
        //                 .create("directory")
        //                 .content(Directory { path: path.clone() })
        //                 .await?;
        //
        //             return anyhow::Ok(
        //                 row.get(0)
        //                     .ok_or(anyhow!("not returning a created directory"))?
        //                     .id
        //                     .to_raw(),
        //             );
        //         }
        //     }
        //     Err(err) => {
        //         return Err(anyhow!(err));
        //     }
        // }
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

    pub(crate) async fn create_file_and_spans(
        &self,
        context: Arc<Mutex<FileContext>>,
    ) -> anyhow::Result<()> {
        self.executor
            .send(DatabaseJob::CreateFileAndSpans { context })
            .await?;
        anyhow::Ok(())
    }
}
