use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum QueryStatus {
    Success,
    Failed,
    PartialSuccess,
    Skipped,
}

impl std::fmt::Display for QueryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryStatus::Success => write!(f, "成功"),
            QueryStatus::Failed => write!(f, "失败"),
            QueryStatus::PartialSuccess => write!(f, "部分成功"),
            QueryStatus::Skipped => write!(f, "已跳过"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Definition {
    pub part_of_speech: Option<String>,
    pub english: Option<String>,
    pub chinese: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Example {
    pub english: String,
    pub chinese: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WordEntry {
    pub word: String,
    pub phonetic: Option<String>,
    pub definitions: Vec<Definition>,
    pub examples: Vec<Example>,
    pub source: String,
    pub queried_at: String,
    pub status: QueryStatus,
    pub error: Option<String>,
}

impl WordEntry {
    pub fn failed(
        word: impl Into<String>,
        source: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            word: word.into(),
            phonetic: None,
            definitions: Vec::new(),
            examples: Vec::new(),
            source: source.into(),
            queried_at: chrono::Utc::now().to_rfc3339(),
            status: QueryStatus::Failed,
            error: Some(error.into()),
        }
    }
}

pub fn parse_words(input: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    input
        .split(',')
        .map(str::trim)
        .filter(|word| !word.is_empty())
        .map(|word| word.to_lowercase())
        .filter(|word| seen.insert(word.clone()))
        .collect()
}
