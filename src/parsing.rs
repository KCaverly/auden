use crate::embedding::Embedding;
use crate::languages::LanguageConfig;
use crate::semantic_index::FileDetails;
use anyhow;
use std::fs;
use std::ops::Range;
use std::path::PathBuf;
use tree_sitter::{Parser, Query, QueryCursor, TreeCursor};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ContextDocument {
    pub start_byte: usize,
    pub end_byte: usize,
    pub content: String,
}

#[derive(Debug)]
pub(crate) struct FileContext {
    pub(crate) details: FileDetails,
    pub(crate) documents: Vec<ContextDocument>,
    pub(crate) embeddings: Vec<Embedding>,
}

impl FileContext {
    pub(crate) fn document_ids(&self) -> Range<usize> {
        0..self.documents.len()
    }
    pub(crate) fn complete(&self) -> bool {
        !self.embeddings.iter().any(|embed| embed.is_empty())
    }
}

impl Drop for FileContext {
    fn drop(&mut self) {
        self.details.directory_state.job_dropped();
    }
}

pub(crate) struct FileContextParser {}

// Can this just be completely functional?
// I am unsure if we need any attributes on this,
// or if we are just using this for the namespace
impl FileContextParser {
    pub(crate) fn parse_file(
        file_details: &FileDetails,
        config: &LanguageConfig,
    ) -> anyhow::Result<FileContext> {
        log::debug!("parsing file: {:?}", file_details);

        let content = fs::read_to_string(&file_details.path)?;
        let documents = FileContextParser::parse_content(&content, config)?;
        let embeddings = documents
            .iter()
            .map(|_| Vec::new())
            .collect::<Vec<Embedding>>();

        anyhow::Ok(FileContext {
            details: file_details.clone(),
            documents,
            embeddings,
        })
    }

    pub(crate) fn parse_content(
        content: &str,
        config: &LanguageConfig,
    ) -> anyhow::Result<Vec<ContextDocument>> {
        let mut parser = config.get_treesitter_parser()?;
        let tree = parser.parse(&content, None).expect("error parsing tree");
        let query = config.get_treesitter_query()?;

        let mut documents = Vec::new();

        let mut query_cursor = QueryCursor::new();
        for m in query_cursor.matches(&query, tree.root_node(), content.as_bytes()) {
            for capture in m.captures {
                if capture.index == config.item_idx() {
                    documents.push(ContextDocument {
                        start_byte: capture.node.start_byte(),
                        end_byte: capture.node.end_byte(),
                        content: content[capture.node.start_byte()..capture.node.end_byte()]
                            .to_string(),
                    });
                }
            }
        }

        anyhow::Ok(documents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::load_languages;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_rust_parsing() {
        let language_registry = load_languages();
        let rust_config = language_registry.get_config_from_extension("rs").unwrap();
        let test_string = "
        struct CodeContextParser {}

        impl CodeContextParser {
            pub fn parse(content: &str) -> anyhow::Result<()> {
                todo!();
            }
        }
        ";

        let documents = FileContextParser::parse_content(test_string, &rust_config).unwrap();
        let test_documents = vec![
            ContextDocument {
                start_byte: 9,
                end_byte: 36,
                content: "struct CodeContextParser {}".to_string(),
            },
            ContextDocument {
                start_byte: 46,
                end_byte: 183,
                content: "impl CodeContextParser {
            pub fn parse(content: &str) -> anyhow::Result<()> {
                todo!();
            }
        }"
                .to_string(),
            },
        ];

        assert_eq!(documents, test_documents);
    }
}
