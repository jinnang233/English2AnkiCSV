use crate::{
    anki::{export_anki, AnkiExportFormat, AnkiExportOptions, AnkiExportTemplate, AnkiField},
    error::AppError,
    models::{parse_words, QueryStatus, WordEntry},
    providers::{
        http_dictionary::HttpDictionaryProvider, mock::MockProvider, openai::OpenAiProvider,
        DictionaryProvider, ProviderKind,
    },
    storage,
};
use eframe::egui;
use futures::{stream, StreamExt};
use serde::Deserialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{runtime::Runtime, sync::mpsc};

pub struct English2AnkiApp {
    input_words: String,
    provider_kind: ProviderKind,
    api_key: String,
    openai_model: String,
    config_path: String,
    force_refresh: bool,
    concurrency: usize,
    entries: Vec<WordEntry>,
    statuses: HashMap<String, QueryLine>,
    runtime: Runtime,
    receiver: Option<mpsc::UnboundedReceiver<QueryMessage>>,
    is_querying: bool,
    progress_done: usize,
    progress_total: usize,
    last_error: Option<String>,
    save_path: String,
    load_path: String,
    export_path: String,
    export_template: AnkiExportTemplate,
    export_format: AnkiExportFormat,
    export_options: AnkiExportOptions,
}

#[derive(Debug, Clone)]
struct QueryLine {
    status: QueryStatus,
    source: String,
    error: Option<String>,
}

enum QueryMessage {
    Started { total: usize },
    Entry(WordEntry),
    Skipped(String),
    Done,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    openai_api_key: Option<String>,
    openai_model: Option<String>,
}

impl English2AnkiApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_cjk_font(&cc.egui_ctx);

        Self {
            input_words: "apple, abandon, beautiful, network".to_string(),
            provider_kind: ProviderKind::Mock,
            api_key: std::env::var("OPENAI_API_KEY").unwrap_or_default(),
            openai_model: "gpt-4o-mini".to_string(),
            config_path: String::new(),
            force_refresh: false,
            concurrency: 4,
            entries: Vec::new(),
            statuses: HashMap::new(),
            runtime: Runtime::new().expect("failed to create tokio runtime"),
            receiver: None,
            is_querying: false,
            progress_done: 0,
            progress_total: 0,
            last_error: None,
            save_path: "words.json".to_string(),
            load_path: "words.json".to_string(),
            export_path: "anki_export.tsv".to_string(),
            export_template: AnkiExportTemplate::Detailed,
            export_format: AnkiExportFormat::Tsv,
            export_options: AnkiExportOptions::default(),
        }
    }

    fn poll_query_messages(&mut self, ctx: &egui::Context) {
        let mut should_clear_receiver = false;
        if let Some(receiver) = &mut self.receiver {
            while let Ok(message) = receiver.try_recv() {
                match message {
                    QueryMessage::Started { total } => {
                        self.progress_total = total;
                        self.progress_done = 0;
                        self.statuses.clear();
                    }
                    QueryMessage::Entry(entry) => {
                        self.progress_done += 1;
                        self.statuses.insert(
                            entry.word.clone(),
                            QueryLine {
                                status: entry.status.clone(),
                                source: entry.source.clone(),
                                error: entry.error.clone(),
                            },
                        );
                        upsert_entry(&mut self.entries, entry);
                    }
                    QueryMessage::Skipped(word) => {
                        self.progress_done += 1;
                        self.statuses.insert(
                            word,
                            QueryLine {
                                status: QueryStatus::Skipped,
                                source: "local-json".to_string(),
                                error: None,
                            },
                        );
                    }
                    QueryMessage::Done => {
                        self.is_querying = false;
                        should_clear_receiver = true;
                    }
                }
                ctx.request_repaint();
            }
        }
        if should_clear_receiver {
            self.receiver = None;
        }
    }

    fn start_query(&mut self) {
        let words = parse_words(&self.input_words);
        if words.is_empty() {
            self.last_error = Some("请输入至少一个英文单词".to_string());
            return;
        }

        let provider = match build_provider(
            self.provider_kind,
            &self.api_key,
            &self.openai_model,
            &self.config_path,
        ) {
            Ok(provider) => provider,
            Err(error) => {
                self.last_error = Some(error.to_string());
                return;
            }
        };

        let existing = Arc::new(
            self.entries
                .iter()
                .map(|entry| (entry.word.to_lowercase(), entry.clone()))
                .collect::<HashMap<_, _>>(),
        );
        let force_refresh = self.force_refresh;
        let concurrency = self.concurrency.max(1);
        let (sender, receiver) = mpsc::unbounded_channel();

        self.receiver = Some(receiver);
        self.is_querying = true;
        self.progress_done = 0;
        self.progress_total = words.len();
        self.last_error = None;

        self.runtime.spawn(async move {
            let _ = sender.send(QueryMessage::Started { total: words.len() });
            stream::iter(words)
                .map(|word| {
                    let sender = sender.clone();
                    let provider = Arc::clone(&provider);
                    let existing = Arc::clone(&existing);
                    async move {
                        if !force_refresh && existing.contains_key(&word) {
                            let _ = sender.send(QueryMessage::Skipped(word));
                            return;
                        }

                        let result = provider.lookup(&word).await.unwrap_or_else(|error| {
                            WordEntry::failed(word.clone(), provider.name(), error.to_string())
                        });
                        let _ = sender.send(QueryMessage::Entry(result));
                    }
                })
                .buffer_unordered(concurrency)
                .collect::<Vec<_>>()
                .await;
            let _ = sender.send(QueryMessage::Done);
        });
    }

    fn save_json(&mut self) {
        let path = PathBuf::from(self.save_path.trim());
        match storage::save_entries(&path, &self.entries) {
            Ok(()) => self.last_error = Some(format!("已保存 JSON: {}", path.display())),
            Err(error) => self.last_error = Some(error.to_string()),
        }
    }

    fn load_json(&mut self) {
        let path = PathBuf::from(self.load_path.trim());
        match storage::load_entries(&path) {
            Ok(entries) => {
                self.entries = entries;
                self.last_error = Some(format!("已加载 JSON: {}", path.display()));
            }
            Err(error) => self.last_error = Some(error.to_string()),
        }
    }

    fn export_anki(&mut self) {
        let path = PathBuf::from(self.export_path.trim());
        let options = self.export_options.clone();
        match export_anki(
            &self.entries,
            &path,
            self.export_template,
            self.export_format,
            options,
        ) {
            Ok(()) => self.last_error = Some(format!("已导出 Anki 文件: {}", path.display())),
            Err(error) => self.last_error = Some(error.to_string()),
        }
    }
}

impl eframe::App for English2AnkiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_query_messages(ctx);

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("English2Anki");
                if self.is_querying {
                    ui.spinner();
                    ui.label(format!(
                        "查询中 {}/{}",
                        self.progress_done, self.progress_total
                    ));
                }
            });
        });

        egui::SidePanel::left("settings")
            .resizable(true)
            .default_width(330.0)
            .show(ctx, |ui| {
                ui.heading("查询");
                ui.label("英文逗号分割的单词列表");
                ui.add(
                    egui::TextEdit::multiline(&mut self.input_words)
                        .desired_rows(5)
                        .desired_width(f32::INFINITY),
                );

                egui::ComboBox::from_label("数据源")
                    .selected_text(self.provider_kind.label())
                    .show_ui(ui, |ui| {
                        for kind in ProviderKind::ALL {
                            ui.selectable_value(&mut self.provider_kind, kind, kind.label());
                        }
                    });

                ui.label("OpenAI API Key");
                ui.add(egui::TextEdit::singleline(&mut self.api_key).password(true));
                ui.label("OpenAI 模型");
                ui.text_edit_singleline(&mut self.openai_model);
                ui.label("配置文件路径（预留，可用于扩展 Provider 配置）");
                ui.text_edit_singleline(&mut self.config_path);
                ui.checkbox(&mut self.force_refresh, "强制刷新已有单词");
                ui.add(egui::Slider::new(&mut self.concurrency, 1..=12).text("并发数"));

                if ui
                    .add_enabled(!self.is_querying, egui::Button::new("开始查询"))
                    .clicked()
                {
                    self.start_query();
                }

                ui.separator();
                ui.heading("JSON");
                path_row(ui, "保存路径", &mut self.save_path, true);
                if ui.button("保存 JSON").clicked() {
                    self.save_json();
                }
                path_row(ui, "加载路径", &mut self.load_path, false);
                if ui.button("加载 JSON").clicked() {
                    self.load_json();
                }

                ui.separator();
                render_anki_settings(ui, self);

                ui.separator();
                ui.heading("消息");
                if let Some(error) = &self.last_error {
                    ui.label(error);
                } else {
                    ui.label("无错误");
                }
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("查询状态");
            let progress = if self.progress_total == 0 {
                0.0
            } else {
                self.progress_done as f32 / self.progress_total as f32
            };
            ui.add(egui::ProgressBar::new(progress).show_percentage());

            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("status_grid")
                    .striped(true)
                    .num_columns(4)
                    .show(ui, |ui| {
                        ui.strong("单词");
                        ui.strong("状态");
                        ui.strong("来源");
                        ui.strong("错误");
                        ui.end_row();

                        let mut rows = self.statuses.iter().collect::<Vec<_>>();
                        rows.sort_by(|(left, _), (right, _)| left.cmp(right));
                        for (word, line) in rows {
                            ui.label(word);
                            ui.label(line.status.to_string());
                            ui.label(&line.source);
                            ui.label(line.error.as_deref().unwrap_or(""));
                            ui.end_row();
                        }
                    });

                ui.separator();
                ui.heading(format!("结果预览（{} 条）", self.entries.len()));
                for entry in &self.entries {
                    ui.collapsing(format!("{} - {}", entry.word, entry.status), |ui| {
                        ui.label(format!("音标: {}", entry.phonetic.as_deref().unwrap_or("")));
                        ui.label(format!("来源: {}", entry.source));
                        ui.label(format!("查询时间: {}", entry.queried_at));
                        if let Some(error) = &entry.error {
                            ui.colored_label(egui::Color32::RED, error);
                        }
                        for definition in &entry.definitions {
                            ui.label(format!(
                                "{} {} {}",
                                definition.part_of_speech.as_deref().unwrap_or(""),
                                definition.chinese.as_deref().unwrap_or(""),
                                definition.english.as_deref().unwrap_or("")
                            ));
                        }
                        for example in &entry.examples {
                            ui.label(format!("Example: {}", example.english));
                            if let Some(chinese) = &example.chinese {
                                ui.label(chinese);
                            }
                        }
                    });
                }
            });
        });
    }
}

fn render_anki_settings(ui: &mut egui::Ui, app: &mut English2AnkiApp) {
    ui.heading("Anki 导出");
    egui::ComboBox::from_label("格式")
        .selected_text(app.export_format.label())
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut app.export_format, AnkiExportFormat::Tsv, "TSV");
            ui.selectable_value(&mut app.export_format, AnkiExportFormat::Csv, "CSV");
        });
    egui::ComboBox::from_label("模板")
        .selected_text(app.export_template.label())
        .show_ui(ui, |ui| {
            for template in AnkiExportTemplate::ALL {
                ui.selectable_value(&mut app.export_template, template, template.label());
            }
        });

    ui.checkbox(&mut app.export_options.include_phonetic, "包含音标");
    ui.checkbox(&mut app.export_options.include_examples, "包含例句");
    ui.checkbox(
        &mut app.export_options.include_english_definition,
        "包含英文释义",
    );
    ui.checkbox(
        &mut app.export_options.include_chinese_definition,
        "包含中文释义",
    );
    ui.checkbox(&mut app.export_options.include_source, "包含来源");
    ui.checkbox(
        &mut app.export_options.include_part_of_speech_hint,
        "中文反向卡正面显示词性提示",
    );

    ui.collapsing("Basic 正面字段", |ui| {
        field_picker(ui, &mut app.export_options.front_fields);
    });
    ui.collapsing("Basic 背面字段", |ui| {
        field_picker(ui, &mut app.export_options.back_fields);
    });

    path_row(ui, "导出路径", &mut app.export_path, true);
    if ui.button("导出 Anki 文件").clicked() {
        app.export_anki();
    }
}

fn field_picker(ui: &mut egui::Ui, fields: &mut Vec<AnkiField>) {
    for field in AnkiField::ALL {
        let mut enabled = fields.contains(&field);
        if ui.checkbox(&mut enabled, field.label()).changed() {
            if enabled {
                fields.push(field);
            } else {
                fields.retain(|item| *item != field);
            }
        }
    }
}

fn path_row(ui: &mut egui::Ui, label: &str, path: &mut String, save: bool) {
    ui.label(label);
    ui.horizontal(|ui| {
        ui.text_edit_singleline(path);
        if ui.button("选择").clicked() {
            let selected = if save {
                rfd::FileDialog::new().save_file()
            } else {
                rfd::FileDialog::new().pick_file()
            };
            if let Some(selected) = selected {
                *path = selected.display().to_string();
            }
        }
    });
}

fn build_provider(
    kind: ProviderKind,
    api_key: &str,
    model: &str,
    config_path: &str,
) -> Result<Arc<dyn DictionaryProvider>, AppError> {
    match kind {
        ProviderKind::Mock => Ok(Arc::new(MockProvider)),
        ProviderKind::HttpDictionary => Ok(Arc::new(HttpDictionaryProvider::new()?)),
        ProviderKind::OpenAi => {
            let file_config = load_file_config(config_path)?;
            let api_key = non_empty(api_key.to_string()).or(file_config.openai_api_key);
            let model = non_empty(model.to_string()).or(file_config.openai_model);
            Ok(Arc::new(OpenAiProvider::from_api_key(api_key, model)?))
        }
    }
}

fn load_file_config(path: &str) -> Result<FileConfig, AppError> {
    let path = path.trim();
    if path.is_empty() {
        return Ok(FileConfig::default());
    }

    let content = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

fn non_empty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn upsert_entry(entries: &mut Vec<WordEntry>, entry: WordEntry) {
    if let Some(existing) = entries
        .iter_mut()
        .find(|existing| existing.word.eq_ignore_ascii_case(&entry.word))
    {
        *existing = entry;
    } else {
        entries.push(entry);
    }
}

fn install_cjk_font(ctx: &egui::Context) {
    let Some((font_name, font_data)) = load_first_existing_font(cjk_font_candidates()) else {
        eprintln!("No CJK font found. Chinese text may render as square boxes.");
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        font_name.clone(),
        egui::FontData::from_owned(font_data).into(),
    );

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, font_name.clone());
    }

    ctx.set_fonts(fonts);
}

fn load_first_existing_font(paths: &[&str]) -> Option<(String, Vec<u8>)> {
    paths.iter().find_map(|path| {
        let path = Path::new(path);
        let bytes = std::fs::read(path).ok()?;
        let name = path.file_stem()?.to_string_lossy().to_string();
        Some((name, bytes))
    })
}

fn cjk_font_candidates() -> &'static [&'static str] {
    &[
        // Windows
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/msyh.ttf",
        "C:/Windows/Fonts/simhei.ttf",
        "C:/Windows/Fonts/simsun.ttc",
        // macOS
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/System/Library/Fonts/STHeiti Medium.ttc",
        // Linux
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJKsc-Regular.otf",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    ]
}
