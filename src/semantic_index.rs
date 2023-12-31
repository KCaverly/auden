use crate::db::{SearchResult, VectorDatabase};
use crate::embedding_queue::{EmbeddingJob, EmbeddingQueue};
use crate::parsers::registry::{load_extensions, ExtensionRegistry};
use crate::parsers::strategy::{parse_file, ParsingStrategy};
use anyhow::anyhow;
use llm_chain::traits::Embeddings;
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
    pub(crate) id: String,
    pub(crate) job_count_tx: watch::Sender<usize>,
    pub(crate) job_count_rx: watch::Receiver<usize>,
    pub(crate) notify: Arc<Notify>,
}

impl DirectoryState {
    pub fn new(id: String) -> Self {
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
pub enum IndexingStatus {
    Indexing { jobs_outstanding: usize },
    Indexed,
    NotIndexed,
}

impl ToString for IndexingStatus {
    fn to_string(&self) -> String {
        match self {
            IndexingStatus::Indexing { .. } => "Indexing",
            IndexingStatus::Indexed => "Indexed",
            IndexingStatus::NotIndexed => "Not Indexed",
        }
        .to_string()
    }
}

impl IndexingStatus {
    pub fn outstanding(&self) -> Option<usize> {
        match self {
            IndexingStatus::Indexing { jobs_outstanding } => Some(*jobs_outstanding),
            _ => None,
        }
    }
}

pub struct SemanticIndex {
    vector_db: VectorDatabase,
    parsers: ExtensionRegistry,
    parse_sender: mpsc::Sender<
        Arc<(
            FileDetails,
            ParsingStrategy,
            Arc<HashMap<Vec<u8>, Vec<f32>>>,
        )>,
    >,
    directory_state: HashMap<PathBuf, Arc<DirectoryState>>,
    embedding_provider: Arc<llm_chain_openai::embeddings::Embeddings>,
}

impl SemanticIndex {
    pub async fn new(database_dir: PathBuf) -> anyhow::Result<Self> {
        let embedding_provider = Arc::new(llm_chain_openai::embeddings::Embeddings::default());

        let (embedding_sender, mut embedding_receiver) = mpsc::channel::<EmbeddingJob>(10000);

        // Create a long-lived background task, which parses files
        let (parse_sender, mut parse_receiver) = mpsc::channel::<
            Arc<(
                FileDetails,
                ParsingStrategy,
                Arc<HashMap<Vec<u8>, Vec<f32>>>,
            )>,
        >(10000);
        tokio::spawn(async move {
            while let Some(file_to_parse) = parse_receiver.recv().await {
                if let Ok(mut context) = parse_file(file_to_parse.0.clone(), &file_to_parse.1) {
                    context.details.directory_state.new_job();

                    // Update embeddings if the shas are already available
                    for (idx, document) in context.documents.iter().enumerate() {
                        if let Some(embedding) = file_to_parse.2.get(&document.sha) {
                            context.embeddings[idx] = embedding.clone();
                        }
                    }

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
                    let result = vector_db.create_file_and_spans(finished_file).await;
                    match result {
                        Ok(_) => {}
                        Err(err) => {
                            log::error!("{:?}", err)
                        }
                    }
                }
            }
        });

        let parsers = load_extensions();
        anyhow::Ok(SemanticIndex {
            vector_db,
            parsers,
            parse_sender,
            directory_state: HashMap::new(),
            embedding_provider,
        })
    }

    async fn walk_directory(
        &self,
        directory_state: Arc<DirectoryState>,
        directory: PathBuf,
        existing_embeddings: Arc<HashMap<Vec<u8>, Vec<f32>>>,
    ) -> anyhow::Result<()> {
        let mut existing_paths = self.vector_db.get_files_for_directory(&directory).await?;

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

        let walker = WalkDir::new(directory.clone()).into_iter();
        for entry in walker.filter_entry(|e| !is_hidden(e) && !is_target_dir(e)) {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_file() && !path.is_symlink() {
                    if let Some(extension) =
                        path.extension().and_then(|extension| extension.to_str())
                    {
                        if let Some(strategy) = self
                            .parsers
                            .get_strategy_for_extension(extension.to_string())
                            .ok()
                        {
                            existing_paths.remove(&path.to_path_buf());

                            let file_details = FileDetails {
                                path: path.to_path_buf(),
                                directory_state: directory_state.clone(),
                            };
                            self.parse_sender
                                .send(Arc::new((
                                    file_details,
                                    strategy.clone(),
                                    existing_embeddings.clone(),
                                )))
                                .await?;
                        }
                    }

                    // if let Some(extension) =
                    //     path.extension().and_then(|extension| extension.to_str())
                    // {
                    //     if let Some(config) = self.languages.get_config_from_extension(extension) {
                    //         existing_paths.remove(&path.to_path_buf());
                    //
                    //         let file_details = FileDetails {
                    //             path: path.to_path_buf(),
                    //             directory_state: directory_state.clone(),
                    //         };
                    //         self.parse_sender
                    //             .send(Arc::new((
                    //                 file_details,
                    //                 config.clone(),
                    //                 existing_embeddings.clone(),
                    //             )))
                    //             .await?;
                    //     }
                    // }
                }
            }
        }

        for path in existing_paths {
            self.vector_db.delete_file(&path).await?;
        }

        anyhow::Ok(())
    }

    pub async fn index_directory(&mut self, directory: PathBuf) -> anyhow::Result<Arc<Notify>> {
        // Get or Create Directory Item in Vector Database
        let directory_id = self.vector_db.get_or_create_directory(&directory).await?;
        let directory_state = Arc::new(DirectoryState::new(directory_id));

        let existing_embeddings = Arc::new(
            self.vector_db
                .get_embeddings_for_directory(&directory)
                .await?,
        );

        // TODO: Make this work for concurrent index calls
        self.directory_state
            .insert(directory.clone(), directory_state.clone());

        let _ = self
            .walk_directory(directory_state.clone(), directory, existing_embeddings)
            .await?;

        anyhow::Ok(directory_state.notify.clone())
    }

    pub async fn search_directory(
        &self,
        directory: PathBuf,
        n: usize,
        search_query: &str,
    ) -> anyhow::Result<Vec<SearchResult>> {
        // Handle for calls to search before indexing is complete, by automatically kicking
        // indexing off.
        // let await = self.index_directory(directory.clone()).await;
        // indexing.await;
        log::debug!("searching {:?} for {:?}", &directory, &search_query);

        if let Some(embedding) = self
            .embedding_provider
            .embed_query(search_query.to_string())
            .await
            .ok()
        {
            self.vector_db
                .get_top_neighbours(directory, &embedding, n)
                .await
        } else {
            Err(anyhow!("embedding provider failed to embed search query"))
        }
    }

    pub async fn get_status(&self, directory: PathBuf) -> IndexingStatus {
        if let Some(directory_state) = self.directory_state.get(&directory) {
            directory_state.status()
        } else {
            IndexingStatus::NotIndexed
        }
    }
}
