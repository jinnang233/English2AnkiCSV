use crate::{
    error::AppError,
    models::{Definition, Example, QueryStatus, WordEntry},
    providers::DictionaryProvider,
};
use async_trait::async_trait;
use serde::Deserialize;
use std::time::Duration;

pub struct HttpDictionaryProvider {
    client: reqwest::Client,
}

impl HttpDictionaryProvider {
    pub fn new() -> Result<Self, AppError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("English2Anki/0.1 (+https://dictionaryapi.dev)")
            .build()?;
        Ok(Self { client })
    }
}

#[async_trait]
impl DictionaryProvider for HttpDictionaryProvider {
    fn name(&self) -> &'static str {
        "dictionaryapi.dev"
    }

    async fn lookup(&self, word: &str) -> Result<WordEntry, AppError> {
        let url = format!("https://api.dictionaryapi.dev/api/v2/entries/en/{word}");
        let response = self.client.get(url).send().await?;
        if !response.status().is_success() {
            return Ok(WordEntry::failed(
                word,
                self.name(),
                format!("HTTP 状态码 {}", response.status()),
            ));
        }

        let payload: Vec<ApiEntry> = response.json().await?;
        let first = payload
            .first()
            .ok_or_else(|| AppError::Parse("API 返回为空".to_string()))?;

        let phonetic = first
            .phonetic
            .clone()
            .or_else(|| first.phonetics.iter().find_map(|p| p.text.clone()));

        let mut definitions = Vec::new();
        let mut examples = Vec::new();
        for meaning in &first.meanings {
            for item in &meaning.definitions {
                definitions.push(Definition {
                    part_of_speech: Some(meaning.part_of_speech.clone()),
                    english: Some(item.definition.clone()),
                    chinese: None,
                });
                if let Some(example) = &item.example {
                    examples.push(Example {
                        english: example.clone(),
                        chinese: None,
                    });
                }
            }
        }

        Ok(WordEntry {
            word: first.word.clone().unwrap_or_else(|| word.to_string()),
            phonetic,
            definitions,
            examples,
            source: self.name().to_string(),
            queried_at: chrono::Utc::now().to_rfc3339(),
            status: QueryStatus::Success,
            error: None,
        })
    }
}

#[derive(Debug, Deserialize)]
struct ApiEntry {
    word: Option<String>,
    phonetic: Option<String>,
    #[serde(default)]
    phonetics: Vec<ApiPhonetic>,
    #[serde(default)]
    meanings: Vec<ApiMeaning>,
}

#[derive(Debug, Deserialize)]
struct ApiPhonetic {
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiMeaning {
    #[serde(rename = "partOfSpeech")]
    part_of_speech: String,
    #[serde(default)]
    definitions: Vec<ApiDefinition>,
}

#[derive(Debug, Deserialize)]
struct ApiDefinition {
    definition: String,
    example: Option<String>,
}
