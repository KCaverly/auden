use crate::parsers::strategy::ParsingStrategy;

pub(crate) fn rust_strategy() -> ParsingStrategy {
    ParsingStrategy::TreeSitter {
        language: "rust".to_string(),
        query: "
        (enum_item) @item
        (struct_item) @item
        (impl_item) @item
    "
        .to_string(),
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::parsers::strategy::{get_sha, parse_content, ContextDocument};
    use indoc::indoc;
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;

    #[test]
    fn test_rust_parsing() {
        let strategy = rust_strategy();

        let content = indoc! {"
            struct CodeContextParser {}

            impl CodeContextParser {
                pub fn parse(content: &str) -> anyhow::Result<()> {
                    todo!();
                }
            }
            "};

        let path = PathBuf::from("/tmp/foo.rs");

        let parsed = parse_content(&path, content, &strategy).unwrap();

        let content1 = indoc! {"The below is a code snippet from the '/tmp/foo.rs' file.\n```rust\nstruct CodeContextParser {}\n```"}.to_string();
        let sha1 = get_sha(&content1);

        let content2 = indoc! {"
            The below is a code snippet from the '/tmp/foo.rs' file.
            ```rust
            impl CodeContextParser {
                pub fn parse(content: &str) -> anyhow::Result<()> {
                    todo!();
                }
            }
            ```"}
        .to_string();
        let sha2 = get_sha(&content2);
        assert_eq!(
            parsed,
            vec![
                ContextDocument {
                    start_byte: 0,
                    end_byte: 27,
                    content: content1,
                    sha: sha1,
                },
                ContextDocument {
                    start_byte: 29,
                    end_byte: 134,
                    content: content2,
                    sha: sha2,
                }
            ]
        );
    }
}
