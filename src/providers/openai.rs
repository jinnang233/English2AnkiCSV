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
    source_language: String,
    target_language: String,
}

impl OpenAiProvider {
    pub fn from_config(
        api_key: Option<String>,
        model: Option<String>,
        api_base_url: Option<String>,
        source_language: String,
        target_language: String,
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
            source_language,
            target_language,
        })
    }
}

#[async_trait]
impl DictionaryProvider for OpenAiProvider {
    fn name(&self) -> &'static str {
        "openai-compatible"
    }

    async fn lookup(&self, word: &str) -> Result<WordEntry, AppError> {
        let word_json = serde_json::to_string(word)?;
        let source = format!("{} ({})", self.name(), self.model);
        let source_language_json = serde_json::to_string(&self.source_language)?;
        let target_language_json = serde_json::to_string(&self.target_language)?;
        let request = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                Message {
                    role: "system".to_string(),
                    content: "You are a strict multilingual dictionary engine. Return strict JSON only, without markdown fences.".to_string(),
                },
                Message {
                    role: "user".to_string(),
                    content: format!(
                        r#"
You are a strict multilingual dictionary engine.

The user input is JSON:
{{"query": {word_json}, "source_language": {source_language_json}, "target_language": {target_language_json}}}

Task:
1. First classify the query.
2. Only create a dictionary entry if the query is a real word, lexical item, or common phrase in source_language.
3. Do not invent meanings.
4. Do not explain random strings, hashes, tokens, code, shell commands, URLs, memes, unsafe commands, or text that is not in source_language as dictionary words.
5. If the query is invalid for the requested dictionary, return status "invalid".

Return strict JSON only. No markdown.

Schema:
{{
  "status": "success" | "invalid",
  "reason": "...",
  "word": "...",
  "query_type": "word" | "phrase" | "proper_noun" | "slang" | "code_or_command" | "random_string" | "non_source_language" | "unsafe" | "unknown",
  "phonetic": null,
  "definitions": [
    {{
      "part_of_speech": "n.|v.|adj.|adv.|phr.|...",
      "source": "definition in source_language",
      "target": "definition or translation in target_language"
    }}
  ],
  "examples": [
    {{
      "source": "example sentence in source_language",
      "target": "example translation in target_language"
    }}
  ]
}}

For success, fill phonetic, definitions, and examples.
For invalid, definitions and examples must be empty arrays.
For invalid, phonetic must be null.
For proper nouns, return invalid unless it is commonly used as a lexical item in source_language.
If unsure, return invalid.
"#
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
                source,
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
        let status = parsed.status.as_deref().unwrap_or("invalid");
        let ai_status = Some(status.to_string());

        if status.eq_ignore_ascii_case("invalid") {
            let reason = parsed.reason.clone();
            let query_type = parsed.query_type.clone();
            return Ok(WordEntry::failed(word, source, parsed.invalid_reason())
                .with_ai_result(ai_status, reason, query_type));
        }

        if !status.eq_ignore_ascii_case("success") {
            return Ok(WordEntry::failed(
                word,
                source,
                format!("AI returned unknown dictionary status: {status}"),
            )
            .with_ai_result(
                ai_status,
                parsed.reason.clone(),
                parsed.query_type.clone(),
            ));
        }

        Ok(WordEntry {
            word: parsed.word.unwrap_or_else(|| word.to_string()),
            phonetic: parsed.phonetic,
            definitions: parsed.definitions,
            examples: parsed.examples,
            source,
            queried_at: chrono::Utc::now().to_rfc3339(),
            status: QueryStatus::Success,
            error: None,
            ai_status,
            ai_reason: parsed.reason,
            query_type: parsed.query_type,
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
    status: Option<String>,
    reason: Option<String>,
    word: Option<String>,
    query_type: Option<String>,
    phonetic: Option<String>,
    #[serde(default)]
    definitions: Vec<Definition>,
    #[serde(default)]
    examples: Vec<Example>,
}

impl OpenAiEntry {
    fn invalid_reason(&self) -> String {
        let reason = self
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .unwrap_or("Not a valid English dictionary query");

        match self.query_type.as_deref().map(str::trim) {
            Some(query_type) if !query_type.is_empty() => {
                format!("Invalid dictionary query ({query_type}): {reason}")
            }
            _ => format!("Invalid dictionary query: {reason}"),
        }
    }
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
