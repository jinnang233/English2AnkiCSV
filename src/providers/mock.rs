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
        let (chinese, example_english, example_chinese) = mock_content(word);

        Ok(WordEntry {
            word: word.to_string(),
            phonetic: Some(format!("/{word}/")),
            definitions: vec![Definition {
                part_of_speech: Some("n.".to_string()),
                english: Some(format!("A mock definition for the word \"{word}\".")),
                chinese: Some(chinese.to_string()),
            }],
            examples: vec![Example {
                english: example_english.to_string(),
                chinese: Some(example_chinese.to_string()),
            }],
            source: self.name().to_string(),
            queried_at: chrono::Utc::now().to_rfc3339(),
            status: QueryStatus::Success,
            error: None,
        })
    }
}

fn mock_content(word: &str) -> (&'static str, &'static str, &'static str) {
    match word.to_lowercase().as_str() {
        "apple" => (
            "苹果；苹果树",
            "I ate an apple after lunch.",
            "午饭后我吃了一个苹果。",
        ),
        "abandon" => (
            "放弃；抛弃",
            "They had to abandon the old plan.",
            "他们不得不放弃原来的计划。",
        ),
        "beautiful" => (
            "美丽的；出色的",
            "The garden looks beautiful in spring.",
            "春天的花园看起来很美。",
        ),
        "network" => (
            "网络；人际网",
            "The company built a secure network.",
            "公司搭建了一个安全的网络。",
        ),
        _ => (
            "示例释义",
            "This is a clear example sentence.",
            "这是一个清晰的例句。",
        ),
    }
}
