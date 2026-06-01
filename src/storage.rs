use crate::{error::AppError, models::WordEntry};
use std::{fs, path::Path};

pub fn save_entries(path: &Path, entries: &[WordEntry]) -> Result<(), AppError> {
    let json = serde_json::to_string_pretty(entries)?;
    fs::write(path, json)?;
    Ok(())
}

pub fn load_entries(path: &Path) -> Result<Vec<WordEntry>, AppError> {
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}
