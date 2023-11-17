use std::time::Duration;

use pg_embed::pg_enums::PgAuthMethod;
use pg_embed::pg_fetch::{PgFetchSettings, PG_V15};
use pg_embed::postgres::{PgEmbed, PgSettings};
use pgvector::Vector;
use sqlx::pool::PoolConnection;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Executor, Pool, Postgres, Row};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

use crate::embedding::Embedding;
use crate::parsing::FileContext;

const DATABASE_NAME: &str = "yars";

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

pub struct SearchResult {
    pub id: usize,
    pub path: PathBuf,
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Clone)]
pub(crate) struct VectorDatabase {
    postgres_handle: Arc<PgEmbed>,
    pool: Pool<Postgres>,
    executor: mpsc::Sender<DatabaseJob>,
}

impl VectorDatabase {
    pub(crate) async fn initialize(database_dir: PathBuf) -> anyhow::Result<VectorDatabase> {
        log::debug!("initializing database at {:?}", database_dir);
        let pg_settings = PgSettings {
            database_dir,
            port: 5432,
            user: "postgres".to_string(),
            password: "password".to_string(),
            auth_method: PgAuthMethod::Plain,
            persistent: true,
            timeout: Some(Duration::from_secs(15)),
            migration_dir: None,
        };

        let fetch_settings = PgFetchSettings {
            version: PG_V15,
            ..Default::default()
        };

        let mut pg = PgEmbed::new(pg_settings, fetch_settings).await?;
        pg.setup().await?;
        pg.start_db().await?;
        // Currently, I am just dropping the database on each run
        // this is not ideal
        if !pg.database_exists(DATABASE_NAME).await? {
            pg.create_database(DATABASE_NAME).await?;
        } else {
            pg.drop_database(DATABASE_NAME).await?;
            pg.create_database(DATABASE_NAME).await?;
        }
        let database_uri = pg.full_db_uri(DATABASE_NAME);

        log::debug!("database initialized appropriately");

        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_uri.as_str())
            .await?;

        // Initialize pg_embedding extension
        pool.execute("CREATE EXTENSION IF NOT EXISTS vector")
            .await?;

        // Create Tables
        log::debug!("creating tables in database");
        pool.execute(
            "
                CREATE TABLE IF NOT EXISTS directory (
                    id SERIAL PRIMARY KEY,
                    path VARCHAR(255) NOT NULL
                )",
        )
        .await?;

        pool.execute(
            "
            CREATE TABLE IF NOT EXISTS file (
                id SERIAL PRIMARY KEY,
                directory_id INT,
                path VARCHAR(255) NOT NULL,
                CONSTRAINT fk_directory
                    FOREIGN KEY(directory_id)
                        REFERENCES directory(id)
            )",
        )
        .await?;

        pool.execute(
            "CREATE TABLE IF NOT EXISTS span (
                id SERIAL PRIMARY KEY,
                file_id INT,
                start_byte INT NOT NULL,
                end_byte INT NOT NULL,
                sha BYTEA NOT NULL,
                embedding vector,
                CONSTRAINT fk_file
                    FOREIGN KEY(file_id)
                        REFERENCES file(id)
            )",
        )
        .await?;

        log::debug!("tables created appropriately in database");

        let (executor, mut receiver) = mpsc::channel::<DatabaseJob>(10000);
        tokio::spawn({
            let pool = pool.clone();
            async move {
                while let Some(job) = receiver.recv().await {
                    log::debug!("receiver db job: {:?}", job);
                    match job {
                        DatabaseJob::WriteFileAndSpans { context } => {
                            let file_context = context.lock().await;
                            let file_id = VectorDatabase::get_or_create_file(
                                &pool,
                                &file_context.details.path,
                                file_context.details.directory_state.id,
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
                                let _ =
                                    VectorDatabase::get_or_create_spans(&pool, file_id, data).await;
                                log::debug!("wrote file {:?}", file_id);
                            }
                        }
                    }
                }
            }
        });

        anyhow::Ok(VectorDatabase {
            postgres_handle: Arc::new(pg),
            pool,
            executor,
        })
    }

    pub(crate) async fn get_conn(&self) -> anyhow::Result<PoolConnection<Postgres>> {
        anyhow::Ok(self.pool.acquire().await?)
    }

    async fn get_or_create_file(
        pool: &Pool<Postgres>,
        path: &PathBuf,
        directory_id: usize,
    ) -> anyhow::Result<usize> {
        let mut conn = pool.acquire().await?;
        let r = conn
            .fetch_one(
                format!(
                    "INSERT INTO file (directory_id, path) VALUES ({}, '{}') RETURNING id",
                    directory_id,
                    path.as_path().to_string_lossy()
                )
                .as_str(),
            )
            .await?;
        return anyhow::Ok(r.get::<i32, _>(0) as usize);
    }

    async fn get_or_create_spans(
        pool: &Pool<Postgres>,
        path_id: usize,
        data: Vec<(usize, usize, Vec<u8>, &Embedding)>,
    ) -> anyhow::Result<()> {
        for row in data {
            sqlx::query("INSERT INTO span (file_id, start_byte, end_byte, sha, embedding) VALUES ($1, $2, $3, $4, $5)").bind(path_id as i32).bind(row.0 as i32).bind(row.1 as i32).bind(row.2).bind(row.3).execute(pool).await?;
        }

        anyhow::Ok(())
    }

    pub(crate) async fn get_or_create_directory(&self, path: &PathBuf) -> anyhow::Result<usize> {
        let pool = self.pool.clone();
        let path_str = path.as_path().to_string_lossy();

        // Identify if the directory already exists

        let account_id =
            sqlx::query(format!("SELECT id FROM directory WHERE path = '{}'", path_str).as_str())
                .fetch_one(&pool)
                .await;

        match account_id {
            Ok(account_id) => {
                return anyhow::Ok(account_id.get::<i32, _>(0) as usize);
            }
            Err(_) => {
                let r = self
                    .get_conn()
                    .await?
                    .fetch_one(
                        format!(
                            "INSERT INTO directory (path) VALUES ('{}') RETURNING id",
                            path_str
                        )
                        .as_str(),
                    )
                    .await?;
                return anyhow::Ok(r.get::<i32, _>(0) as usize);
            }
        }
    }

    pub(crate) async fn get_top_neighbours(
        &self,
        directory: PathBuf,
        embedding: &Embedding,
        n: usize,
    ) -> anyhow::Result<Vec<SearchResult>> {
        let pool = self.pool.clone();
        let vector = Vector::from(embedding.clone());
        let rows = sqlx::query(
            "SELECT span.id, file.path, span.start_byte, span.end_byte FROM span LEFT JOIN file ON span.file_id = file.id LEFT JOIN directory on file.directory_id = directory.id WHERE directory.path = $1 ORDER BY embedding <-> $2 LIMIT $3",
        )
        .bind(directory.as_path().to_string_lossy())
        .bind(vector)
        .bind(n as i32)
        .fetch_all(&pool)
        .await?;

        let mut results = Vec::new();
        for row in rows {
            let id = row.try_get::<i32, _>(0)? as usize;
            let path = PathBuf::from(row.try_get::<String, _>(1)?);
            let start_byte = row.try_get::<i32, _>(2)? as usize;
            let end_byte = row.try_get::<i32, _>(3)? as usize;

            results.push(SearchResult {
                id,
                path,
                start_byte,
                end_byte,
            })
        }

        anyhow::Ok(results)
    }

    pub(crate) async fn queue(&self, database_job: DatabaseJob) -> anyhow::Result<()> {
        log::debug!("sending database job for execution: {:?}", database_job);
        anyhow::Ok(self.executor.send(database_job).await?)
    }
}
