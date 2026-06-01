# English2Anki

一个 Rust 桌面程序，用于批量查询英文单词释义，保存为本地 JSON，并导出为 Anki 可导入的 UTF-8 TSV/CSV。

## 运行

```powershell
cargo run
```

默认选择 `ECDICT 本地词库`。第一次查询时程序会下载 ECDICT CSV 到本地缓存目录，之后会直接使用本地词库查询。

## GitHub Actions 发布

项目包含 `.github/workflows/release.yml`，推送 `v*` 标签时会自动构建并发布 GitHub Release：

```powershell
git tag v0.1.0
git push origin v0.1.0
```

Actions 会构建 Windows、macOS 和 Linux 版本，并把压缩包上传到 Release。也可以在 GitHub Actions 页面手动运行 `Build and Release`，输入 `release_tag`，例如 `v0.1.0`。

## 数据源

项目使用统一 trait 扩展词典来源：

```rust
#[async_trait]
pub trait DictionaryProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn lookup(&self, word: &str) -> Result<WordEntry, AppError>;
}
```

已实现 Provider：

- `EcdictProvider`：运行时下载 ECDICT CSV，本地缓存并查询，提供英文释义和中文释义。
- `MockProvider`：离线可用，生成示例释义和例句。
- `HttpDictionaryProvider`：调用 `https://api.dictionaryapi.dev`，可获取英文释义和例句，通常没有中文释义。
- `OpenAiProvider`：调用 OpenAI 兼容的 Chat Completions API，生成结构化双语释义、音标和例句。可用于 OpenAI，也可配置为其他兼容服务。

新增数据源时，在 `src/providers/` 新增模块并实现 `DictionaryProvider`，再在 `ProviderKind` 和 `build_provider` 中注册即可。

### ECDICT 本地词库

默认下载地址：

```text
https://raw.githubusercontent.com/skywind3000/ECDICT/master/ecdict.csv
```

默认缓存位置为系统用户缓存目录下的 `English2Anki/dictionaries/ecdict.csv`。可以用环境变量覆盖：

```powershell
$env:ECDICT_CSV_URL="https://example.com/ecdict.csv"
$env:ECDICT_CACHE_PATH="D:\Dictionaries\ecdict.csv"
cargo run
```

ECDICT 文件不会被嵌入程序二进制，Release 包里也不会包含词典数据。

## AI API 配置

不要把 API Key 写入源码。可以使用任意一种方式：

```powershell
$env:AI_API_KEY="sk-..."
$env:AI_API_BASE_URL="https://api.openai.com/v1"
cargo run
```

也可以在 GUI 的 `AI API Key`、`AI 模型` 和 `AI API Base URL` 输入框中填写。默认模型为 `gpt-4o-mini`，默认 Base URL 为 `https://api.openai.com/v1`。

还可以复制 `config.example.json`，填入：

```json
{
  "ai_api_key": "sk-...",
  "ai_model": "gpt-4o-mini",
  "ai_api_base_url": "https://api.openai.com/v1"
}
```

然后在 GUI 的 `配置文件路径` 中选择该文件。读取优先级为：界面输入、配置文件、环境变量。旧字段 `openai_api_key`、`openai_model`、`openai_api_base_url` 仍然兼容。

如果使用第三方 OpenAI 兼容服务，把 `ai_api_base_url` 改成该服务的 `/v1` 地址即可。例如：

```json
{
  "ai_api_key": "your-key",
  "ai_model": "deepseek-chat",
  "ai_api_base_url": "https://api.deepseek.com/v1"
}
```

## JSON 存储

- `保存 JSON` 会使用格式化 JSON 保存当前查询结果。
- `加载 JSON` 可以读取已有数据。
- 查询时默认跳过 JSON 中已存在的单词。
- 勾选 `强制刷新已有单词` 后会重新查询并覆盖同名单词。
- 单个单词失败不会中断批量任务，失败原因会写入对应 `WordEntry.error`。

## Anki 导出

支持 TSV 和 CSV：

- TSV 会将字段用 Tab 分隔，并把字段内的 Tab 替换为空格。
- CSV 使用 `csv` crate 正确处理逗号、双引号和换行。
- 字段内换行会替换为 `<br>`。
- 文件均以 UTF-8 写出。

支持模板：

- 词条详情模板：`单词 / 音标 / 词性和释义 / 例句 / 例句翻译 / 来源`
- Basic 正反面卡：`Front / Back`，可在 GUI 勾选正面和背面字段
- 中文到英文反向卡：`Front / Back`
- 输入答案卡：`Prompt / Answer / Extra`

输入答案卡可在 Anki 中使用类似模板：

```html
{{Prompt}}
<br>
{{type:Answer}}
```

背面模板：

```html
{{FrontSide}}
<hr>
{{Extra}}
```

## 推荐项目结构

```text
src/
  main.rs
  app.rs
  models.rs
  storage.rs
  anki.rs
  error.rs
  providers/
    mod.rs
    mock.rs
    openai.rs
    http_dictionary.rs
```
