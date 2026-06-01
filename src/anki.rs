use crate::{
    error::AppError,
    models::{Definition, Example, WordEntry},
};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Write, path::Path};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnkiExportTemplate {
    Detailed,
    BasicFrontBack,
    ChineseToEnglish,
    TypeAnswer,
}

impl AnkiExportTemplate {
    pub const ALL: [Self; 4] = [
        Self::Detailed,
        Self::BasicFrontBack,
        Self::ChineseToEnglish,
        Self::TypeAnswer,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Detailed => "词条详情模板",
            Self::BasicFrontBack => "Basic 正反面卡",
            Self::ChineseToEnglish => "中文到英文反向卡",
            Self::TypeAnswer => "输入答案卡",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnkiExportFormat {
    Tsv,
    Csv,
}

impl AnkiExportFormat {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tsv => "TSV",
            Self::Csv => "CSV",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnkiField {
    Word,
    Phonetic,
    PartOfSpeech,
    EnglishDefinition,
    ChineseDefinition,
    Examples,
    ExampleTranslations,
    Source,
}

impl AnkiField {
    pub const ALL: [Self; 8] = [
        Self::Word,
        Self::Phonetic,
        Self::PartOfSpeech,
        Self::EnglishDefinition,
        Self::ChineseDefinition,
        Self::Examples,
        Self::ExampleTranslations,
        Self::Source,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Word => "单词",
            Self::Phonetic => "音标",
            Self::PartOfSpeech => "词性",
            Self::EnglishDefinition => "英文释义",
            Self::ChineseDefinition => "中文释义",
            Self::Examples => "例句",
            Self::ExampleTranslations => "例句翻译",
            Self::Source => "来源",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnkiExportOptions {
    pub include_phonetic: bool,
    pub include_examples: bool,
    pub include_english_definition: bool,
    pub include_chinese_definition: bool,
    pub include_source: bool,
    pub include_part_of_speech_hint: bool,
    pub front_fields: Vec<AnkiField>,
    pub back_fields: Vec<AnkiField>,
}

impl Default for AnkiExportOptions {
    fn default() -> Self {
        Self {
            include_phonetic: true,
            include_examples: true,
            include_english_definition: true,
            include_chinese_definition: true,
            include_source: true,
            include_part_of_speech_hint: false,
            front_fields: vec![AnkiField::Word],
            back_fields: vec![
                AnkiField::Phonetic,
                AnkiField::PartOfSpeech,
                AnkiField::ChineseDefinition,
                AnkiField::EnglishDefinition,
                AnkiField::Examples,
            ],
        }
    }
}

pub fn export_anki(
    entries: &[WordEntry],
    path: &Path,
    template: AnkiExportTemplate,
    format: AnkiExportFormat,
    options: AnkiExportOptions,
) -> Result<(), AppError> {
    let rows: Vec<Vec<String>> = entries
        .iter()
        .filter(|entry| !entry.definitions.is_empty() || !entry.examples.is_empty())
        .map(|entry| render_row(entry, template, &options))
        .collect();

    match format {
        AnkiExportFormat::Tsv => write_tsv(path, &rows),
        AnkiExportFormat::Csv => write_csv(path, &rows),
    }
}

fn write_tsv(path: &Path, rows: &[Vec<String>]) -> Result<(), AppError> {
    let mut file = File::create(path)?;
    for row in rows {
        let line = row
            .iter()
            .map(|field| sanitize_tsv(field))
            .collect::<Vec<_>>()
            .join("\t");
        writeln!(file, "{line}")?;
    }
    Ok(())
}

fn write_csv(path: &Path, rows: &[Vec<String>]) -> Result<(), AppError> {
    let mut writer = csv::WriterBuilder::new().from_path(path)?;
    for row in rows {
        writer.write_record(row.iter().map(|field| sanitize_common(field)))?;
    }
    writer.flush()?;
    Ok(())
}

fn render_row(
    entry: &WordEntry,
    template: AnkiExportTemplate,
    options: &AnkiExportOptions,
) -> Vec<String> {
    match template {
        AnkiExportTemplate::Detailed => vec![
            entry.word.clone(),
            if options.include_phonetic {
                entry.phonetic.clone().unwrap_or_default()
            } else {
                String::new()
            },
            definitions_text(&entry.definitions, options),
            examples_text(&entry.examples, false),
            examples_text(&entry.examples, true),
            if options.include_source {
                entry.source.clone()
            } else {
                String::new()
            },
        ],
        AnkiExportTemplate::BasicFrontBack => vec![
            fields_text(entry, &options.front_fields, options),
            fields_text(entry, &options.back_fields, options),
        ],
        AnkiExportTemplate::ChineseToEnglish => vec![
            chinese_prompt(entry, options),
            fields_text(entry, &chinese_to_english_back_fields(options), options),
        ],
        AnkiExportTemplate::TypeAnswer => vec![
            chinese_prompt(entry, options),
            entry.word.clone(),
            fields_text(
                entry,
                &[
                    AnkiField::Phonetic,
                    AnkiField::PartOfSpeech,
                    AnkiField::EnglishDefinition,
                    AnkiField::Examples,
                    AnkiField::Source,
                ],
                options,
            ),
        ],
    }
}

fn fields_text(entry: &WordEntry, fields: &[AnkiField], options: &AnkiExportOptions) -> String {
    fields
        .iter()
        .filter_map(|field| {
            let text = match field {
                AnkiField::Word => entry.word.clone(),
                AnkiField::Phonetic if options.include_phonetic => {
                    entry.phonetic.clone().unwrap_or_default()
                }
                AnkiField::PartOfSpeech => part_of_speech_text(&entry.definitions),
                AnkiField::EnglishDefinition if options.include_english_definition => {
                    english_definitions_text(&entry.definitions)
                }
                AnkiField::ChineseDefinition if options.include_chinese_definition => {
                    chinese_definitions_text(&entry.definitions)
                }
                AnkiField::Examples if options.include_examples => {
                    examples_text(&entry.examples, false)
                }
                AnkiField::ExampleTranslations if options.include_examples => {
                    examples_text(&entry.examples, true)
                }
                AnkiField::Source if options.include_source => entry.source.clone(),
                _ => String::new(),
            };
            (!text.trim().is_empty()).then_some(text)
        })
        .collect::<Vec<_>>()
        .join("<br>")
}

fn chinese_to_english_back_fields(options: &AnkiExportOptions) -> Vec<AnkiField> {
    let mut fields = vec![AnkiField::Word, AnkiField::Phonetic];
    if options.include_part_of_speech_hint {
        fields.push(AnkiField::PartOfSpeech);
    }
    fields.extend([AnkiField::EnglishDefinition, AnkiField::Examples]);
    fields
}

fn definitions_text(definitions: &[Definition], options: &AnkiExportOptions) -> String {
    definitions
        .iter()
        .map(|definition| {
            let mut parts = Vec::new();
            if let Some(pos) = &definition.part_of_speech {
                parts.push(pos.clone());
            }
            if options.include_chinese_definition {
                if let Some(chinese) = &definition.chinese {
                    parts.push(chinese.clone());
                }
            }
            if options.include_english_definition {
                if let Some(english) = &definition.english {
                    parts.push(english.clone());
                }
            }
            parts.join(" ")
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("<br>")
}

fn english_definitions_text(definitions: &[Definition]) -> String {
    definitions
        .iter()
        .filter_map(|d| d.english.as_deref())
        .collect::<Vec<_>>()
        .join("; ")
}

fn chinese_definitions_text(definitions: &[Definition]) -> String {
    definitions
        .iter()
        .filter_map(|d| d.chinese.as_deref())
        .collect::<Vec<_>>()
        .join("; ")
}

fn part_of_speech_text(definitions: &[Definition]) -> String {
    let mut seen = std::collections::HashSet::new();
    definitions
        .iter()
        .filter_map(|d| d.part_of_speech.as_deref())
        .filter(|pos| seen.insert((*pos).to_string()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn examples_text(examples: &[Example], translations_only: bool) -> String {
    examples
        .iter()
        .filter_map(|example| {
            if translations_only {
                example.chinese.clone()
            } else if example.english.trim().is_empty() {
                None
            } else {
                Some(format!("Example: {}", example.english))
            }
        })
        .collect::<Vec<_>>()
        .join("<br>")
}

fn chinese_prompt(entry: &WordEntry, options: &AnkiExportOptions) -> String {
    let definitions = entry
        .definitions
        .iter()
        .filter_map(|definition| {
            let chinese = definition.chinese.as_ref()?;
            if options.include_part_of_speech_hint {
                Some(match &definition.part_of_speech {
                    Some(pos) => format!("{pos}. {chinese}"),
                    None => chinese.clone(),
                })
            } else {
                Some(chinese.clone())
            }
        })
        .collect::<Vec<_>>()
        .join("; ");

    if definitions.is_empty() {
        english_definitions_text(&entry.definitions)
    } else {
        definitions
    }
}

fn sanitize_common(input: &str) -> String {
    input
        .replace("\r\n", "<br>")
        .replace('\n', "<br>")
        .replace('\r', "<br>")
}

fn sanitize_tsv(input: &str) -> String {
    sanitize_common(input).replace('\t', " ")
}
