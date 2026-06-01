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
    api_base_url: String,
}

impl OpenAiProvider {
    pub fn from_config(
        api_key: Option<String>,
        model: Option<String>,
        api_base_url: Option<String>,
    ) -> Result<Self, AppError> {
        let api_key = api_key
            .filter(|key| !key.trim().is_empty())
            .or_else(|| std::env::var("AI_API_KEY").ok())
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .ok_or_else(|| {
                AppError::Config(
                    "缺少 AI API Key，可在界面输入或设置 AI_API_KEY / OPENAI_API_KEY".to_string(),
                )
            })?;
        let api_base_url = api_base_url
            .filter(|url| !url.trim().is_empty())
            .or_else(|| std::env::var("AI_API_BASE_URL").ok())
            .or_else(|| std::env::var("OPENAI_API_BASE_URL").ok())
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent("English2Anki/0.1")
            .build()?;
        Ok(Self {
            client,
            api_key,
            model: model.unwrap_or_else(|| "gpt-4o-mini".to_string()),
            api_base_url,
        })
    }
}

#[async_trait]
impl DictionaryProvider for OpenAiProvider {
    fn name(&self) -> &'static str {
        "openai-compatible"
    }

    async fn lookup(&self, word: &str) -> Result<WordEntry, AppError> {
        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: "You are a bilingual English dictionary. Return strict JSON only, without markdown fences.".to_string(),
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
            .post(chat_completions_url(&self.api_base_url))
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(WordEntry::failed(
                word,
                self.name(),
                format!("AI API HTTP 状态码 {}", response.status()),
            ));
        }

        let chat: ChatResponse = response.json().await?;
        let content = chat
            .choices
            .first()
            .map(|choice| choice.message.content.as_str())
            .ok_or_else(|| AppError::Parse("AI API 返回缺少 choices".to_string()))?;
        let parsed: OpenAiEntry = serde_json::from_str(clean_json_content(content))?;

        Ok(WordEntry {
            word: parsed.word.unwrap_or_else(|| word.to_string()),
            phonetic: parsed.phonetic,
            definitions: parsed.definitions,
            examples: parsed.examples,
            source: format!("{} ({})", self.name(), self.model),
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
}

#[derive(Debug, Serialize)]
struct Message {
    role: String,
    content: String,
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

fn chat_completions_url(api_base_url: &str) -> String {
    let trimmed = api_base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

fn clean_json_content(content: &str) -> &str {
    let trimmed = content.trim();
    if let Some(stripped) = trimmed.strip_prefix("```json") {
        return stripped.trim().trim_end_matches("```").trim();
    }
    if let Some(stripped) = trimmed.strip_prefix("```") {
        return stripped.trim().trim_end_matches("```").trim();
    }
    trimmed
}
