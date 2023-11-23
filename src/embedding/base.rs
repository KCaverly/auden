use async_trait::async_trait;

pub type Embedding = Vec<f32>;

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, spans: Vec<String>) -> anyhow::Result<Vec<Embedding>>;
}

pub struct FakeEmbeddingProvider;
#[async_trait]
impl EmbeddingProvider for FakeEmbeddingProvider {
    async fn embed(&self, spans: Vec<String>) -> anyhow::Result<Vec<Embedding>> {
        let mut embeddings = Vec::<Embedding>::new();
        for _ in spans {
            embeddings.push([0.32; 5].to_vec());
        }

        anyhow::Ok(embeddings)
    }
}
