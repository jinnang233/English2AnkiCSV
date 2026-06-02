pub mod ecdict;
pub mod http_dictionary;
pub mod mock;
pub mod openai;

use crate::{error::AppError, models::WordEntry};
use async_trait::async_trait;

#[async_trait]
pub trait DictionaryProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn lookup(&self, word: &str) -> Result<WordEntry, AppError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Ecdict,
    Mock,
    HttpDictionary,
    OpenAi,
}

impl ProviderKind {
    pub const ALL: [Self; 4] = [Self::Ecdict, Self::Mock, Self::HttpDictionary, Self::OpenAi];
}
