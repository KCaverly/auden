use crate::embedding::{Embedding, EmbeddingProvider};
use crate::parsing::FileContext;
use std::mem;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::{mpsc, oneshot};

pub(crate) enum EmbeddingJob {
    Embed {
        file_context: Arc<Mutex<FileContext>>,
    },
    Flush,
}

#[derive(Debug)]
struct FileFragment {
    file_context: Arc<Mutex<FileContext>>,
    start_idx: usize,
    end_idx: usize,
}

pub(crate) struct EmbeddingQueue {
    queue: Vec<FileFragment>,
    embed_tx: mpsc::Sender<(Vec<String>, oneshot::Sender<Vec<Embedding>>)>,
    finished_files_tx: mpsc::Sender<Arc<Mutex<FileContext>>>,
    pub(crate) finished_files_rx: mpsc::Receiver<Arc<Mutex<FileContext>>>,
}

impl EmbeddingQueue {
    pub(crate) fn new(provider: Box<dyn EmbeddingProvider>) -> Self {
        // Create a Task Which is just embedding the spans it receives
        let (embed_tx, mut receiver) =
            mpsc::channel::<(Vec<String>, oneshot::Sender<Vec<Embedding>>)>(10000);
        tokio::spawn(async move {
            while let Some((spans, embedding_sender)) = receiver.recv().await {
                let embeddings = provider.embed(spans);
                println!("SENDING BACK!");
                let _ = embedding_sender.send(embeddings);
            }
        });

        let (finished_files_tx, finished_files_rx) =
            mpsc::channel::<Arc<Mutex<FileContext>>>(10000);

        EmbeddingQueue {
            queue: Vec::new(),
            embed_tx,
            finished_files_rx,
            finished_files_tx,
        }
    }

    pub(crate) async fn flush_queue(&mut self) {
        log::debug!("flushing queue");
        // For each file context, we would be expected to send start idx -> end idx to the
        // embedding engine

        let mut spans = Vec::new();
        let queue = mem::take(&mut self.queue);
        for fragment in &queue {
            let unlocked = fragment.file_context.lock().await;
            for idx in fragment.start_idx..fragment.end_idx {
                spans.push(unlocked.documents[idx].content.clone());
            }
        }

        let (sender, receiver) = oneshot::channel();
        let _ = self.embed_tx.send((spans, sender)).await;

        // Keeping this in the exact same, code path and awaiting it like this,
        // Likely leaves this code, running syncronously, as we dont move on to
        // other work, until new embeddings are sent back
        // I think we should move this into the embeddin channel itself
        // maybe the channel, grabs the spawn from the queue, updates the file_context objects
        // and checks if its complete if its complete, it then sends it to the finished_files
        // channel
        if let Some(embeddings) = receiver.await.ok() {
            let mut i = 0;
            for fragment in &queue {
                let mut unlocked = fragment.file_context.lock().await;
                for idx in fragment.start_idx..(fragment.end_idx + 1) {
                    unlocked.embeddings[idx] = embeddings[i].clone();
                }
                i += 1;

                if unlocked.complete() {
                    let _ = self
                        .finished_files_tx
                        .send(fragment.file_context.clone())
                        .await;
                }
            }
        }
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
                    file_context.lock().await.path
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
                println!("FLUSHING QUEUE!!!!");
                self.flush_queue().await;
            }
        }
    }
}
