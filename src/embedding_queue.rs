use crate::embedding::{Embedding, EmbeddingProvider};
use crate::parsing::FileContext;
use std::mem;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::{broadcast, mpsc};

pub(crate) enum EmbeddingJob {
    Embed {
        file_context: Arc<Mutex<FileContext>>,
    },
    Flush,
}

#[derive(Debug, Clone)]
struct FileFragment {
    file_context: Arc<Mutex<FileContext>>,
    start_idx: usize,
    end_idx: usize,
}

#[derive(Clone)]
pub(crate) struct EmbeddingQueue {
    queue: Vec<FileFragment>,
    embed_tx: mpsc::Sender<Vec<FileFragment>>,
    finished_files_tx: broadcast::Sender<Arc<Mutex<FileContext>>>,
}

impl EmbeddingQueue {
    pub(crate) fn new(provider: Arc<dyn EmbeddingProvider>) -> Self {
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
                        for idx in fragment.start_idx..fragment.end_idx {
                            spans.push(unlocked.documents[idx].content.clone());
                        }
                    }

                    let embeddings = provider.embed(spans);

                    // Update File Context with Completed Embeddings
                    let mut i = 0;
                    for fragment in &queue {
                        let mut unlocked = fragment.file_context.lock().await;
                        for idx in fragment.start_idx..(fragment.end_idx + 1) {
                            unlocked.embeddings[idx] = embeddings[i].clone();
                        }
                        i += 1;

                        if unlocked.complete() {
                            let _ = finished_files_tx.send(fragment.file_context.clone());
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
        self.queue
            .iter()
            .map(|f| (f.end_idx - f.start_idx) + 1)
            .sum()
    }

    pub(crate) async fn queue_job(&mut self, job: EmbeddingJob) {
        let mut size = self.queue_size();
        match job {
            EmbeddingJob::Embed { file_context } => {
                log::debug!(
                    "queueing embedding job: {:?}",
                    file_context.lock().await.details.path
                );
                let mut current_idx = 0;
                let mut last_idx = 0;
                let ids = file_context.lock().await.document_ids();
                for idx in ids {
                    size += 1;

                    if size == 10 {
                        self.queue.push(FileFragment {
                            file_context: file_context.clone(),
                            start_idx: current_idx,
                            end_idx: idx,
                        });
                        current_idx = idx;
                        self.flush_queue().await;
                    }

                    last_idx = idx;
                }

                if current_idx != last_idx {
                    self.queue.push(FileFragment {
                        file_context: file_context.clone(),
                        start_idx: current_idx,
                        end_idx: last_idx,
                    });
                }
            }
            EmbeddingJob::Flush => {
                self.flush_queue().await;
            }
        }
    }
}
