use crate::parsers::rust::rust_strategy;
use crate::parsers::strategy::ParsingStrategy;
use anyhow::anyhow;
use std::collections::HashMap;

#[derive(Debug)]
pub(crate) struct ExtensionRegistry {
    extension_strategies: HashMap<String, ParsingStrategy>,
}

impl ExtensionRegistry {
    fn new() -> Self {
        ExtensionRegistry {
            extension_strategies: HashMap::new(),
        }
    }
    fn register(&mut self, extension: String, strategy: ParsingStrategy) {
        self.extension_strategies.insert(extension, strategy);
    }
    pub(crate) fn get_strategy_for_extension(
        &self,
        extension: String,
    ) -> anyhow::Result<&ParsingStrategy> {
        self.extension_strategies
            .get(&extension)
            .ok_or(anyhow!("strategy not found for extension {}", extension))
    }
}

pub(crate) fn load_extensions() -> ExtensionRegistry {
    let mut registry = ExtensionRegistry::new();
    registry.register("rs".to_string(), rust_strategy());

    registry
}
