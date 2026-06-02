use crate::{
    error::AppError,
    models::{Definition, QueryStatus, WordEntry},
    providers::DictionaryProvider,
};
use async_trait::async_trait;
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;

const DEFAULT_ECDICT_CSV_URL: &str =
    "https://raw.githubusercontent.com/skywind3000/ECDICT/master/ecdict.csv";

pub struct EcdictProvider {
    csv_url: String,
    cache_path: PathBuf,
    index: Mutex<Option<Arc<HashMap<String, EcdictRecord>>>>,
}

impl EcdictProvider {
    pub fn new() -> Self {
        let csv_url =
            std::env::var("ECDICT_CSV_URL").unwrap_or_else(|_| DEFAULT_ECDICT_CSV_URL.to_string());
        let cache_path = std::env::var("ECDICT_CACHE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_cache_path().join("ecdict.csv"));

        Self {
            csv_url,
            cache_path,
            index: Mutex::new(None),
        }
    }

    async fn ensure_index(&self) -> Result<Arc<HashMap<String, EcdictRecord>>, AppError> {
        let mut index = self.index.lock().await;
        if let Some(index) = index.as_ref() {
            return Ok(Arc::clone(index));
        }

        if !self.cache_path.exists() {
            download_ecdict_csv(&self.csv_url, &self.cache_path).await?;
        }

        let loaded = Arc::new(load_ecdict_csv(&self.cache_path)?);
        *index = Some(Arc::clone(&loaded));
        Ok(loaded)
    }
}

#[async_trait]
impl DictionaryProvider for EcdictProvider {
    fn name(&self) -> &'static str {
        "ECDICT"
    }

    async fn lookup(&self, word: &str) -> Result<WordEntry, AppError> {
        let index = self.ensure_index().await?;
        let key = word.to_lowercase();
        let Some(record) = index.get(&key) else {
            return Ok(WordEntry::failed(
                word,
                self.name(),
                "ECDICT 本地词库未找到该单词",
            ));
        };

        let definitions = build_definitions(record);
        let status = if definitions.is_empty() {
            QueryStatus::PartialSuccess
        } else {
            QueryStatus::Success
        };
        let error = definitions
            .is_empty()
            .then(|| "找到单词，但词条缺少释义".to_string());

        Ok(WordEntry {
            word: record.word.clone(),
            phonetic: empty_to_none(record.phonetic.as_deref()),
            definitions,
            examples: Vec::new(),
            source: format!("{} ({})", self.name(), self.cache_path.display()),
            queried_at: chrono::Utc::now().to_rfc3339(),
            status,
            error,
            ai_status: None,
            ai_reason: None,
            query_type: None,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
struct EcdictRecord {
    word: String,
    phonetic: Option<String>,
    definition: Option<String>,
    translation: Option<String>,
    pos: Option<String>,
}

async fn download_ecdict_csv(url: &str, path: &Path) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let response = reqwest::Client::builder()
        .user_agent("English2Anki/0.1")
        .build()?
        .get(url)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(AppError::Provider(format!(
            "下载 ECDICT 失败，HTTP 状态码 {}",
            response.status()
        )));
    }

    let bytes = response.bytes().await?;
    std::fs::write(path, bytes)?;
    Ok(())
}

fn load_ecdict_csv(path: &Path) -> Result<HashMap<String, EcdictRecord>, AppError> {
    let mut reader = csv::ReaderBuilder::new().flexible(true).from_path(path)?;
    let mut index = HashMap::new();

    for result in reader.deserialize::<EcdictRecord>() {
        let record = result?;
        if record.word.trim().is_empty() {
            continue;
        }
        index.insert(record.word.to_lowercase(), record);
    }

    Ok(index)
}

fn build_definitions(record: &EcdictRecord) -> Vec<Definition> {
    let chinese_lines = split_lines(record.translation.as_deref());
    let english_lines = split_lines(record.definition.as_deref());
    let fallback_pos = empty_to_none(record.pos.as_deref());
    let len = chinese_lines.len().max(english_lines.len());

    (0..len)
        .map(|index| {
            let chinese = chinese_lines.get(index).cloned();
            let english = english_lines.get(index).cloned();
            let (part_of_speech, chinese) = chinese
                .map(|line| split_pos_prefix(&line))
                .unwrap_or((None, None));

            Definition {
                part_of_speech: part_of_speech.or_else(|| fallback_pos.clone()),
                source: english,
                target: chinese,
            }
        })
        .filter(|definition| definition.source.is_some() || definition.target.is_some())
        .collect()
}

fn split_lines(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn split_pos_prefix(line: &str) -> (Option<String>, Option<String>) {
    let Some((first, rest)) = line.split_once(' ') else {
        return (None, empty_to_none(Some(line)));
    };

    if looks_like_pos(first) {
        (Some(first.to_string()), empty_to_none(Some(rest)))
    } else {
        (None, empty_to_none(Some(line)))
    }
}

fn looks_like_pos(value: &str) -> bool {
    let value = value.trim();
    value.ends_with('.') || matches!(value, "n" | "v" | "vi" | "vt" | "adj" | "adv" | "prep")
}

fn empty_to_none(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn default_cache_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("English2Anki")
        .join("dictionaries")
}
