use crate::{
    error::AppError,
    models::{Definition, Example, QueryStatus, WordEntry},
    providers::DictionaryProvider,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl OpenAiProvider {
    pub fn from_api_key(api_key: Option<String>, model: Option<String>) -> Result<Self, AppError> {
        let api_key = api_key
            .filter(|key| !key.trim().is_empty())
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .ok_or_else(|| {
                AppError::Config(
                    "缺少 OpenAI API Key，可在界面输入或设置 OPENAI_API_KEY".to_string(),
                )
            })?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent("English2Anki/0.1")
            .build()?;
        Ok(Self {
            client,
            api_key,
            model: model.unwrap_or_else(|| "gpt-4o-mini".to_string()),
        })
    }
}

#[async_trait]
impl DictionaryProvider for OpenAiProvider {
    fn name(&self) -> &'static str {
        "openai"
    }

    async fn lookup(&self, word: &str) -> Result<WordEntry, AppError> {
        let request = ChatRequest {
            model: self.model.clone(),
            response_format: ResponseFormat {
                kind: "json_object".to_string(),
            },
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: "You are a bilingual English dictionary. Return strict JSON only.".to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: format!(
                        "Create a dictionary entry for \"{word}\". JSON schema: {{\"word\":\"...\",\"phonetic\":\"/.../\",\"definitions\":[{{\"part_of_speech\":\"n.\",\"english\":\"...\",\"chinese\":\"...\"}}],\"examples\":[{{\"english\":\"...\",\"chinese\":\"...\"}}]}}"
                    ),
                },
            ],
        };

        let response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(WordEntry::failed(
                word,
                self.name(),
                format!("OpenAI HTTP 状态码 {}", response.status()),
            ));
        }

        let chat: ChatResponse = response.json().await?;
        let content = chat
            .choices
            .first()
            .map(|choice| choice.message.content.as_str())
            .ok_or_else(|| AppError::Parse("OpenAI 返回缺少 choices".to_string()))?;
        let parsed: OpenAiEntry = serde_json::from_str(content)?;

        Ok(WordEntry {
            word: parsed.word.unwrap_or_else(|| word.to_string()),
            phonetic: parsed.phonetic,
            definitions: parsed.definitions,
            examples: parsed.examples,
            source: self.name().to_string(),
            queried_at: chrono::Utc::now().to_rfc3339(),
            status: QueryStatus::Success,
            error: None,
        })
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    response_format: ResponseFormat,
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChoiceMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiEntry {
    word: Option<String>,
    phonetic: Option<String>,
    #[serde(default)]
    definitions: Vec<Definition>,
    #[serde(default)]
    examples: Vec<Example>,
}
