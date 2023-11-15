pub type Embedding = Vec<f32>;

pub trait EmbeddingProvider: Send {
    fn embed(&self, spans: Vec<String>) -> Vec<Embedding>;
}

pub struct DummyEmbeddingProvider;
impl EmbeddingProvider for DummyEmbeddingProvider {
    fn embed(&self, spans: Vec<String>) -> Vec<Embedding> {
        let mut embeddings = Vec::<Embedding>::new();
        for span in spans {
            embeddings.push([0.32; 526].to_vec());
        }

        embeddings
    }
}
