use crate::db::VectorDatabase;
use crate::embedding::DummyEmbeddingProvider;
use crate::embedding_queue::{EmbeddingJob, EmbeddingQueue};
use crate::languages::{load_languages, LanguageConfig, LanguageRegistry};
use crate::parsing::FileContextParser;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::Duration;
use walkdir::{DirEntry, WalkDir};

pub(crate) struct SemanticIndex {
    vector_db: VectorDatabase,
    languages: LanguageRegistry,
    parse_sender: mpsc::Sender<Arc<(PathBuf, LanguageConfig)>>,
}

impl SemanticIndex {
    pub(crate) async fn new(database_dir: PathBuf) -> anyhow::Result<Self> {
        let (embedding_sender, mut embedding_receiver) = mpsc::channel::<EmbeddingJob>(10000);

        // Create a long-lived background task, which parses files
        let (parse_sender, mut parse_receiver) =
            mpsc::channel::<Arc<(PathBuf, LanguageConfig)>>(10000);
        tokio::spawn(async move {
            while let Some(file_to_parse) = parse_receiver.recv().await {
                if let Ok(context) =
                    FileContextParser::parse_file(&file_to_parse.0, &file_to_parse.1)
                {
                    let _ = embedding_sender
                        .send(EmbeddingJob::Embed {
                            file_context: Arc::new(Mutex::new(context)),
                        })
                        .await;
                }
            }
        });

        // Create a long-lived background task, which queues files for embedding
        let mut embedding_queue = EmbeddingQueue::new(Box::new(DummyEmbeddingProvider {}));
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
        let mut finished_files_rx = long_lived_embedding_queue.finished_files_rx().await;
        tokio::spawn(async move {
            while let Some(finished_file) = finished_files_rx.recv().await.ok() {
                log::debug!(
                    "received finished file: {:?}",
                    finished_file.lock().await.path
                );
            }
        });

        let vector_db = VectorDatabase::initialize(database_dir).await?;
        let languages = load_languages();
        anyhow::Ok(SemanticIndex {
            vector_db,
            languages,
            parse_sender,
        })
    }

    async fn walk_directory(&self, directory: PathBuf) -> anyhow::Result<()> {
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
                            self.parse_sender
                                .send(Arc::new((path.to_path_buf(), config.clone())))
                                .await?;
                        }
                    }
                }
            }
        }

        anyhow::Ok(())
    }

    pub(crate) async fn index_directory(&self, directory: PathBuf) -> anyhow::Result<()> {
        let _ = self.walk_directory(directory).await?;
        anyhow::Ok(())
    }
}
