use crate::parsers::strategy::FileContext;
use anyhow::anyhow;
use llm_chain::traits::Embeddings;
use std::mem;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, Mutex};

pub(crate) enum EmbeddingJob {
    Embed {
        file_context: Arc<Mutex<FileContext>>,
    },
    Flush,
}

#[derive(Debug, Clone)]
struct FileFragment {
    file_context: Arc<Mutex<FileContext>>,
    embeddable_ids: Vec<usize>,
}

#[derive(Clone)]
pub(crate) struct EmbeddingQueue {
    queue: Vec<FileFragment>,
    embed_tx: mpsc::Sender<Vec<FileFragment>>,
    finished_files_tx: broadcast::Sender<Arc<Mutex<FileContext>>>,
}

impl EmbeddingQueue {
    pub(crate) fn new(provider: Arc<llm_chain_openai::embeddings::Embeddings>) -> Self {
        let (finished_files_tx, _) = broadcast::channel::<Arc<Mutex<FileContext>>>(10000);
        // Create a long lived task to embed and send off completed files
        let (embed_tx, mut receiver) = mpsc::channel::<Vec<FileFragment>>(10000);
        tokio::spawn({
            let finished_files_tx = finished_files_tx.clone();
            async move {
                // get spans and embed them
                while let Some(queue) = receiver.recv().await {
                    let mut spans = Vec::new();
                    for fragment in &queue {
                        let unlocked = fragment.file_context.lock().await;
                        for idx in &fragment.embeddable_ids {
                            spans.push(unlocked.documents[*idx].content.clone());
                        }
                    }

                    let embeddings = provider.embed_texts(spans).await;

                    match embeddings {
                        Ok(embeddings) => {
                            // Update File Context with Completed Embeddings
                            let mut i = 0;
                            for fragment in &queue {
                                let mut unlocked = fragment.file_context.lock().await;
                                for idx in &fragment.embeddable_ids {
                                    unlocked.embeddings[*idx] = embeddings[i].clone();
                                }
                                i += 1;

                                let complete = unlocked.complete();
                                drop(unlocked);
                                if complete {
                                    let _ = finished_files_tx.send(fragment.file_context.clone());
                                }
                            }
                        }
                        Err(err) => {
                            log::error!("{:?}", anyhow!(err));
                        }
                    }
                }
            }
        });

        EmbeddingQueue {
            queue: Vec::new(),
            embed_tx,
            finished_files_tx,
        }
    }

    pub(crate) async fn flush_queue(&mut self) {
        log::debug!("flushing queue");
        let queue = mem::take(&mut self.queue);
        let _ = self.embed_tx.send(queue).await;
    }

    pub(crate) async fn finished_files_rx(
        &mut self,
    ) -> tokio::sync::broadcast::Receiver<Arc<Mutex<FileContext>>> {
        self.finished_files_tx.subscribe()
    }

    fn queue_size(&self) -> usize {
        self.queue.iter().map(|f| f.embeddable_ids.len()).sum()
    }

    pub(crate) async fn queue_job(&mut self, job: EmbeddingJob) {
        let mut size = self.queue_size();
        match job {
            EmbeddingJob::Embed { file_context } => {
                log::debug!(
                    "queueing embedding job: {:?}",
                    file_context.lock().await.details.path
                );
                let outstanding_ids = file_context.lock().await.document_ids();
                let mut embeddable_ids = Vec::new();

                for idx in outstanding_ids {
                    size += 1;
                    embeddable_ids.push(idx);

                    if size == 10 {
                        let fragment_ids = mem::take(&mut embeddable_ids);
                        self.queue.push(FileFragment {
                            file_context: file_context.clone(),
                            embeddable_ids: fragment_ids,
                        });
                        self.flush_queue().await;
                        size = 0;
                    };
                }

                if embeddable_ids.len() != 0 {
                    self.queue.push(FileFragment {
                        file_context: file_context.clone(),
                        embeddable_ids,
                    });
                }
            }
            EmbeddingJob::Flush => {
                self.flush_queue().await;
            }
        }
    }
}
