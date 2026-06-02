use crate::{
    anki::{export_anki, AnkiExportFormat, AnkiExportOptions, AnkiExportTemplate, AnkiField},
    error::AppError,
    models::{parse_words, QueryStatus, WordEntry},
    providers::{
        ecdict::EcdictProvider, http_dictionary::HttpDictionaryProvider, mock::MockProvider,
        openai::OpenAiProvider, DictionaryProvider, ProviderKind,
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
    ui_language: UiLanguage,
    source_language: DictionaryLanguage,
    target_language: DictionaryLanguage,
    provider_kind: ProviderKind,
    api_key: String,
    ai_model: String,
    ai_api_base_url: String,
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
    show_startup_warning: bool,
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
    ai_status: Option<String>,
    ai_reason: Option<String>,
    query_type: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiLanguage {
    ZhHans,
    En,
}

impl UiLanguage {
    const ALL: [Self; 2] = [Self::ZhHans, Self::En];

    fn label(self) -> &'static str {
        match self {
            Self::ZhHans => "简体中文",
            Self::En => "English",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DictionaryLanguage {
    English,
    ChineseSimplified,
    ChineseTraditional,
    Japanese,
    Korean,
    French,
    German,
    Spanish,
}

impl DictionaryLanguage {
    const ALL: [Self; 8] = [
        Self::English,
        Self::ChineseSimplified,
        Self::ChineseTraditional,
        Self::Japanese,
        Self::Korean,
        Self::French,
        Self::German,
        Self::Spanish,
    ];

    fn label(self, ui_language: UiLanguage) -> &'static str {
        match ui_language {
            UiLanguage::ZhHans => match self {
                Self::English => "英语",
                Self::ChineseSimplified => "简体中文",
                Self::ChineseTraditional => "繁体中文",
                Self::Japanese => "日语",
                Self::Korean => "韩语",
                Self::French => "法语",
                Self::German => "德语",
                Self::Spanish => "西班牙语",
            },
            UiLanguage::En => match self {
                Self::English => "English",
                Self::ChineseSimplified => "Simplified Chinese",
                Self::ChineseTraditional => "Traditional Chinese",
                Self::Japanese => "Japanese",
                Self::Korean => "Korean",
                Self::French => "French",
                Self::German => "German",
                Self::Spanish => "Spanish",
            },
        }
    }

    fn prompt_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::ChineseSimplified => "Simplified Chinese",
            Self::ChineseTraditional => "Traditional Chinese",
            Self::Japanese => "Japanese",
            Self::Korean => "Korean",
            Self::French => "French",
            Self::German => "German",
            Self::Spanish => "Spanish",
        }
    }
}

enum QueryMessage {
    Started { total: usize },
    Entry(WordEntry),
    Skipped(String),
    Done,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    ai_api_key: Option<String>,
    ai_model: Option<String>,
    ai_api_base_url: Option<String>,
    source_language: Option<String>,
    target_language: Option<String>,
    openai_api_key: Option<String>,
    openai_model: Option<String>,
    openai_api_base_url: Option<String>,
}

impl English2AnkiApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_cjk_font(&cc.egui_ctx);

        Self {
            input_words: "apple, abandon, beautiful, network".to_string(),
            ui_language: UiLanguage::ZhHans,
            source_language: DictionaryLanguage::English,
            target_language: DictionaryLanguage::ChineseSimplified,
            provider_kind: ProviderKind::Ecdict,
            api_key: std::env::var("AI_API_KEY")
                .or_else(|_| std::env::var("OPENAI_API_KEY"))
                .unwrap_or_default(),
            ai_model: "gpt-4o-mini".to_string(),
            ai_api_base_url: std::env::var("AI_API_BASE_URL")
                .or_else(|_| std::env::var("OPENAI_API_BASE_URL"))
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string()),
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
            show_startup_warning: true,
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
                                ai_status: entry.ai_status.clone(),
                                ai_reason: entry.ai_reason.clone(),
                                query_type: entry.query_type.clone(),
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
                                ai_status: None,
                                ai_reason: None,
                                query_type: None,
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
            self.last_error = Some(tr(self.ui_language, "input_empty").to_string());
            return;
        }

        let provider = match build_provider(
            self.provider_kind,
            &self.api_key,
            &self.ai_model,
            &self.ai_api_base_url,
            &self.config_path,
            self.source_language,
            self.target_language,
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
            Ok(()) => {
                self.last_error = Some(format!(
                    "{}: {}",
                    tr(self.ui_language, "saved_json"),
                    path.display()
                ))
            }
            Err(error) => self.last_error = Some(error.to_string()),
        }
    }

    fn load_json(&mut self) {
        let path = PathBuf::from(self.load_path.trim());
        match storage::load_entries(&path) {
            Ok(entries) => {
                self.entries = entries;
                self.last_error = Some(format!(
                    "{}: {}",
                    tr(self.ui_language, "loaded_json"),
                    path.display()
                ));
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
            Ok(()) => {
                self.last_error = Some(format!(
                    "{}: {}",
                    tr(self.ui_language, "exported_anki"),
                    path.display()
                ))
            }
            Err(error) => self.last_error = Some(error.to_string()),
        }
    }
}

impl eframe::App for English2AnkiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_query_messages(ctx);
        self.render_startup_warning(ctx);
        let ui_language = self.ui_language;

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("English2Anki");
                if self.is_querying {
                    ui.spinner();
                    ui.label(format!(
                        "{} {}/{}",
                        tr(ui_language, "querying"),
                        self.progress_done,
                        self.progress_total
                    ));
                }
            });
        });

        egui::SidePanel::left("settings")
            .resizable(true)
            .default_width(330.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading(tr(ui_language, "query"));

                    ui_language_picker(ui, &mut self.ui_language, ui_language);

                    ui.label(tr(ui_language, "word_list"));
                    ui.add(
                        egui::TextEdit::multiline(&mut self.input_words)
                            .desired_rows(5)
                            .desired_width(f32::INFINITY),
                    );

                    egui::ComboBox::from_label(tr(ui_language, "provider"))
                        .selected_text(provider_label(self.provider_kind, ui_language))
                        .show_ui(ui, |ui| {
                            for kind in ProviderKind::ALL {
                                ui.selectable_value(
                                    &mut self.provider_kind,
                                    kind,
                                    provider_label(kind, ui_language),
                                );
                            }
                        });

                    let language_controls_enabled = self.provider_kind == ProviderKind::OpenAi;
                    ui.add_enabled_ui(language_controls_enabled, |ui| {
                        egui::ComboBox::from_label(tr(ui_language, "source_language"))
                            .selected_text(self.source_language.label(ui_language))
                            .show_ui(ui, |ui| {
                                for language in DictionaryLanguage::ALL {
                                    ui.selectable_value(
                                        &mut self.source_language,
                                        language,
                                        language.label(ui_language),
                                    );
                                }
                            });

                        egui::ComboBox::from_label(tr(ui_language, "target_language"))
                            .selected_text(self.target_language.label(ui_language))
                            .show_ui(ui, |ui| {
                                for language in DictionaryLanguage::ALL {
                                    ui.selectable_value(
                                        &mut self.target_language,
                                        language,
                                        language.label(ui_language),
                                    );
                                }
                            });
                    });
                    if !language_controls_enabled {
                        ui.small(tr(ui_language, "language_openai_only"));
                    }

                    ui.label("AI API Key");
                    ui.add(egui::TextEdit::singleline(&mut self.api_key).password(true));
                    ui.label(tr(ui_language, "ai_model"));
                    ui.text_edit_singleline(&mut self.ai_model);
                    ui.label(tr(ui_language, "ai_api_base_url"));
                    ui.text_edit_singleline(&mut self.ai_api_base_url);
                    ui.label(tr(ui_language, "config_path"));
                    ui.text_edit_singleline(&mut self.config_path);
                    ui.checkbox(&mut self.force_refresh, tr(ui_language, "force_refresh"));
                    ui.add(
                        egui::Slider::new(&mut self.concurrency, 1..=12)
                            .text(tr(ui_language, "concurrency")),
                    );

                    if ui
                        .add_enabled(
                            !self.is_querying,
                            egui::Button::new(tr(ui_language, "start")),
                        )
                        .clicked()
                    {
                        self.start_query();
                    }

                    ui.separator();
                    ui.heading("JSON");
                    path_row(
                        ui,
                        tr(ui_language, "save_path"),
                        &mut self.save_path,
                        true,
                        tr(ui_language, "choose"),
                    );
                    if ui.button(tr(ui_language, "save_json")).clicked() {
                        self.save_json();
                    }
                    path_row(
                        ui,
                        tr(ui_language, "load_path"),
                        &mut self.load_path,
                        false,
                        tr(ui_language, "choose"),
                    );
                    if ui.button(tr(ui_language, "load_json")).clicked() {
                        self.load_json();
                    }

                    ui.separator();
                    render_anki_settings(ui, self, ui_language);

                    ui.separator();
                    ui.heading(tr(ui_language, "messages"));
                    if let Some(error) = &self.last_error {
                        ui.label(error);
                    } else {
                        ui.label(tr(ui_language, "no_error"));
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(tr(ui_language, "query_status"));
            let progress = if self.progress_total == 0 {
                0.0
            } else {
                self.progress_done as f32 / self.progress_total as f32
            };
            ui.add(egui::ProgressBar::new(progress).show_percentage());

            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("status_grid")
                    .striped(true)
                    .num_columns(6)
                    .show(ui, |ui| {
                        ui.strong(tr(ui_language, "word"));
                        ui.strong(tr(ui_language, "status"));
                        ui.strong(tr(ui_language, "ai_status"));
                        ui.strong(tr(ui_language, "query_type"));
                        ui.strong(tr(ui_language, "source"));
                        ui.strong(tr(ui_language, "reason_error"));
                        ui.end_row();

                        let mut rows = self.statuses.iter().collect::<Vec<_>>();
                        rows.sort_by(|(left, _), (right, _)| left.cmp(right));
                        for (word, line) in rows {
                            ui.label(word);
                            ui.label(status_label(&line.status, ui_language));
                            if let Some(ai_status) = &line.ai_status {
                                ui.colored_label(ai_status_color(ai_status), ai_status);
                            } else {
                                ui.label("-");
                            }
                            ui.label(line.query_type.as_deref().unwrap_or("-"));
                            ui.label(&line.source);
                            ui.label(
                                line.error
                                    .as_deref()
                                    .or(line.ai_reason.as_deref())
                                    .unwrap_or(""),
                            );
                            ui.end_row();
                        }
                    });

                ui.separator();
                ui.heading(format!(
                    "{}{}{}",
                    tr(ui_language, "result_preview_prefix"),
                    self.entries.len(),
                    tr(ui_language, "result_preview_suffix")
                ));
                for entry in &self.entries {
                    ui.collapsing(
                        format!(
                            "{} - {}",
                            entry.word,
                            status_label(&entry.status, ui_language)
                        ),
                        |ui| {
                            ui.label(format!(
                                "{}: {}",
                                tr(ui_language, "phonetic"),
                                entry.phonetic.as_deref().unwrap_or("")
                            ));
                            ui.label(format!("{}: {}", tr(ui_language, "source"), entry.source));
                            ui.label(format!(
                                "{}: {}",
                                tr(ui_language, "queried_at"),
                                entry.queried_at
                            ));
                            if let Some(ai_status) = &entry.ai_status {
                                ui.colored_label(
                                    ai_status_color(ai_status),
                                    format!("{}: {ai_status}", tr(ui_language, "ai_status")),
                                );
                            }
                            if let Some(query_type) = &entry.query_type {
                                ui.label(format!(
                                    "{}: {query_type}",
                                    tr(ui_language, "query_type")
                                ));
                            }
                            if let Some(ai_reason) = &entry.ai_reason {
                                ui.label(format!("{}: {ai_reason}", tr(ui_language, "ai_reason")));
                            }
                            if let Some(error) = &entry.error {
                                ui.colored_label(egui::Color32::RED, error);
                            }
                            for definition in &entry.definitions {
                                ui.label(format!(
                                    "{} {} {}",
                                    definition.part_of_speech.as_deref().unwrap_or(""),
                                    definition.target.as_deref().unwrap_or(""),
                                    definition.source.as_deref().unwrap_or("")
                                ));
                            }
                            for example in &entry.examples {
                                ui.label(format!(
                                    "{}: {}",
                                    tr(ui_language, "example"),
                                    example.source
                                ));
                                if let Some(target) = &example.target {
                                    ui.label(target);
                                }
                            }
                        },
                    );
                }
            });
        });
    }
}

impl English2AnkiApp {
    fn render_startup_warning(&mut self, ctx: &egui::Context) {
        if !self.show_startup_warning {
            return;
        }

        let ui_language = self.ui_language;
        egui::Window::new(tr(ui_language, "safety_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                ui.set_max_width(560.0);
                ui_language_picker(ui, &mut self.ui_language, ui_language);
                let ui_language = self.ui_language;
                ui.add_space(8.0);
                ui.heading(tr(ui_language, "safety_heading"));
                ui.separator();
                ui.label(tr(ui_language, "safety_body_1"));
                ui.add_space(8.0);
                ui.label(tr(ui_language, "safety_body_2"));
                ui.add_space(8.0);
                ui.label(tr(ui_language, "safety_body_3"));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button(tr(ui_language, "understood")).clicked() {
                        self.show_startup_warning = false;
                    }
                });
            });
    }
}

fn render_anki_settings(ui: &mut egui::Ui, app: &mut English2AnkiApp, ui_language: UiLanguage) {
    ui.heading(tr(ui_language, "anki_export"));
    egui::ComboBox::from_label(tr(ui_language, "format"))
        .selected_text(app.export_format.label())
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut app.export_format, AnkiExportFormat::Tsv, "TSV");
            ui.selectable_value(&mut app.export_format, AnkiExportFormat::Csv, "CSV");
        });
    egui::ComboBox::from_label(tr(ui_language, "template"))
        .selected_text(template_label(app.export_template, ui_language))
        .show_ui(ui, |ui| {
            for template in AnkiExportTemplate::ALL {
                ui.selectable_value(
                    &mut app.export_template,
                    template,
                    template_label(template, ui_language),
                );
            }
        });

    ui.checkbox(
        &mut app.export_options.include_phonetic,
        tr(ui_language, "include_phonetic"),
    );
    ui.checkbox(
        &mut app.export_options.include_examples,
        tr(ui_language, "include_examples"),
    );
    ui.checkbox(
        &mut app.export_options.include_english_definition,
        tr(ui_language, "include_source_definition"),
    );
    ui.checkbox(
        &mut app.export_options.include_chinese_definition,
        tr(ui_language, "include_target_definition"),
    );
    ui.checkbox(
        &mut app.export_options.include_source,
        tr(ui_language, "include_source"),
    );
    ui.checkbox(
        &mut app.export_options.include_part_of_speech_hint,
        tr(ui_language, "include_pos_hint"),
    );

    ui.collapsing(tr(ui_language, "basic_front_fields"), |ui| {
        field_picker(ui, &mut app.export_options.front_fields, ui_language);
    });
    ui.collapsing(tr(ui_language, "basic_back_fields"), |ui| {
        field_picker(ui, &mut app.export_options.back_fields, ui_language);
    });

    path_row(
        ui,
        tr(ui_language, "export_path"),
        &mut app.export_path,
        true,
        tr(ui_language, "choose"),
    );
    if ui.button(tr(ui_language, "export_anki")).clicked() {
        app.export_anki();
    }
}

fn ai_status_color(status: &str) -> egui::Color32 {
    match status.to_ascii_lowercase().as_str() {
        "success" => egui::Color32::from_rgb(40, 150, 80),
        "invalid" | "failed" | "failure" => egui::Color32::RED,
        _ => egui::Color32::from_rgb(180, 130, 20),
    }
}

fn ui_language_picker(
    ui: &mut egui::Ui,
    selected_language: &mut UiLanguage,
    ui_language: UiLanguage,
) {
    egui::ComboBox::from_label(tr(ui_language, "ui_language"))
        .selected_text(selected_language.label())
        .show_ui(ui, |ui| {
            for language in UiLanguage::ALL {
                ui.selectable_value(selected_language, language, language.label());
            }
        });
}

fn tr(language: UiLanguage, key: &str) -> &'static str {
    match language {
        UiLanguage::ZhHans => match key {
            "querying" => "查询中",
            "input_empty" => "请输入至少一个词条",
            "saved_json" => "已保存 JSON",
            "loaded_json" => "已加载 JSON",
            "exported_anki" => "已导出 Anki 文件",
            "safety_title" => "安全提醒",
            "safety_heading" => "请在使用前仔细阅读",
            "safety_body_1" => "本项目代码由 AI 辅助生成。使用、修改、发布或集成前，请务必进行人工审查，重点检查代码逻辑、网络请求、本地文件读写、依赖来源和许可证兼容性，并自行承担相应风险。",
            "safety_body_2" => "在运行本项目的可执行文件前，建议先使用 VirusTotal 等工具扫描，并在沙盒环境中进行检查。本项目不能保证构建环境绝对干净安全；如果您对此有顾虑，请不要使用本软件。",
            "safety_body_3" => "如果您发现任何代码安全问题、版权问题或许可证问题，请在 GitHub Issues 中提出。感谢您的理解。",
            "understood" => "我已了解",
            "query" => "查询",
            "ui_language" => "界面语言",
            "word_list" => "逗号分隔的词条列表",
            "provider" => "词典源",
            "source_language" => "词典源语言",
            "target_language" => "词典目标语言",
            "language_openai_only" => "源语言和目标语言仅适用于 AI API 词典源。",
            "ai_model" => "AI 模型",
            "ai_api_base_url" => "AI API Base URL（OpenAI 兼容）",
            "config_path" => "配置文件路径（可选）",
            "force_refresh" => "强制刷新已有词条",
            "concurrency" => "并发数",
            "start" => "开始查询",
            "save_path" => "保存路径",
            "load_path" => "加载路径",
            "choose" => "选择",
            "save_json" => "保存 JSON",
            "load_json" => "加载 JSON",
            "messages" => "消息",
            "no_error" => "无错误",
            "query_status" => "查询状态",
            "word" => "词条",
            "status" => "状态",
            "ai_status" => "AI 返回",
            "query_type" => "查询类型",
            "source" => "来源",
            "reason_error" => "原因/错误",
            "result_preview_prefix" => "结果预览（",
            "result_preview_suffix" => " 条）",
            "phonetic" => "音标",
            "queried_at" => "查询时间",
            "ai_reason" => "AI 原因",
            "example" => "例句",
            "anki_export" => "Anki 导出",
            "format" => "格式",
            "template" => "模板",
            "include_phonetic" => "包含音标",
            "include_examples" => "包含例句",
            "include_source_definition" => "包含源语言释义",
            "include_target_definition" => "包含目标语言释义",
            "include_source" => "包含来源",
            "include_pos_hint" => "反向卡包含词性",
            "basic_front_fields" => "Basic 正面字段",
            "basic_back_fields" => "Basic 背面字段",
            "export_path" => "导出路径",
            "export_anki" => "导出 Anki 文件",
            _ => "",
        },
        UiLanguage::En => match key {
            "querying" => "Querying",
            "input_empty" => "Please enter at least one entry",
            "saved_json" => "Saved JSON",
            "loaded_json" => "Loaded JSON",
            "exported_anki" => "Exported Anki file",
            "safety_title" => "Safety Notice",
            "safety_heading" => "Please read before use",
            "safety_body_1" => "This project was built with AI assistance. Before using, modifying, publishing, or integrating it, manually review the code logic, network requests, local file access, dependency sources, and license compatibility. You are responsible for your own risk.",
            "safety_body_2" => "Before running this project's executable, consider scanning it with tools such as VirusTotal and checking it in a sandbox. This project cannot guarantee that every build environment is completely clean or safe; if that concerns you, do not use this software.",
            "safety_body_3" => "If you find any code safety, copyright, or license issue, please report it in GitHub Issues. Thank you for understanding.",
            "understood" => "Understood",
            "query" => "Query",
            "ui_language" => "Interface Language",
            "word_list" => "Comma-separated entry list",
            "provider" => "Dictionary Source",
            "source_language" => "Source Language",
            "target_language" => "Target Language",
            "language_openai_only" => {
                "Source and target languages only apply to the AI API dictionary source."
            }
            "ai_model" => "AI Model",
            "ai_api_base_url" => "AI API Base URL (OpenAI-compatible)",
            "config_path" => "Config File Path (optional)",
            "force_refresh" => "Refresh existing entries",
            "concurrency" => "Concurrency",
            "start" => "Start Query",
            "save_path" => "Save Path",
            "load_path" => "Load Path",
            "choose" => "Choose",
            "save_json" => "Save JSON",
            "load_json" => "Load JSON",
            "messages" => "Messages",
            "no_error" => "No errors",
            "query_status" => "Query Status",
            "word" => "Entry",
            "status" => "Status",
            "ai_status" => "AI Status",
            "query_type" => "Query Type",
            "source" => "Source",
            "reason_error" => "Reason/Error",
            "result_preview_prefix" => "Result Preview (",
            "result_preview_suffix" => " entries)",
            "phonetic" => "Phonetic",
            "queried_at" => "Queried At",
            "ai_reason" => "AI Reason",
            "example" => "Example",
            "anki_export" => "Anki Export",
            "format" => "Format",
            "template" => "Template",
            "include_phonetic" => "Include phonetic",
            "include_examples" => "Include examples",
            "include_source_definition" => "Include source-language definitions",
            "include_target_definition" => "Include target-language definitions",
            "include_source" => "Include source",
            "include_pos_hint" => "Include part of speech on reverse cards",
            "basic_front_fields" => "Basic Front Fields",
            "basic_back_fields" => "Basic Back Fields",
            "export_path" => "Export Path",
            "export_anki" => "Export Anki File",
            _ => "",
        },
    }
}

fn provider_label(kind: ProviderKind, language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::ZhHans => match kind {
            ProviderKind::Ecdict => "ECDICT 本地词典",
            ProviderKind::Mock => "Mock Provider",
            ProviderKind::HttpDictionary => "Free Dictionary API",
            ProviderKind::OpenAi => "AI API（OpenAI 兼容）",
        },
        UiLanguage::En => match kind {
            ProviderKind::Ecdict => "ECDICT Local Dictionary",
            ProviderKind::Mock => "Mock Provider",
            ProviderKind::HttpDictionary => "Free Dictionary API",
            ProviderKind::OpenAi => "AI API (OpenAI-compatible)",
        },
    }
}

fn status_label(status: &QueryStatus, language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::ZhHans => match status {
            QueryStatus::Success => "成功",
            QueryStatus::Failed => "失败",
            QueryStatus::PartialSuccess => "部分成功",
            QueryStatus::Skipped => "已跳过",
        },
        UiLanguage::En => match status {
            QueryStatus::Success => "Success",
            QueryStatus::Failed => "Failed",
            QueryStatus::PartialSuccess => "Partial Success",
            QueryStatus::Skipped => "Skipped",
        },
    }
}

fn template_label(template: AnkiExportTemplate, language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::ZhHans => match template {
            AnkiExportTemplate::Detailed => "词条详情模板",
            AnkiExportTemplate::BasicFrontBack => "Basic 正反面卡",
            AnkiExportTemplate::ChineseToEnglish => "目标语言到源语言反向卡",
            AnkiExportTemplate::TypeAnswer => "输入答案卡",
        },
        UiLanguage::En => match template {
            AnkiExportTemplate::Detailed => "Detailed Entry Template",
            AnkiExportTemplate::BasicFrontBack => "Basic Front/Back Card",
            AnkiExportTemplate::ChineseToEnglish => "Target-to-Source Reverse Card",
            AnkiExportTemplate::TypeAnswer => "Type Answer Card",
        },
    }
}

fn anki_field_label(field: AnkiField, language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::ZhHans => match field {
            AnkiField::Word => "词条",
            AnkiField::Phonetic => "音标",
            AnkiField::PartOfSpeech => "词性",
            AnkiField::EnglishDefinition => "源语言释义",
            AnkiField::ChineseDefinition => "目标语言释义",
            AnkiField::Examples => "例句",
            AnkiField::ExampleTranslations => "例句翻译",
            AnkiField::Source => "来源",
        },
        UiLanguage::En => match field {
            AnkiField::Word => "Entry",
            AnkiField::Phonetic => "Phonetic",
            AnkiField::PartOfSpeech => "Part of Speech",
            AnkiField::EnglishDefinition => "Source Definition",
            AnkiField::ChineseDefinition => "Target Definition",
            AnkiField::Examples => "Examples",
            AnkiField::ExampleTranslations => "Example Translations",
            AnkiField::Source => "Source",
        },
    }
}

fn field_picker(ui: &mut egui::Ui, fields: &mut Vec<AnkiField>, ui_language: UiLanguage) {
    for field in AnkiField::ALL {
        let mut enabled = fields.contains(&field);
        if ui
            .checkbox(&mut enabled, anki_field_label(field, ui_language))
            .changed()
        {
            if enabled {
                fields.push(field);
            } else {
                fields.retain(|item| *item != field);
            }
        }
    }
}

fn path_row(ui: &mut egui::Ui, label: &str, path: &mut String, save: bool, choose_label: &str) {
    ui.label(label);
    ui.horizontal(|ui| {
        ui.text_edit_singleline(path);
        if ui.button(choose_label).clicked() {
            let dialog = file_dialog_for_path(path);
            let selected = if save {
                dialog.save_file()
            } else {
                dialog.pick_file()
            };
            if let Some(selected) = selected {
                *path = selected.display().to_string();
            }
        }
    });
}

fn file_dialog_for_path(path: &str) -> rfd::FileDialog {
    let mut dialog = rfd::FileDialog::new()
        .add_filter("JSON (*.json)", &["json"])
        .add_filter("Anki TSV (*.tsv)", &["tsv"])
        .add_filter("CSV (*.csv)", &["csv"])
        .add_filter("All files (*.*)", &["*"]);

    let path = Path::new(path.trim());
    if let Some(file_name) = path.file_name().and_then(|file_name| file_name.to_str()) {
        if !file_name.is_empty() {
            dialog = dialog.set_file_name(file_name);
        }
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        dialog = dialog.set_directory(parent);
    }

    dialog
}

fn build_provider(
    kind: ProviderKind,
    api_key: &str,
    model: &str,
    api_base_url: &str,
    config_path: &str,
    source_language: DictionaryLanguage,
    target_language: DictionaryLanguage,
) -> Result<Arc<dyn DictionaryProvider>, AppError> {
    match kind {
        ProviderKind::Ecdict => Ok(Arc::new(EcdictProvider::new())),
        ProviderKind::Mock => Ok(Arc::new(MockProvider::new()?)),
        ProviderKind::HttpDictionary => Ok(Arc::new(HttpDictionaryProvider::new()?)),
        ProviderKind::OpenAi => {
            let file_config = load_file_config(config_path)?;
            let api_key = non_empty(api_key.to_string())
                .or(file_config.ai_api_key)
                .or(file_config.openai_api_key);
            let model = non_empty(model.to_string())
                .or(file_config.ai_model)
                .or(file_config.openai_model);
            let api_base_url = non_empty(api_base_url.to_string())
                .or(file_config.ai_api_base_url)
                .or(file_config.openai_api_base_url);
            let source_language = file_config
                .source_language
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(source_language.prompt_name())
                .to_string();
            let target_language = file_config
                .target_language
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or(target_language.prompt_name())
                .to_string();
            Ok(Arc::new(OpenAiProvider::from_config(
                api_key,
                model,
                api_base_url,
                source_language,
                target_language,
            )?))
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
    let mut fonts = egui::FontDefinitions::default();
    let mut loaded_font_names = Vec::new();

    for (font_name, font_data) in load_existing_fonts(phonetic_font_candidates())
        .into_iter()
        .chain(load_existing_fonts(cjk_font_candidates()))
    {
        fonts.font_data.insert(
            font_name.clone(),
            egui::FontData::from_owned(font_data).into(),
        );
        loaded_font_names.push(font_name);
    }

    if loaded_font_names.is_empty() {
        eprintln!("No app fallback font found. Some text may render as square boxes.");
        return;
    }

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        let family_fonts = fonts.families.entry(family).or_default();
        for font_name in loaded_font_names.iter().rev() {
            family_fonts.insert(0, font_name.clone());
        }
    }

    ctx.set_fonts(fonts);
}

fn load_existing_fonts(paths: &[&str]) -> Vec<(String, Vec<u8>)> {
    paths
        .iter()
        .filter_map(|path| {
            let path = Path::new(path);
            let bytes = std::fs::read(path).ok()?;
            let name = path.file_stem()?.to_string_lossy().to_string();
            Some((name, bytes))
        })
        .collect()
}

fn phonetic_font_candidates() -> &'static [&'static str] {
    &[
        // Windows
        "C:/Windows/Fonts/segoeui.ttf",
        "C:/Windows/Fonts/arial.ttf",
        "C:/Windows/Fonts/calibri.ttf",
        "C:/Windows/Fonts/times.ttf",
        // macOS
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/System/Library/Fonts/Supplemental/Times New Roman.ttf",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        // Linux
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
    ]
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
