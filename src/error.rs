use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("HTTP 错误: {0}")]
    Http(#[from] reqwest::Error),
    #[error("CSV 错误: {0}")]
    Csv(#[from] csv::Error),
    #[error("配置错误: {0}")]
    Config(String),
    #[error("数据解析错误: {0}")]
    Parse(String),
    #[error("Provider 错误: {0}")]
    Provider(String),
}
