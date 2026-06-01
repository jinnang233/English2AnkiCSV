use crate::{
    error::AppError,
    models::{Definition, Example, QueryStatus, WordEntry},
    providers::DictionaryProvider,
};
use async_trait::async_trait;

pub struct MockProvider;

#[async_trait]
impl DictionaryProvider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn lookup(&self, word: &str) -> Result<WordEntry, AppError> {
        Ok(WordEntry {
            word: word.to_string(),
            phonetic: Some(format!("/{word}/")),
            definitions: vec![Definition {
                part_of_speech: Some("n.".to_string()),
                english: Some(format!("A mock definition for the word \"{word}\".")),
                chinese: Some(format!("{word} 的示例中文释义")),
            }],
            examples: vec![Example {
                english: format!("This is an example sentence for {word}."),
                chinese: Some(format!("这是包含 {word} 的示例句子。")),
            }],
            source: self.name().to_string(),
            queried_at: chrono::Utc::now().to_rfc3339(),
            status: QueryStatus::Success,
            error: None,
        })
    }
}
