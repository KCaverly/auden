<br>
<br>
<p align="center">
  <img src="logo.png" width="600"/>
</p>

<p align="center"><i>yet another retrieval server</i></p>

## Embedded Engine for On-Device Retrieval Applications

This is very much a work in progress project, in which I am hoping to create a standalone directory based retrieval engine in Rust. The hope is that this can either be leveraged as a package directly inside another Rust application to manage on-device retrieval, or run as a binary and exposed over something like gRPC.

At a high level *auden* uses:
- [tokio](https://tokio.rs): Fast async runtime.
- [surrealdb](https://surrealdb.com): Embedded Database for Vector Retrieval.
- [tonic](https://github.com/hyperium/tonic): gRPC serving.
- [treesitter](https://tree-sitter.github.io/tree-sitter/): Semantic Parsing of Source Content.
- [llm chain](https://github.com/sobelio/llm-chain): For interacting with embedding models and LLM agents.

### Example - As a Library

The below is a short example, of how simple one can index and search a directory. 

```rust
use auden::semantic_index::SemanticIndex;

#[tokio::main]
fn main() {

    let data_directory = PathBuf::from("/home/<user>/.auden/db");

    // Initializes the database and starts workers
    if let Some(mut index) = SemanticIndex::new(data_directory).await.ok() {

        // Index directory, will crawl the directory provided, parse eligible content,
        // and embed this content with the OpenAI API, and store these results in an
        // embedded SurrealDB for easy vector retrieval.
        let directory_to_index = PathBuf::from("/home/kcaverly/auden");
        if let Some(indexing) = index.index_directory(directory_to_index.clone()).await.ok() {

            // Wait for indexing to complete
            indexing.notified().await;

            // Query Directory, returns the top 10 nearest neighbours using cosine
            let query = "where do I manage getting a Parsing Strategy for an extension?";
            let results = index.search_directory(directory_to_index, 10, query)).await.unwrap();

            // Will return a vector of SearchResults which look like the following
            // You can then leverage this to retrieve the underling data as you wish
            // struct SearchResult {
            //     id: RecordId, 
            //     start_byte: usize,
            //     end_byte: usize,
            //     path: PathBuf,
            //     similarity: f32
            // }
        }

    }

}

```

### What content does it parse?

Ultimately, many retrieval pipelines have very specific purposes in mind, and the information parsed and stored will be different.
I am currently building this for its use in terminal or editor tools, and as such, am focusing on parsing code and common repository file formats.
Currently, I include object level embeddings for Rust, along with whole file embeddings for Markdown & TOML.

The goal is to provide a general, and high quality enough retrieval engine, to make localized tooling for RAG applications possible without a whole bunch of redundant prework.
You may kinda think of this project, as an lsp for context retrieval.

### Roadmap

This project is primarily an experiment right now, and very simple. It may remain as a retrieval only utility, or I may incorporate Agent actions directly into this server as well. 

The short term focus, will be on hardening the engine (specifically surrounding managing for API failures), and improving retrieval quality (exploring ReRanking or leveraging GPTs as ReRankers). The medium-term goal is to build a Terminal utility for exploring my local filesystems, and the projects I work on daily, as such, language support will be priotized based on what content I actively need embedded.

There is a strong possibility that the terminal utility built will not exist in Rust (Im drawn towards using something like [charm.sh](https://charm.sh/)). If go is chosen to build the terminal, you will likely see the gRPC side of the project built out more, as I harden its use inside non-Rust applications.
