use anyhow::anyhow;
use tree_sitter::{Language, Parser, Query};
// I think at some point we will need a more efficient
// way to go from
//
//    LanguageName -> LanguageConfig
//    Suffix -> Languageconfig
//
// but for now, maybe just a walking a path naively will work.

#[derive(Debug)]
pub(crate) struct LanguageRegistry {
    languages: Vec<LanguageConfig>,
}

impl LanguageRegistry {
    // This is largely terrible
    // TODO: Make this performant
    pub fn get_config_from_extension(&self, extension: &str) -> Option<&LanguageConfig> {
        for language in &self.languages {
            for language_extension in &language.extensions {
                if language_extension == &extension {
                    return Some(language);
                }
            }
        }

        None
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LanguageConfig {
    name: String,
    extensions: Vec<&'static str>,
    query: &'static str,
}

impl LanguageConfig {
    fn get_treesitter_language(&self) -> anyhow::Result<Language> {
        match self.name.as_str() {
            "Rust" => anyhow::Ok(tree_sitter_rust::language()),
            _ => Err(anyhow!("no treesitter parser available")),
        }
    }
    pub(crate) fn get_treesitter_parser(&self) -> anyhow::Result<Parser> {
        let language = self.get_treesitter_language()?;

        let mut parser = Parser::new();
        parser.set_language(language);
        anyhow::Ok(parser)
    }

    pub(crate) fn get_treesitter_query(&self) -> anyhow::Result<Query> {
        let language = self.get_treesitter_language()?;
        anyhow::Ok(Query::new(language, self.query)?)
    }

    pub(crate) fn item_idx(&self) -> u32 {
        0
    }
}

pub(crate) fn load_languages() -> LanguageRegistry {
    LanguageRegistry {
        languages: vec![LanguageConfig {
            name: "Rust".to_string(),
            extensions: vec!["rs"],
            query: "
                (struct_item) @item
                (impl_item) @item",
        }],
    }
}
