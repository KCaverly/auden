use std::time::Duration;

use pg_embed::pg_enums::PgAuthMethod;
use pg_embed::pg_fetch::{PgFetchSettings, PG_V15};
use pg_embed::postgres::{PgEmbed, PgSettings};
use sqlx::pool::PoolConnection;
use sqlx::postgres::PgPoolOptions;
use sqlx::{Connection, Executor, Pool, Postgres, Row};
use std::path::PathBuf;
use tokio::sync::oneshot;

const DATABASE_NAME: &str = "syntax_surfer";

pub(crate) enum DatabaseJob {}

pub(crate) struct VectorDatabase {
    postgres_handle: PgEmbed,
    pool: Pool<Postgres>,
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
            "CREATE TABLE IF NOT EXISTS spans (
                id SERIAL PRIMARY KEY,
                file_id INT,
                start_byte INT NOT NULL,
                end_byte INT NOT NULL,
                embedding vector,
                CONSTRAINT fk_file
                    FOREIGN KEY(file_id)
                        REFERENCES file(id)
            )",
        )
        .await?;

        log::debug!("tables created appropriately in database");

        anyhow::Ok(VectorDatabase {
            postgres_handle: pg,
            pool,
        })
    }

    pub(crate) async fn get_conn(&self) -> anyhow::Result<PoolConnection<Postgres>> {
        anyhow::Ok(self.pool.acquire().await?)
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
}
