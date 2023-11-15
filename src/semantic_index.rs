use crate::db::{DatabaseJob, VectorDatabase};
use crate::embedding::{DummyEmbeddingProvider, EmbeddingProvider};
use crate::embedding_queue::{EmbeddingJob, EmbeddingQueue};
use crate::languages::{load_languages, LanguageConfig, LanguageRegistry};
use crate::parsing::FileContextParser;
use anyhow::anyhow;
use std::borrow::BorrowMut;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex, Notify};
use tokio::time::Duration;
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Clone)]
pub(crate) struct FileDetails {
    pub(crate) path: PathBuf,
    pub(crate) directory_state: Arc<DirectoryState>,
}

#[derive(Debug)]
pub(crate) struct DirectoryState {
    pub(crate) id: usize,
    pub(crate) job_count_tx: watch::Sender<usize>,
    pub(crate) job_count_rx: watch::Receiver<usize>,
    pub(crate) notify: Arc<Notify>,
}

impl DirectoryState {
    pub fn new(id: usize) -> Self {
        let (job_count_tx, job_count_rx) = watch::channel::<usize>(0);
        let notify = Arc::new(Notify::new());
        DirectoryState {
            id,
            job_count_tx,
            job_count_rx,
            notify,
        }
    }

    pub fn new_job(&self) {
        let current_count = self.job_count_rx.borrow().clone();
        self.job_count_tx.send_replace(current_count + 1);
    }

    pub fn job_dropped(&self) {
        let current_count = self.job_count_rx.borrow().clone();
        let new_count = current_count - 1;
        self.job_count_tx.send_replace(new_count);

        if new_count == 0 {
            self.notify.notify_one();
        }
    }

    pub fn status(&self) -> IndexingStatus {
        let jobs_outstanding = self.job_count_rx.borrow().clone();
        if jobs_outstanding == 0 {
            IndexingStatus::Indexed
        } else {
            IndexingStatus::Indexing { jobs_outstanding }
        }
    }
}

#[derive(Debug)]
pub(crate) enum IndexingStatus {
    Indexing { jobs_outstanding: usize },
    Indexed,
    NotIndexed,
}

pub(crate) struct SemanticIndex {
    vector_db: VectorDatabase,
    languages: LanguageRegistry,
    parse_sender: mpsc::Sender<Arc<(FileDetails, LanguageConfig)>>,
    embedding_provider: Arc<dyn EmbeddingProvider>,
    directory_state: HashMap<PathBuf, Arc<DirectoryState>>,
}

impl SemanticIndex {
    pub(crate) async fn new(
        database_dir: PathBuf,
        embedding_provider: Arc<dyn EmbeddingProvider>,
    ) -> anyhow::Result<Self> {
        let (embedding_sender, mut embedding_receiver) = mpsc::channel::<EmbeddingJob>(10000);

        // Create a long-lived background task, which parses files
        let (parse_sender, mut parse_receiver) =
            mpsc::channel::<Arc<(FileDetails, LanguageConfig)>>(10000);
        tokio::spawn(async move {
            while let Some(file_to_parse) = parse_receiver.recv().await {
                if let Ok(context) =
                    FileContextParser::parse_file(&file_to_parse.0, &file_to_parse.1)
                {
                    context.details.directory_state.new_job();

                    let _ = embedding_sender
                        .send(EmbeddingJob::Embed {
                            file_context: Arc::new(Mutex::new(context)),
                        })
                        .await;
                }
            }
        });

        // Create a long-lived background task, which queues files for embedding
        let mut embedding_queue = EmbeddingQueue::new(embedding_provider.clone());
        let mut long_lived_embedding_queue = embedding_queue.clone(); // I dont really like this
        tokio::spawn(async move {
            let mut new_values = false;
            loop {
                match tokio::time::timeout(Duration::from_millis(250), embedding_receiver.recv())
                    .await
                {
                    Ok(embedding_job) => {
                        new_values = true;
                        if let Some(embedding_job) = embedding_job {
                            embedding_queue.queue_job(embedding_job).await;
                        }
                    }
                    Err(_) => {
                        if new_values {
                            embedding_queue.queue_job(EmbeddingJob::Flush).await;
                            new_values = false;
                        }
                    }
                }
            }
        });

        // Create a long-lived background task, which gets finished files and writes them to the
        // database
        let vector_db = VectorDatabase::initialize(database_dir).await?;
        let mut finished_files_rx = long_lived_embedding_queue.finished_files_rx().await;
        tokio::spawn({
            let vector_db = vector_db.clone();
            async move {
                while let Some(finished_file) = finished_files_rx.recv().await.ok() {
                    let _ = vector_db
                        .queue(DatabaseJob::WriteFileAndSpans {
                            context: finished_file,
                        })
                        .await;
                }
            }
        });

        let languages = load_languages();
        anyhow::Ok(SemanticIndex {
            vector_db,
            languages,
            parse_sender,
            embedding_provider,
            directory_state: HashMap::new(),
        })
    }

    async fn walk_directory(
        &self,
        directory_state: Arc<DirectoryState>,
        directory: PathBuf,
    ) -> anyhow::Result<()> {
        fn is_hidden(entry: &DirEntry) -> bool {
            entry
                .file_name()
                .to_str()
                .map(|s| s.starts_with("."))
                .unwrap_or(false)
        }

        fn is_target_dir(entry: &DirEntry) -> bool {
            entry
                .file_name()
                .to_str()
                .map(|s| s.starts_with("target"))
                .unwrap_or(false)
        }

        let walker = WalkDir::new(directory).into_iter();
        for entry in walker.filter_entry(|e| !is_hidden(e) && !is_target_dir(e)) {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_file() && !path.is_symlink() {
                    if let Some(extension) =
                        path.extension().and_then(|extension| extension.to_str())
                    {
                        if let Some(config) = self.languages.get_config_from_extension(extension) {
                            let file_details = FileDetails {
                                path: path.to_path_buf(),
                                directory_state: directory_state.clone(),
                            };
                            self.parse_sender
                                .send(Arc::new((file_details, config.clone())))
                                .await?;
                        }
                    }
                }
            }
        }

        anyhow::Ok(())
    }

    pub(crate) async fn index_directory(
        &mut self,
        directory: PathBuf,
    ) -> anyhow::Result<Arc<Notify>> {
        // Get or Create Directory Item in Vector Database
        let directory_id = self.vector_db.get_or_create_directory(&directory).await?;
        let directory_state = Arc::new(DirectoryState::new(directory_id));

        // TODO: Make this work for concurrent index calls
        self.directory_state
            .insert(directory.clone(), directory_state.clone());

        let _ = self
            .walk_directory(directory_state.clone(), directory)
            .await?;

        anyhow::Ok(directory_state.notify.clone())
    }

    pub(crate) async fn search_directory(
        &self,
        directory: PathBuf,
        n: usize,
        search_query: &str,
    ) -> anyhow::Result<Vec<i32>> {
        // Handle for calls to search before indexing is complete, by automatically kicking
        // indexing off.
        // let await = self.index_directory(directory.clone()).await;
        // indexing.await;

        if let Some(embedding) = self
            .embedding_provider
            .embed(vec![search_query.to_string()])
            .get(0)
        {
            self.vector_db
                .get_top_neighbours(directory, embedding, n)
                .await
        } else {
            Err(anyhow!("embedding provider failed to embed search query"))
        }
    }

    pub(crate) async fn get_status(&self, directory: PathBuf) -> IndexingStatus {
        if let Some(directory_state) = self.directory_state.get(&directory) {
            directory_state.status()
        } else {
            IndexingStatus::NotIndexed
        }
    }
}
