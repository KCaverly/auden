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

### Roadmap

This project is primarily an experiment, right now, and very simple.
It may remain as a retrieval only utility, or I may incorporate Agent actions directly into this server as well.
