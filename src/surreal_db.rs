use crate::embedding::Embedding;
use crate::parsing::FileContext;
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use surrealdb::engine::local::RocksDb;
use surrealdb::opt::RecordId;
use surrealdb::sql::Thing;
use surrealdb::Surreal;
use tokio::sync::oneshot;
use tokio::sync::{mpsc, Mutex};

pub(crate) enum DatabaseJob {
    GetOrCreateDirectory {
        path: PathBuf,
        sender: oneshot::Sender<anyhow::Result<String>>,
    },
    CreateFileAndSpans {
        context: Arc<Mutex<FileContext>>,
        sender: oneshot::Sender<anyhow::Result<()>>,
    },
    SearchDirectory {
        path: PathBuf,
        embedding: Embedding,
        n: usize,
        sender: oneshot::Sender<anyhow::Result<Vec<SearchResult>>>,
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
            DatabaseJob::SearchDirectory { .. } => {
                write!(f, "DatabaseJob::SearchDirectory",)
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SearchResult {
    pub id: RecordId,
    pub path: PathBuf,
    pub start_byte: usize,
    pub end_byte: usize,
    pub similarity: f32,
}

#[derive(Debug, Deserialize)]
pub struct EmbeddingResult {
    pub id: usize,
    pub embedding: Vec<f32>,
}

#[derive(PartialEq, Debug, Serialize, Deserialize)]
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

        let (executor, mut receiver) = mpsc::channel::<DatabaseJob>(1000);
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
                            DEFINE FIELD sha ON TABLE span TYPE array<int>;
                            DEFINE FIELD sha.* ON TABLE span TYPE int;
                            DEFINE FIELD embedding ON TABLE span TYPE array<float>;
                            DEFINE FIELD embedding.* ON TABLE span TYPE float;
                            ",
                        )
                        .await
                        .unwrap();

                        while let Some(job) = receiver.recv().await {
                            match job {
                                DatabaseJob::GetOrCreateDirectory { path, sender } => {
                                    let result = get_or_create_directory(&db, &path).await;
                                    let _ = sender.send(result);
                                }
                                DatabaseJob::CreateFileAndSpans { context, sender } => {
                                    let result = create_file_and_spans(&db, context.clone()).await;
                                    let _ = sender.send(result);
                                }
                                DatabaseJob::SearchDirectory {
                                    path,
                                    embedding,
                                    n,
                                    sender,
                                } => {
                                    let result = search_directory(&db, &path, &embedding, n).await;
                                    let _ = sender.send(result);
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

        receiver.await?
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
        let (sender, receiver) = oneshot::channel::<anyhow::Result<Vec<SearchResult>>>();
        let job = DatabaseJob::SearchDirectory {
            path: directory,
            embedding: embedding.clone(),
            n,
            sender,
        };

        self.queue(job).await?;
        receiver.await?
    }

    pub(crate) async fn create_file_and_spans(
        &self,
        context: Arc<Mutex<FileContext>>,
    ) -> anyhow::Result<()> {
        let (sender, receiver) = oneshot::channel::<anyhow::Result<()>>();
        let job = DatabaseJob::CreateFileAndSpans { context, sender };
        self.queue(job).await?;
        receiver.await?
    }
}

async fn get_or_create_directory(
    db: &Surreal<surrealdb::engine::local::Db>,
    path: &PathBuf,
) -> anyhow::Result<String> {
    if let Ok(mut paths) = db
        .query("SELECT id FROM directory WHERE path = $path")
        .bind(("path", path.clone()))
        .await
    {
        let id: Vec<Thing> = paths.take("id").unwrap();
        if id.len() == 1 {
            return anyhow::Ok(id[0].id.to_raw());
        } else {
            let row: Vec<Record> = db
                .create("directory")
                .content(Directory { path: path.clone() })
                .await?;

            if let Some(id) = row.get(0) {
                return anyhow::Ok(id.id.id.to_raw());
            }
        }
    }
    Err(anyhow!(anyhow!("failed to get or create directory")))
}

async fn create_file(
    db: &Surreal<surrealdb::engine::local::Db>,
    path: &PathBuf,
    directory_id: String,
) -> anyhow::Result<String> {
    let row: Vec<Record> = db
        .create("file")
        .content(File { path: path.clone() })
        .await?;

    let file_id = row.get(0).ok_or(anyhow!("row not created"))?.id.id.to_raw();
    let query = format!(
        "RELATE directory:{}->owns->file:{}",
        directory_id,
        file_id.clone()
    );
    let result = db.query(query).await?;
    result.check()?;

    anyhow::Ok(file_id)
}

async fn create_span(
    db: &Surreal<surrealdb::engine::local::Db>,
    span: Span,
    file_id: String,
) -> anyhow::Result<()> {
    let result: Vec<Record> = db.create("span").content(&span).await?;

    let id = result
        .get(0)
        .ok_or(anyhow!("span not created"))?
        .id
        .id
        .to_raw();

    debug_assert!({
        let result: Vec<Span> = db.select("span").range(&id..).await.unwrap();
        assert_eq!(
            result.get(0).unwrap(),
            &span,
            "span written and provided are different"
        );
        true
    });

    let query = format!("RELATE file:{}->contains->span:{}", file_id, id);
    let result = db.query(query).await?;
    result.check()?;

    anyhow::Ok(())
}

async fn delete_file_and_spans(
    db: &Surreal<surrealdb::engine::local::Db>,
    path: &PathBuf,
) -> anyhow::Result<()> {
    let query = format!(
        "DELETE span WHERE <-contains<-(file WHERE path = '{}')",
        path.to_string_lossy()
    );
    // Delete Spans
    let query = format!("DELETE file WHERE path = '{}'", path.to_string_lossy());
    let result = db.query(query).await?;
    result.check()?;

    // Delete Relations
    let query = format!(
        "DELETE contains WHERE in.path = '{}'",
        path.to_string_lossy()
    );
    let result = db.query(query).await?;
    result.check()?;

    // Delete File
    let query = format!("DELETE file WHERE path = '{}'", path.to_string_lossy());
    let result = db.query(query).await?;
    result.check()?;

    anyhow::Ok(())
}

async fn create_file_and_spans(
    db: &Surreal<surrealdb::engine::local::Db>,
    context: Arc<Mutex<FileContext>>,
) -> anyhow::Result<()> {
    let file_context = context.lock().await;
    let path = file_context.details.path.clone();
    let directory_id = file_context.details.directory_state.id.clone();

    // Automatically overwriting everything currently
    delete_file_and_spans(db, &path).await?;

    // Convert to Proper Data
    let mut data: Vec<Span> = Vec::new();
    for (embedding, document) in file_context.embeddings.iter().zip(&file_context.documents) {
        debug_assert!(
            embedding.len() > 0,
            "embedding length passed to creation is empty"
        );
        data.push(Span {
            start_byte: document.start_byte,
            end_byte: document.end_byte,
            sha: document.sha.clone(),
            embedding: embedding.clone(),
        });
    }

    let file_id = create_file(db, &path, directory_id).await?;
    for span in data {
        create_span(db, span, file_id.clone()).await?;
    }

    anyhow::Ok(())
}

async fn search_directory(
    db: &Surreal<surrealdb::engine::local::Db>,
    path: &PathBuf,
    embedding: &Embedding,
    n: usize,
) -> anyhow::Result<Vec<SearchResult>> {
    let query = format!(
        "
        SELECT id, array::first(<-contains<-file.path) as path, start_byte, end_byte, vector::similarity::cosine(embedding, $target) AS similarity
        FROM span 
        WHERE <-contains<-file<-owns<-(directory WHERE path = '{}')
        ORDER BY similarity DESC LIMIT $limit",
        path.to_string_lossy()
    );

    let mut response = db
        .query(query)
        .bind(("target", embedding))
        .bind(("limit", n))
        .await?;

    let results: Vec<SearchResult> = response.take(0)?;

    anyhow::Ok(results)
}

#[cfg(test)]
mod tests {
    use crate::parsing::ContextDocument;
    use crate::semantic_index::{DirectoryState, FileDetails};

    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_create_spans() {
        let tmp_dir = tempdir().unwrap();
        let tmp_path = PathBuf::from(tmp_dir.path());
        let db = VectorDatabase::initialize(tmp_path).await.unwrap();

        let directory_state = Arc::new(DirectoryState::new("id0".to_string()));
        directory_state.new_job();

        let test_file = Arc::new(Mutex::new(FileContext {
            details: FileDetails {
                path: PathBuf::from("/tmp/foo"),
                directory_state,
            },
            documents: vec![ContextDocument {
                start_byte: 0,
                end_byte: 10,
                sha: vec![1, 2, 3],
                content: "this is a test document".to_string(),
            }],
            embeddings: vec![vec![0.1, 0.2, 0.3]],
        }));

        let result = db.create_file_and_spans(test_file).await;
        result.unwrap();
    }

    async fn _test_create_spans_and_search() {
        let tmp_dir = tempdir().unwrap();
        let tmp_path = PathBuf::from(tmp_dir.path());
        let db = VectorDatabase::initialize(tmp_path).await.unwrap();

        let directory_path = PathBuf::from("/tmp");
        let directory_id = db.get_or_create_directory(&directory_path).await.unwrap();

        let directory_state = Arc::new(DirectoryState::new(directory_id));
        directory_state.new_job();

        let test_file = Arc::new(Mutex::new(FileContext {
            details: FileDetails {
                path: PathBuf::from("/tmp/foo"),
                directory_state: directory_state.clone(),
            },
            documents: vec![
                ContextDocument {
                    start_byte: 0,
                    end_byte: 10,
                    sha: vec![1, 2, 3],
                    content: "this is a test document".to_string(),
                },
                ContextDocument {
                    start_byte: 1,
                    end_byte: 12,
                    sha: vec![4, 5, 6],
                    content: "this is a second test document".to_string(),
                },
            ],
            embeddings: vec![vec![0.1, 0.2, 0.3], vec![0.9, 0.9, 0.1]],
        }));

        let result = db.create_file_and_spans(test_file).await;
        result.unwrap();

        let test_file2 = Arc::new(Mutex::new(FileContext {
            details: FileDetails {
                path: PathBuf::from("/tmp/foo2"),
                directory_state: directory_state.clone(),
            },
            documents: vec![ContextDocument {
                start_byte: 1,
                end_byte: 12,
                sha: vec![4, 5, 6],
                content: "this is a second test document".to_string(),
            }],
            embeddings: vec![vec![0.5, 0.2, 0.3]],
        }));

        directory_state.new_job();
        let result = db.create_file_and_spans(test_file2).await;
        result.unwrap();

        let search_results = db
            .get_top_neighbours(directory_path, &vec![0.8, 0.9, 0.2], 1)
            .await
            .unwrap();

        assert_eq!(search_results[0].path, PathBuf::from("/tmp/foo"));
        assert_eq!(search_results[0].start_byte, 1);
        assert_eq!(search_results[0].end_byte, 12);
    }

    #[test]
    fn test_create_spans_and_search() {
        // This hack is here because of the following issue with surrealdb
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
            .block_on(_test_create_spans_and_search())
    }
}
