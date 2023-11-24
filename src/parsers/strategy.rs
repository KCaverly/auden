use anyhow::anyhow;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use tree_sitter::{Language, Parser, Query, QueryCursor};

use crate::semantic_index::FileDetails;

#[derive(Debug, Clone)]
pub(crate) enum ParsingStrategy {
    TreeSitter { language: String, query: String },
}

pub(crate) fn get_sha(content: &str) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hasher.finalize()[..].to_vec()
}

fn get_treesitter_language(language_name: &str) -> anyhow::Result<Language> {
    match language_name {
        "rust" => anyhow::Ok(tree_sitter_rust::language()),
        _ => Err(anyhow!(
            "no treesitter parser available for {}",
            language_name
        )),
    }
}

fn parse_treesitter(
    content: &str,
    language_name: &str,
    query: &str,
    path: &str,
) -> anyhow::Result<Vec<ContextDocument>> {
    // Get Treesitter Parser
    let language = get_treesitter_language(language_name)?;
    let mut parser = Parser::new();
    parser.set_language(language)?;
    let query = Query::new(language, query)?;

    let tree = parser.parse(&content, None).expect("");

    let mut documents = Vec::new();
    let mut query_cursor = QueryCursor::new();
    for m in query_cursor.matches(&query, tree.root_node(), content.as_bytes()) {
        for capture in m.captures {
            if capture.index == 0 {
                let span = &content[capture.node.start_byte()..capture.node.end_byte()];
                let filled = format!(
                    "The below is a code snippet from the '{path}' file.\n```{language_name}\n{span}\n```"
                );
                let sha = get_sha(&filled);
                documents.push(ContextDocument {
                    start_byte: capture.node.start_byte(),
                    end_byte: capture.node.end_byte(),
                    content: filled.to_string(),
                    sha,
                });
            }
        }
    }

    anyhow::Ok(documents)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ContextDocument {
    pub start_byte: usize,
    pub end_byte: usize,
    pub content: String,
    pub sha: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct FileContext {
    pub(crate) details: FileDetails,
    pub(crate) documents: Vec<ContextDocument>,
    pub(crate) embeddings: Vec<Vec<f32>>,
}

impl FileContext {
    pub(crate) fn document_ids(&self) -> Vec<usize> {
        let mut ids = Vec::new();
        for (idx, embedding) in self.embeddings.iter().enumerate() {
            if embedding.is_empty() {
                ids.push(idx);
            }
        }
        ids
    }
    pub(crate) fn complete(&self) -> bool {
        let complete = !self.embeddings.iter().any(|embed| embed.is_empty());
        complete
    }
}

impl Drop for FileContext {
    fn drop(&mut self) {
        self.details.directory_state.job_dropped();
    }
}
pub(crate) fn parse_file(
    details: FileDetails,
    strategy: &ParsingStrategy,
) -> anyhow::Result<FileContext> {
    let content = fs::read_to_string(&details.path)?;

    let documents = parse_content(&details.path, content.as_str(), strategy)?;
    let embeddings = documents.iter().map(|_| vec![]).collect::<Vec<Vec<f32>>>();

    anyhow::Ok(FileContext {
        details,
        documents,
        embeddings,
    })
}

pub(crate) fn parse_content(
    path: &PathBuf,
    content: &str,
    strategy: &ParsingStrategy,
) -> anyhow::Result<Vec<ContextDocument>> {
    match strategy {
        ParsingStrategy::TreeSitter { language, query } => parse_treesitter(
            content,
            language,
            query,
            path.to_str()
                .ok_or(anyhow!("failed to parse path to string"))?,
        ),
    }
}
