use auden::semantic_index::SemanticIndex;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

async fn run_example() {
    simple_logger::init_with_env().unwrap();

    let tmp_dir = tempdir().unwrap();
    let tmp_path = PathBuf::from(tmp_dir.path());

    let directory = "/home/kcaverly/personal/auden";

    if let Some(mut index) = SemanticIndex::new(tmp_path).await.ok() {
        if let Some(indexing) = index.index_directory(PathBuf::from(directory)).await.ok() {
            indexing.notified().await;

            let query = r#"
se std::collections::HashMap;

// Define a trait for parsing strategies
trait ParsingStrategy {
    fn parse(&self, data: &str);
}

// Implement parsing strategies for different file types
struct JsonParser;
impl ParsingStrategy for JsonParser {
    fn parse(&self, data: &str) {
        println!("Parsing JSON: {}", data);
    }
}

struct XmlParser;
impl ParsingStrategy for XmlParser {
    fn parse(&self, data: &str) {
        println!("Parsing XML: {}", data);
    }
}

fn main() {
    // Create a HashMap to map file extensions to parsing strategies
    let mut extension_to_parser: HashMap<String, Box<dyn ParsingStrategy>> = HashMap::new();

    // Insert parsers into the HashMap
    extension_to_parser.insert("json".to_string(), Box::new(JsonParser));
    extension_to_parser.insert("xml".to_string(), Box::new(XmlParser));

    // Function to parse data based on file extension
    fn parse_data(extension: &str, data: &str, parser_map: &HashMap<String, Box<dyn ParsingStrategy>>) {
        match parser_map.get(extension) {
            Some(parser) => parser.parse(data),
            None => println!("No parser available for the extension: {}", extension),
        }
    }

    // Example usage
    let file_extension = "json";
    let file_data = "{ \"key\": \"value\" }";
    parse_data(file_extension, file_data, &extension_to_parser);
}"#;

            let results = index
                .search_directory(PathBuf::from(directory), 10, query)
                .await
                .unwrap();

            // println!("QUERY: {:?}", &query);
            for result in results {
                let content = fs::read_to_string(result.path).unwrap();
                println!("----- RESULT ----");
                println!("SIMILARITY: {:?}", result.similarity);
                println!(
                    "{:?}",
                    content[result.start_byte..result.end_byte]
                        .split("\n")
                        .into_iter()
                        .collect::<Vec<&str>>()[0]
                        .to_string()
                );
            }
        };
    }
}

fn main() {
    // This is required for surrealdb recursive queries
    // https://github.com/surrealdb/surrealdb/issues/2920
    let stack_size = 10 * 1024 * 1024;

    // Stack frames are generally larger in debug mode.
    #[cfg(debug_assertions)]
    let stack_size = stack_size * 2;

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(stack_size)
        .build()
        .unwrap()
        .block_on(run_example());
}
