use crate::{
    error::AppError,
    models::{Definition, Example, QueryStatus, WordEntry},
    providers::DictionaryProvider,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path};

const MOCK_DICTIONARY_FILE: &str = "mock_dictionary.json";
const DEFAULT_MOCK_DICTIONARY_JSON: &str = include_str!("../../mock_dictionary.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MockDictionary {
    #[serde(default)]
    entries: HashMap<String, MockEntry>,
    default_entry: MockEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MockEntry {
    #[serde(default)]
    phonetic: Option<String>,
    #[serde(default)]
    definitions: Vec<Definition>,
    #[serde(default)]
    examples: Vec<Example>,
}

pub struct MockProvider {
    dictionary: MockDictionary,
}

impl MockProvider {
    pub fn new() -> Result<Self, AppError> {
        let path = std::env::current_dir()?.join(MOCK_DICTIONARY_FILE);
        ensure_mock_dictionary_file(&path)?;

        let content = std::fs::read_to_string(&path)?;
        let dictionary = normalize_dictionary(serde_json::from_str(&content)?);

        Ok(Self { dictionary })
    }
}

#[async_trait]
impl DictionaryProvider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn lookup(&self, word: &str) -> Result<WordEntry, AppError> {
        let key = word.to_lowercase();
        let entry = self
            .dictionary
            .entries
            .get(&key)
            .unwrap_or(&self.dictionary.default_entry);

        Ok(WordEntry {
            word: word.to_string(),
            phonetic: entry.phonetic.clone().or_else(|| Some(format!("/{word}/"))),
            definitions: render_definitions(&entry.definitions, word),
            examples: render_examples(&entry.examples, word),
            source: self.name().to_string(),
            queried_at: chrono::Utc::now().to_rfc3339(),
            status: QueryStatus::Success,
            error: None,
            ai_status: None,
            ai_reason: None,
            query_type: None,
        })
    }
}

fn ensure_mock_dictionary_file(path: &Path) -> Result<(), AppError> {
    if path.exists() {
        return Ok(());
    }

    std::fs::write(path, DEFAULT_MOCK_DICTIONARY_JSON)?;
    Ok(())
}

fn normalize_dictionary(mut dictionary: MockDictionary) -> MockDictionary {
    dictionary.entries = dictionary
        .entries
        .into_iter()
        .map(|(word, entry)| (word.to_lowercase(), entry))
        .collect();
    dictionary
}

fn render_definitions(definitions: &[Definition], word: &str) -> Vec<Definition> {
    definitions
        .iter()
        .cloned()
        .map(|mut definition| {
            definition.source = definition.source.map(|value| render_template(&value, word));
            definition.target = definition.target.map(|value| render_template(&value, word));
            definition
        })
        .collect()
}

fn render_examples(examples: &[Example], word: &str) -> Vec<Example> {
    examples
        .iter()
        .cloned()
        .map(|mut example| {
            example.source = render_template(&example.source, word);
            example.target = example.target.map(|value| render_template(&value, word));
            example
        })
        .collect()
}

fn render_template(value: &str, word: &str) -> String {
    value.replace("{word}", word)
}
