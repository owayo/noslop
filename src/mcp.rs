//! MCP (Model Context Protocol) サーバー (`noslop mcp`)。
//!
//! 標準入出力で、1 行に 1 つの JSON-RPC 2.0 メッセージをやり取りする。AI エージェントが
//! 書いた文章をその場で検査して、改稿指示 (brief) を受け取れるようにする。
//!
//! 版の扱い (両方の方式に答える):
//!
//! - 2026-07-28 以降: 版・クライアントの能力をリクエストごとの `_meta` で名乗る方式。
//!   `server/discover` に答え、`_meta` の版が対応外なら -32022 を返す
//! - 2025-11-25 以前: `initialize` で版を決める方式。要求された版に対応していれば同じ版を、
//!   そうでなければ対応する最新の版を返す
//!
//! ツールは `check` (本文の検査)・`diff` (改稿の前後の比較)・`explain` (ルールの説明)・
//! `rules` (ルールの一覧)。
//! 引数の誤りや設定の誤りは、落ちずにツールの実行エラー (`isError: true`) として返す。

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::io::{self, BufRead, Write};
use std::path::Path;

use serde_json::{Map, Value, json};

use crate::document::SourceFormat;
use crate::engine::{Engine, EngineOptions, RunReport};
use crate::genre::Genre;
use crate::output::{self, RenderOptions, RuleCatalog};

/// 対応する版 (新しい順)。
pub const SUPPORTED_VERSIONS: [&str; 5] = [
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];

/// リクエストごとに `_meta` で版を名乗る方式の版。
const PER_REQUEST_VERSIONS: [&str; 1] = ["2026-07-28"];

/// `initialize` で決める方式のうち、最も新しい版。
const LATEST_HANDSHAKE_VERSION: &str = "2025-11-25";

const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

/// `check` で受け付ける本文の上限 (バイト)。
const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

/// 1 メッセージ (1 行) の上限 (バイト)。本文の上限に JSON のエスケープのぶんを見込む。
const MAX_LINE_BYTES: usize = 2 * MAX_TEXT_BYTES;

/// サーバーの使い方の説明 (クライアントが LLM に渡す)。
const INSTRUCTIONS: &str = "noslop は日本語の文章から「AI 臭さ」(LLM の文章に特有の言い回し・単調なリズム・体裁) を機械的に検出します。文章を書いたり直したりした後に `check` を呼び、返ってきた改稿指示の「改稿のルール」に従って見直してください。指摘は疑いの提示です。文脈上必要なら直さずに残してかまいません。原文にない事実・数字・体験は足さず、足りない材料は書き手に確認してください。指摘の件数を減らすこと自体を目的にせず、検査のやり直しは 1 回までにしてください。直した後は `diff` に直す前と後の本文を渡すと、新しく出た指摘と、改稿で消えた数字や固有名詞をまとめて確かめられます。ルールの詳しい説明は `explain` で引けます。`check` の改稿指示は、`format` に `toon` を渡すと同じ内容を少ないトークンで、`json` を渡すとプログラムで扱える形で受け取れます。";

/// MCP サーバーの状態。
pub struct Server {
    /// 設定ファイルから組み立てたエンジンの設定。設定を読めなかったときはその理由。
    base: Result<EngineOptions, String>,
    /// ジャンルと実験的ルールの有無ごとのエンジン。独自ルールの組み立ては確保したメモリを
    /// 解放しないので、呼ばれるたびに作り直さずに使い回す。
    engines: HashMap<(Genre, bool), (Engine, RuleCatalog)>,
    /// `initialize` で決めた版 (2025-11-25 以前の方式)。`_meta` で版を名乗らないリクエストは、
    /// これが決まってからでないと受け付けない。
    handshake: Option<&'static str>,
}

/// リクエストの版の方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Era {
    /// `_meta` で版を名乗る方式 (2026-07-28 以降)。
    PerRequest,
    /// `initialize` で版を決める方式 (2025-11-25 以前)。
    Handshake,
}

impl Server {
    pub fn new(base: Result<EngineOptions, String>) -> Self {
        Self {
            base,
            engines: HashMap::new(),
            handshake: None,
        }
    }

    /// 1 行 (1 メッセージ) を処理する。応答しないメッセージ (通知・応答) なら `None`。
    pub fn handle_line(&mut self, line: &str) -> Option<Value> {
        match serde_json::from_str::<Value>(line) {
            Ok(message) => self.handle(message),
            Err(e) => Some(error_response(
                Value::Null,
                PARSE_ERROR,
                "Parse error",
                Some(json!({ "detail": e.to_string() })),
            )),
        }
    }

    /// 1 メッセージを処理する。
    pub fn handle(&mut self, message: Value) -> Option<Value> {
        // バッチ (配列) は 2025-06-18 以降の版にないので受け付けない
        let Value::Object(message) = message else {
            return Some(invalid_request(
                Value::Null,
                "メッセージは JSON オブジェクト 1 つにしてください",
            ));
        };
        let id = message.get("id").cloned();
        let reply_id = id.clone().filter(valid_id).unwrap_or(Value::Null);
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(invalid_request(
                reply_id,
                "`jsonrpc` は \"2.0\" にしてください",
            ));
        }
        let Some(method) = message.get("method") else {
            // クライアントからの応答 (このサーバーはリクエストを送らないので読み捨てる)
            if message.contains_key("result") || message.contains_key("error") {
                return None;
            }
            return Some(invalid_request(reply_id, "`method` がありません"));
        };
        let Some(method) = method.as_str() else {
            return Some(invalid_request(reply_id, "`method` は文字列にしてください"));
        };
        let Some(id) = id else {
            // 通知 (`notifications/initialized` など) には応答しない
            return None;
        };
        if !valid_id(&id) {
            return Some(invalid_request(
                Value::Null,
                "`id` は文字列か整数にしてください",
            ));
        }
        let params = match message.get("params") {
            None => Map::new(),
            Some(Value::Object(p)) => p.clone(),
            Some(_) => {
                return Some(error_response(
                    id,
                    INVALID_PARAMS,
                    "Invalid params: `params` はオブジェクトにしてください",
                    None,
                ));
            }
        };
        let era = match request_era(&params) {
            Ok(era) => era,
            Err((code, message, data)) => return Some(error_response(id, code, &message, data)),
        };
        // `_meta` で版を名乗らないリクエストは、`initialize` の後 (と ping) だけ受け付ける。
        // 版の決まっていないリクエストを黙って処理すると、クライアントが方式を判別できない
        if era == Era::Handshake
            && self.handshake.is_none()
            && !matches!(method, "initialize" | "ping")
        {
            return Some(error_response(
                id,
                INVALID_PARAMS,
                &format!(
                    "Invalid params: `_meta` に `{META_PROTOCOL_VERSION}` がありません (2025-11-25 以前の版では、先に initialize を送ってください)"
                ),
                None,
            ));
        }
        let result = match method {
            "initialize" => {
                let (version, result) = initialize(&params);
                self.handshake = Some(version);
                Ok(result)
            }
            "server/discover" => Ok(discover()),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => self.call_tool(&params),
            _ => Err((
                METHOD_NOT_FOUND,
                format!("Method not found: {method}"),
                None,
            )),
        };
        Some(match result {
            Ok(mut result) => {
                // 結果の種類とサーバー名は、`_meta` で版を名乗る方式の結果にだけ付ける
                if era == Era::PerRequest
                    && let Value::Object(obj) = &mut result
                {
                    obj.insert("resultType".to_string(), json!("complete"));
                    let meta = obj
                        .entry("_meta")
                        .or_insert_with(|| Value::Object(Map::new()));
                    if let Value::Object(meta) = meta {
                        meta.insert(META_SERVER_INFO.to_string(), server_info());
                    }
                }
                json!({ "jsonrpc": "2.0", "id": id, "result": result })
            }
            Err((code, message, data)) => error_response(id, code, &message, data),
        })
    }

    fn call_tool(&mut self, params: &Map<String, Value>) -> Result<Value, RpcError> {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return Err((
                INVALID_PARAMS,
                "Invalid params: `name` (ツール名) を文字列で指定してください".to_string(),
                None,
            ));
        };
        let empty = Map::new();
        let args = match params.get("arguments") {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(a)) => a,
            Some(_) => {
                return Err((
                    INVALID_PARAMS,
                    "Invalid params: `arguments` はオブジェクトにしてください".to_string(),
                    None,
                ));
            }
        };
        let outcome = match name {
            "check" => self.check(args),
            "diff" => self.diff(args),
            "explain" => self.explain(args),
            "rules" => self.rules(args),
            _ => return Err((INVALID_PARAMS, format!("Unknown tool: {name}"), None)),
        };
        let (text, is_error) = match outcome {
            Ok(text) => (text, false),
            Err(message) => (message, true),
        };
        Ok(json!({
            "content": [{ "type": "text", "text": text }],
            "isError": is_error,
        }))
    }

    /// ジャンルと実験的ルールの有無に合うエンジン (なければ作る)。
    fn engine(
        &mut self,
        genre: Option<Genre>,
        experimental: bool,
    ) -> Result<&(Engine, RuleCatalog), String> {
        let base = self
            .base
            .as_ref()
            .map_err(|e| format!("noslop の設定を読み込めません: {e}"))?;
        let genre = genre.unwrap_or(base.genre);
        let experimental = experimental || base.experimental;
        match self.engines.entry((genre, experimental)) {
            Entry::Occupied(entry) => Ok(entry.into_mut()),
            Entry::Vacant(entry) => {
                let mut options = base.clone();
                options.genre = genre;
                options.experimental = experimental;
                let engine = Engine::new(options)
                    .map_err(|e| format!("noslop の設定を読み込めません: {e}"))?;
                let catalog = RuleCatalog::from_engine(&engine);
                Ok(entry.insert((engine, catalog)))
            }
        }
    }

    /// `check`: 本文を検査して brief (既定) か JSON を返す。
    fn check(&mut self, args: &Map<String, Value>) -> Result<String, String> {
        let text = text_arg(args, "text", "検査する本文")?;
        let filename = string_arg(args, "filename")?;
        let genre = genre_arg(args)?;
        let experimental = bool_arg(args, "experimental")?.unwrap_or(false);
        let output = check_output(string_arg(args, "report")?, string_arg(args, "format")?)?;
        let (engine, catalog) = self.engine(genre, experimental)?;
        let (name, format) = match filename {
            Some(name) => (name.to_string(), SourceFormat::from_path(Path::new(name))),
            None => ("<text>".to_string(), SourceFormat::Markdown),
        };
        let report = RunReport {
            files: vec![engine.lint_source(name, text.clone(), format)],
            errors: Vec::new(),
            morphology: engine.morphology().clone(),
        };
        let opts = RenderOptions {
            genre: engine.options().genre,
            experimental: engine.options().experimental,
            catalog: catalog.clone(),
            ..Default::default()
        };
        let mut buf = Vec::new();
        let rendered = match output {
            CheckOutput::BriefMarkdown => output::brief::render(&report, &opts, &mut buf),
            CheckOutput::BriefJson => output::brief::render_json(&report, &opts, &mut buf),
            CheckOutput::BriefToon => output::brief::render_toon(&report, &opts, &mut buf),
            CheckOutput::FullJson => output::json::render(&report, &opts, &mut buf),
            CheckOutput::FullToon => output::json::render_toon(&report, &opts, &mut buf),
        };
        rendered.map_err(|e| format!("結果を書き出せません: {e}"))?;
        String::from_utf8(buf).map_err(|e| format!("結果を書き出せません: {e}"))
    }

    /// `diff`: 改稿の前後を同じエンジンで検査し、確認事項 (新しい指摘・事実の変化・改稿の偏り) を返す。
    fn diff(&mut self, args: &Map<String, Value>) -> Result<String, String> {
        let before = text_arg(args, "before", "改稿前の本文")?;
        let after = text_arg(args, "after", "改稿後の本文")?;
        let filename = string_arg(args, "filename")?;
        let genre = genre_arg(args)?;
        let experimental = bool_arg(args, "experimental")?.unwrap_or(false);
        let format = string_arg(args, "format")?.unwrap_or("text");
        if !matches!(format, "text" | "json" | "toon") {
            return Err(format!(
                "未知の形式です: {format} (text・json・toon のどれかを指定してください)"
            ));
        }
        let (engine, _) = self.engine(genre, experimental)?;
        let (name, source_format) = match filename {
            Some(name) => (name.to_string(), SourceFormat::from_path(Path::new(name))),
            None => ("<text>".to_string(), SourceFormat::Markdown),
        };
        let before = engine.lint_source(format!("{name} (改稿前)"), before.clone(), source_format);
        let after = engine.lint_source(format!("{name} (改稿後)"), after.clone(), source_format);
        let report = crate::diff::compare(before, after);
        let mut buf = Vec::new();
        match format {
            "json" => crate::diff::render_json(&report, &mut buf),
            "toon" => crate::diff::render_toon(&report, &mut buf),
            _ => crate::diff::render_text(&report, &mut buf),
        }
        .map_err(|e| format!("結果を書き出せません: {e}"))?;
        // text は端末向けの書式 (太字など) を含むので、エージェントに渡す前に外す
        let plain = if format == "text" {
            anstream::adapter::strip_bytes(&buf).into_vec()
        } else {
            buf
        };
        String::from_utf8(plain).map_err(|e| format!("結果を書き出せません: {e}"))
    }

    /// `explain`: ルールの詳細を返す。
    fn explain(&mut self, args: &Map<String, Value>) -> Result<String, String> {
        let Some(rule) = string_arg(args, "rule")? else {
            return Err("`rule` (ルールの ID か名前) を指定してください".to_string());
        };
        let genre = genre_arg(args)?;
        let (engine, _) = self.engine(genre, false)?;
        let entry = engine.find(rule).ok_or_else(|| {
            format!("未知のルールです: {rule} (`rules` ツールで一覧を確認できます)")
        })?;
        Ok(crate::cli::explain_text(engine, entry))
    }

    /// `rules`: ルールの一覧を返す。
    fn rules(&mut self, args: &Map<String, Value>) -> Result<String, String> {
        let genre = genre_arg(args)?;
        let experimental = bool_arg(args, "experimental")?.unwrap_or(false);
        let (engine, _) = self.engine(genre, experimental)?;
        Ok(crate::cli::rules_text(engine))
    }
}

/// JSON-RPC のエラー (コード・メッセージ・追加情報)。
type RpcError = (i64, String, Option<Value>);

/// 標準入出力のループ。入力が閉じたら終わる。
pub fn serve<R: BufRead, W: Write>(
    server: &mut Server,
    mut input: R,
    mut output: W,
) -> io::Result<()> {
    let mut buf = Vec::new();
    loop {
        let response = match read_line(&mut input, &mut buf, MAX_LINE_BYTES)? {
            Line::End => return Ok(()),
            Line::TooLong => Some(invalid_request(
                Value::Null,
                &format!(
                    "1 行 (1 メッセージ) が大きすぎます (上限 {} MiB)",
                    MAX_LINE_BYTES / 1024 / 1024
                ),
            )),
            Line::Complete => match std::str::from_utf8(&buf) {
                Ok(line) if line.trim().is_empty() => continue,
                Ok(line) => server.handle_line(line.trim()),
                Err(_) => Some(error_response(
                    Value::Null,
                    PARSE_ERROR,
                    "Parse error",
                    Some(json!({ "detail": "UTF-8 として読めません" })),
                )),
            },
        };
        if let Some(response) = response {
            // 文字列中の改行はエスケープされるので、1 メッセージは必ず 1 行になる
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
}

/// [`read_line`] の結果。
#[derive(Debug, PartialEq, Eq)]
enum Line {
    /// 1 行を読んだ (改行を含む)。
    Complete,
    /// 上限を超えたので、行の終わりまで読み捨てた。
    TooLong,
    /// 入力の終わり。
    End,
}

/// 1 行を `buf` に読む。上限を超える行は、メモリに溜めずに行の終わりまで読み捨てる。
fn read_line<R: BufRead>(input: &mut R, buf: &mut Vec<u8>, limit: usize) -> io::Result<Line> {
    buf.clear();
    let mut read_any = false;
    let mut too_long = false;
    loop {
        let available = match input.fill_buf() {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            return Ok(match (read_any, too_long) {
                (false, _) => Line::End,
                (true, false) => Line::Complete,
                (true, true) => Line::TooLong,
            });
        }
        read_any = true;
        let newline = available.iter().position(|&b| b == b'\n');
        let len = newline.map_or(available.len(), |i| i + 1);
        if !too_long {
            if buf.len() + len > limit {
                too_long = true;
                buf.clear();
            } else {
                buf.extend_from_slice(&available[..len]);
            }
        }
        input.consume(len);
        if newline.is_some() {
            return Ok(if too_long {
                Line::TooLong
            } else {
                Line::Complete
            });
        }
    }
}
/// リクエストの `_meta` から版の方式を決め、`_meta` で名乗る方式なら必要な項目を確かめる。
fn request_era(params: &Map<String, Value>) -> Result<Era, RpcError> {
    let invalid = |detail: String| (INVALID_PARAMS, format!("Invalid params: {detail}"), None);
    let meta = match params.get("_meta") {
        None => return Ok(Era::Handshake),
        Some(Value::Object(meta)) => meta,
        Some(_) => return Err(invalid("`_meta` はオブジェクトにしてください".to_string())),
    };
    let version = match meta.get(META_PROTOCOL_VERSION) {
        None => return Ok(Era::Handshake),
        Some(Value::String(v)) => v.as_str(),
        Some(_) => {
            return Err(invalid(format!(
                "`{META_PROTOCOL_VERSION}` は文字列にしてください"
            )));
        }
    };
    if !SUPPORTED_VERSIONS.contains(&version) {
        return Err((
            UNSUPPORTED_PROTOCOL_VERSION,
            "Unsupported protocol version".to_string(),
            Some(json!({ "supported": SUPPORTED_VERSIONS, "requested": version })),
        ));
    }
    if !PER_REQUEST_VERSIONS.contains(&version) {
        // 2025-11-25 以前の版は `_meta` で名乗っても `initialize` の方式で扱う
        return Ok(Era::Handshake);
    }
    if !meta
        .get(META_CLIENT_CAPABILITIES)
        .is_some_and(Value::is_object)
    {
        return Err(invalid(format!(
            "`_meta` の `{META_CLIENT_CAPABILITIES}` をオブジェクトで指定してください"
        )));
    }
    if meta.get(META_CLIENT_INFO).is_some_and(|v| !v.is_object()) {
        return Err(invalid(format!(
            "`_meta` の `{META_CLIENT_INFO}` はオブジェクトにしてください"
        )));
    }
    Ok(Era::PerRequest)
}

/// `initialize` の結果と、決めた版。
fn initialize(params: &Map<String, Value>) -> (&'static str, Value) {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = SUPPORTED_VERSIONS
        .iter()
        .copied()
        .filter(|v| !PER_REQUEST_VERSIONS.contains(v))
        .find(|v| Some(*v) == requested)
        .unwrap_or(LATEST_HANDSHAKE_VERSION);
    let result = json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": server_info(),
        "instructions": INSTRUCTIONS,
    });
    (version, result)
}

fn discover() -> Value {
    json!({
        "supportedVersions": SUPPORTED_VERSIONS,
        "capabilities": { "tools": {} },
        "instructions": INSTRUCTIONS,
        "_meta": { META_SERVER_INFO: server_info() },
    })
}

fn server_info() -> Value {
    json!({ "name": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION") })
}

/// 公開するツール。
fn tools() -> Value {
    let genre = json!({
        "type": "string",
        "enum": ["general", "tech", "business", "essay"],
        "description": "文書のジャンル。省略すると設定ファイルの値 (なければ general)"
    });
    let experimental = json!({
        "type": "boolean",
        "description": "実験的な (未校正の) ルールも動かす"
    });
    let read_only = json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false
    });
    json!([
        {
            "name": "check",
            "title": "AI 臭さの検査",
            "description": "日本語の文章 (Markdown かテキスト) から AI 臭さを検出し、直す箇所をルールごとにまとめた改稿指示を返します。指摘は疑いの提示です。語句を機械的に置き換えず、改稿指示の「改稿のルール」に従って、効果の大きい箇所だけを直してください。直した後の検査は 1 回までにしてください。改稿指示は、読むなら markdown、プログラムで扱うなら json、少ないトークンで受け取るなら toon を選べます。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "検査する本文" },
                    "filename": {
                        "type": "string",
                        "description": "表示名。拡張子 (.md / .txt) で形式を判断する (省略時は Markdown)"
                    },
                    "genre": genre,
                    "experimental": experimental,
                    "report": {
                        "type": "string",
                        "enum": ["brief", "full"],
                        "default": "brief",
                        "description": "brief は直す箇所をルールごとにまとめた改稿指示 (改稿のルール・なぜ疑わしいか・直し方の方向・該当箇所)。full は全指摘のレポート (位置・metrics・fingerprint を含む)"
                    },
                    "format": {
                        "type": "string",
                        "enum": ["markdown", "json", "toon"],
                        "description": "markdown (改稿指示の既定。report: brief のときだけ)・json (full の既定)・toon (JSON と同じ内容を少ないトークンで表す TOON)"
                    }
                },
                "required": ["text"]
            },
            "annotations": read_only
        },
        {
            "name": "diff",
            "title": "改稿の前後の比較",
            "description": "改稿の前後の本文を同じ設定で検査し、改稿で新しく出た指摘、消えた数字・日付・固有名詞らしい語・引用・URL、文書全体に一律に当てた直し (読点の一括削除など) を確認事項として返します。直した後の確認は、check をやり直す代わりにこれで 1 回だけ行ってください。確認事項は失敗の判定ではありません。消えた事実は、削ってよい情報か書き手に確かめてください。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "before": { "type": "string", "description": "改稿前の本文" },
                    "after": { "type": "string", "description": "改稿後の本文" },
                    "filename": {
                        "type": "string",
                        "description": "表示名。拡張子 (.md / .txt) で形式を判断する (省略時は Markdown)"
                    },
                    "genre": genre,
                    "experimental": experimental,
                    "format": {
                        "type": "string",
                        "enum": ["text", "json", "toon"],
                        "default": "text",
                        "description": "text は確認事項の一覧、json は機械可読な結果 (hasConcerns で確認事項の有無が分かる)、toon は JSON と同じ内容を少ないトークンで表す TOON"
                    }
                },
                "required": ["before", "after"]
            },
            "annotations": read_only
        },
        {
            "name": "explain",
            "title": "ルールの説明",
            "description": "ルールの詳細 (何を見るか・なぜ問題か・直し方・例・根拠) を返します。改稿指示に出たルール ID (P01 など) を渡してください。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "rule": {
                        "type": "string",
                        "description": "ルールの ID か名前 (例: P01、AI_CONCLUSION)"
                    },
                    "genre": genre
                },
                "required": ["rule"]
            },
            "annotations": read_only
        },
        {
            "name": "rules",
            "title": "ルールの一覧",
            "description": "noslop のルールの一覧 (ID・名前・レーン・状態・重大度・既定で有効か) を返します。",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "genre": genre,
                    "experimental": experimental
                }
            },
            "annotations": read_only
        }
    ])
}

fn valid_id(id: &Value) -> bool {
    id.is_string() || id.is_i64() || id.is_u64()
}

fn error_response(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

fn invalid_request(id: Value, detail: &str) -> Value {
    error_response(
        id,
        INVALID_REQUEST,
        "Invalid Request",
        Some(json!({ "detail": detail })),
    )
}

/// `check` が返すもの (内容と形式の組)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckOutput {
    BriefMarkdown,
    BriefJson,
    BriefToon,
    FullJson,
    FullToon,
}

/// `check` の `report` と `format` から返すものを決める。
///
/// エージェントに渡す既定は改稿指示 (`report: brief`) の Markdown。全指摘のレポート
/// (`report: full`) は JSON (既定) か TOON で返す。
fn check_output(report: Option<&str>, format: Option<&str>) -> Result<CheckOutput, String> {
    use CheckOutput as O;
    match (report.unwrap_or("brief"), format) {
        ("brief", None | Some("markdown")) => Ok(O::BriefMarkdown),
        ("brief", Some("json")) => Ok(O::BriefJson),
        ("brief", Some("toon")) => Ok(O::BriefToon),
        ("full", None | Some("json")) => Ok(O::FullJson),
        ("full", Some("toon")) => Ok(O::FullToon),
        ("full", Some("markdown")) => Err(
            "Markdown で返せるのは改稿指示 (report: brief) だけです。全指摘のレポートは json か toon を指定してください"
                .to_string(),
        ),
        ("brief" | "full", Some(other)) => Err(format!(
            "未知の形式です: {other} (markdown・json・toon のどれかを指定してください)"
        )),
        (other, _) => Err(format!(
            "未知の内容です: {other} (brief か full を指定してください)"
        )),
    }
}

/// 必須の本文の引数 (`check` の `text`、`diff` の `before` / `after`)。大きすぎる本文は断る。
fn text_arg<'a>(
    args: &'a Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<&'a String, String> {
    let Some(Value::String(text)) = args.get(key) else {
        return Err(format!("`{key}` ({label}) を文字列で指定してください"));
    };
    if text.len() > MAX_TEXT_BYTES {
        return Err(format!(
            "{label}が大きすぎます ({} バイト)。{} MiB 以下に分けてください",
            text.len(),
            MAX_TEXT_BYTES / 1024 / 1024
        ));
    }
    Ok(text)
}

fn string_arg<'a>(args: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(format!("`{key}` は文字列で指定してください")),
    }
}

fn bool_arg(args: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` は true か false で指定してください")),
    }
}

fn genre_arg(args: &Map<String, Value>) -> Result<Option<Genre>, String> {
    string_arg(args, "genre")?
        .map(|g| g.parse::<Genre>())
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        Server::new(Ok(EngineOptions::default()))
    }

    fn initialize_request(id: i64, version: &str) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0" }
            }
        })
    }

    /// `initialize` を済ませたサーバー (`_meta` なしのリクエストを受け付ける)。
    fn initialized() -> Server {
        let mut s = server();
        s.handle(initialize_request(0, "2025-11-25")).unwrap();
        s
    }

    /// 1 行ずつ送り、返ってきた行を JSON として読む。
    fn exchange(server: &mut Server, lines: &[Value]) -> Vec<Value> {
        let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
        let mut out = Vec::new();
        serve(server, input.as_bytes(), &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
            .collect()
    }

    fn call(id: i64, name: &str, arguments: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })
    }

    fn modern(id: i64, method: &str, mut params: Value) -> Value {
        params["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "0" },
            "io.modelcontextprotocol/clientCapabilities": {}
        });
        json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
    }

    fn tool_text(response: &Value) -> &str {
        response["result"]["content"][0]["text"].as_str().unwrap()
    }

    const SMELLY: &str = include_str!("../examples/ai-smelly.md");

    #[test]
    fn handshake_list_and_call_round_trip() {
        let mut s = server();
        let responses = exchange(
            &mut s,
            &[
                initialize_request(1, "2025-11-25"),
                json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
                json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
                call(3, "check", json!({ "text": SMELLY, "filename": "a.md" })),
                json!({ "jsonrpc": "2.0", "id": "p", "method": "ping" }),
            ],
        );
        assert_eq!(responses.len(), 4, "通知には応答しない: {responses:?}");
        let init = &responses[0];
        assert_eq!(init["id"], 1);
        assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(init["result"]["serverInfo"]["name"], "noslop");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        assert!(
            init["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("check")
        );

        let names: Vec<&str> = responses[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["check", "diff", "explain", "rules"]);
        for tool in responses[1]["result"]["tools"].as_array().unwrap() {
            assert_eq!(tool["inputSchema"]["type"], "object");
        }

        let checked = &responses[2];
        assert_eq!(checked["result"]["isError"], false);
        let text = tool_text(checked);
        assert!(text.starts_with("# noslop の改稿指示"), "{text}");
        assert!(text.contains("## 改稿のルール"), "{text}");
        assert!(text.contains("## a.md"), "{text}");
        assert!(text.contains("P01"), "{text}");

        assert_eq!(responses[3]["id"], "p");
        assert_eq!(responses[3]["result"], json!({}));
        for r in &responses {
            assert!(
                r["result"].get("resultType").is_none() && r["result"].get("_meta").is_none(),
                "initialize で始めた接続の結果は 2025-11-25 の形のまま: {r}"
            );
        }
    }

    #[test]
    fn initialize_answers_a_supported_version_or_the_latest_handshake_version() {
        let mut s = server();
        let version = |s: &mut Server, v: &str| {
            s.handle(initialize_request(1, v)).unwrap()["result"]["protocolVersion"].clone()
        };
        assert_eq!(version(&mut s, "2025-06-18"), "2025-06-18");
        assert_eq!(version(&mut s, "2024-11-05"), "2024-11-05");
        assert_eq!(version(&mut s, "2099-01-01"), "2025-11-25");
        assert_eq!(
            version(&mut s, "2026-07-28"),
            "2025-11-25",
            "initialize で決めるのは 2025-11-25 以前の版"
        );
    }

    #[test]
    fn requests_without_a_version_are_rejected_before_initialize() {
        let mut s = server();
        let listed = s
            .handle(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }))
            .unwrap();
        assert_eq!(listed["error"]["code"], INVALID_PARAMS);
        assert!(
            listed["error"]["message"]
                .as_str()
                .unwrap()
                .contains("initialize")
        );
        let discover = s
            .handle(json!({ "jsonrpc": "2.0", "id": 2, "method": "server/discover" }))
            .unwrap();
        assert_eq!(discover["error"]["code"], INVALID_PARAMS);
        let ping = s
            .handle(json!({ "jsonrpc": "2.0", "id": 3, "method": "ping" }))
            .unwrap();
        assert_eq!(
            ping["result"],
            json!({}),
            "ping は initialize の前でも答える"
        );
    }

    #[test]
    fn stateless_requests_are_checked_and_answered_with_server_info() {
        let mut s = server();
        let discover = s.handle(modern(1, "server/discover", json!({}))).unwrap();
        let result = &discover["result"];
        assert_eq!(result["resultType"], "complete");
        assert_eq!(result["supportedVersions"][0], "2026-07-28");
        assert!(result["capabilities"]["tools"].is_object());
        assert_eq!(result["_meta"][META_SERVER_INFO]["name"], "noslop");

        let checked = s
            .handle(modern(
                2,
                "tools/call",
                json!({ "name": "check", "arguments": { "text": SMELLY } }),
            ))
            .unwrap();
        assert_eq!(checked["result"]["resultType"], "complete");
        assert_eq!(checked["result"]["isError"], false);
        assert_eq!(
            checked["result"]["_meta"][META_SERVER_INFO]["name"],
            "noslop"
        );
        assert!(
            s.handshake.is_none(),
            "_meta で名乗るリクエストは initialize なしで答える"
        );

        let unsupported = s
            .handle(json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/list",
                "params": { "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "1900-01-01",
                    "io.modelcontextprotocol/clientCapabilities": {}
                } }
            }))
            .unwrap();
        assert_eq!(unsupported["error"]["code"], UNSUPPORTED_PROTOCOL_VERSION);
        assert_eq!(
            unsupported["error"]["message"],
            "Unsupported protocol version"
        );
        assert_eq!(unsupported["error"]["data"]["requested"], "1900-01-01");
        assert_eq!(unsupported["error"]["data"]["supported"][0], "2026-07-28");

        for meta in [
            json!({ "io.modelcontextprotocol/protocolVersion": "2026-07-28" }),
            json!({
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": null
            }),
            json!({
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": "test"
            }),
            json!({ "io.modelcontextprotocol/protocolVersion": 20260728 }),
            json!("2026-07-28"),
        ] {
            let r = s
                .handle(json!({
                    "jsonrpc": "2.0",
                    "id": 4,
                    "method": "tools/list",
                    "params": { "_meta": meta }
                }))
                .unwrap();
            assert_eq!(r["error"]["code"], INVALID_PARAMS, "{r}");
        }
    }

    #[test]
    fn protocol_errors_do_not_stop_the_server() {
        let mut s = initialized();
        let mut out = Vec::new();
        let input = "{not json\n[1,2]\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"resources/list\"}\n\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":{}}\n";
        serve(&mut s, input.as_bytes(), &mut out).unwrap();
        let lines: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            lines.len(),
            4,
            "クライアントからの応答には返さない: {lines:?}"
        );
        assert_eq!(lines[0]["error"]["code"], PARSE_ERROR);
        assert_eq!(lines[0]["id"], Value::Null);
        assert_eq!(lines[1]["error"]["code"], INVALID_REQUEST);
        assert_eq!(lines[2]["error"]["code"], METHOD_NOT_FOUND);
        assert_eq!(lines[2]["id"], 1);
        assert_eq!(lines[3]["id"], 2);
        assert!(lines[3]["result"].is_object());
    }

    #[test]
    fn invalid_utf8_is_a_parse_error() {
        let mut s = server();
        let mut out = Vec::new();
        serve(&mut s, &b"\xff\xfe\n"[..], &mut out).unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["error"]["code"], PARSE_ERROR);
    }

    #[test]
    fn oversized_lines_are_skipped_without_buffering_them() {
        let mut input = &b"0123456789\nabc\n\nlast"[..];
        let mut buf = Vec::new();
        assert_eq!(read_line(&mut input, &mut buf, 5).unwrap(), Line::TooLong);
        assert!(buf.is_empty());
        assert_eq!(read_line(&mut input, &mut buf, 5).unwrap(), Line::Complete);
        assert_eq!(buf, b"abc\n");
        assert_eq!(read_line(&mut input, &mut buf, 5).unwrap(), Line::Complete);
        assert_eq!(buf, b"\n");
        assert_eq!(read_line(&mut input, &mut buf, 5).unwrap(), Line::Complete);
        assert_eq!(buf, b"last");
        assert_eq!(read_line(&mut input, &mut buf, 5).unwrap(), Line::End);
    }

    #[test]
    fn unknown_tools_are_protocol_errors_and_bad_arguments_are_tool_errors() {
        let mut s = initialized();
        let unknown = s.handle(call(1, "fix", json!({}))).unwrap();
        assert_eq!(unknown["error"]["code"], INVALID_PARAMS);
        assert!(
            unknown["error"]["message"]
                .as_str()
                .unwrap()
                .contains("fix")
        );

        for (args, needle) in [
            (json!({}), "`text`"),
            (json!({ "text": 1 }), "`text`"),
            (json!({ "text": "本文。", "genre": "poem" }), "poem"),
            (json!({ "text": "本文。", "format": "xml" }), "xml"),
            (
                json!({ "text": "本文。", "experimental": "yes" }),
                "`experimental`",
            ),
            (
                json!({ "text": "あ".repeat(MAX_TEXT_BYTES / 3 + 1) }),
                "大きすぎます",
            ),
        ] {
            let r = s.handle(call(2, "check", args)).unwrap();
            assert_eq!(r["result"]["isError"], true, "{r}");
            assert!(tool_text(&r).contains(needle), "{r}");
        }
        let r = s
            .handle(call(3, "explain", json!({ "rule": "Z99" })))
            .unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(tool_text(&r).contains("Z99"));
    }

    #[test]
    fn check_returns_the_brief_or_the_full_report_as_json_or_toon() {
        let mut s = initialized();
        let text = |s: &mut Server, id: i64, args: Value| {
            let r = s.handle(call(id, "check", args)).unwrap();
            assert_eq!(r["result"]["isError"], false, "{r}");
            tool_text(&r).to_string()
        };

        // 改稿指示 (既定) の JSON
        let v: Value = serde_json::from_str(&text(
            &mut s,
            1,
            json!({ "text": SMELLY, "format": "json" }),
        ))
        .unwrap();
        assert_eq!(v["kind"], "brief");
        assert_eq!(v["files"][0]["path"], "<text>");
        assert!(!v["files"][0]["rules"].as_array().unwrap().is_empty());

        // 改稿指示の TOON は、ルールと該当箇所を表にする
        let toon = text(
            &mut s,
            2,
            json!({ "text": SMELLY, "report": "brief", "format": "toon" }),
        );
        assert!(
            toon.starts_with("schemaVersion: 1\nkind: brief\n"),
            "{toon}"
        );
        assert!(toon.contains("occurrences["), "{toon}");

        // 全指摘のレポート (JSON が既定、TOON も選べる)
        let v: Value = serde_json::from_str(&text(
            &mut s,
            3,
            json!({ "text": SMELLY, "report": "full" }),
        ))
        .unwrap();
        assert!(!v["files"][0]["diagnostics"].as_array().unwrap().is_empty());
        assert!(v["files"][0]["counts"]["slop"].is_object(), "{v}");
        let toon = text(
            &mut s,
            4,
            json!({ "text": SMELLY, "report": "full", "format": "toon" }),
        );
        assert!(
            toon.starts_with(&format!(
                "schemaVersion: {}\n",
                crate::output::json::SCHEMA_VERSION
            )),
            "{toon}"
        );
        assert!(toon.contains("diagnostics["), "{toon}");

        // 組み合わせられないものと未知の値はツールの実行エラー
        for (id, args) in [
            (
                5,
                json!({ "text": SMELLY, "report": "full", "format": "markdown" }),
            ),
            (6, json!({ "text": SMELLY, "format": "brief" })),
            (7, json!({ "text": SMELLY, "report": "all" })),
        ] {
            let r = s.handle(call(id, "check", args)).unwrap();
            assert_eq!(r["result"]["isError"], true, "{r}");
        }
    }

    #[test]
    fn check_says_so_when_nothing_is_found() {
        let mut s = initialized();
        let clean = s
            .handle(call(
                2,
                "check",
                json!({ "text": "今日は晴れた。散歩に出かけた。\n" }),
            ))
            .unwrap();
        assert!(tool_text(&clean).contains("指摘はありません。"));
    }

    #[test]
    fn diff_reports_new_findings_and_lost_facts_without_terminal_styles() {
        let mut s = initialized();
        let before = "# 案内\n\n2024 年に導入した機能は、利用者の声から生まれました。\n";
        let after = "# 案内\n\n新しい機能は、利用者の声から生まれました。いかがでしたか。\n";
        let r = s
            .handle(call(
                1,
                "diff",
                json!({ "before": before, "after": after, "filename": "guide.md" }),
            ))
            .unwrap();
        assert_eq!(r["result"]["isError"], false, "{r}");
        let text = tool_text(&r);
        assert!(
            text.contains("guide.md (改稿前) → guide.md (改稿後)"),
            "{text}"
        );
        assert!(text.contains("P01"), "新しく出た指摘: {text}");
        assert!(text.contains("2024"), "消えた数値: {text}");
        assert!(!text.contains('\u{1b}'), "端末向けの書式は外す: {text:?}");

        let r = s
            .handle(call(
                2,
                "diff",
                json!({ "before": before, "after": after, "format": "json" }),
            ))
            .unwrap();
        let v: Value = serde_json::from_str(tool_text(&r)).unwrap();
        assert_eq!(v["kind"], "diff");
        assert_eq!(v["hasConcerns"], true);
        assert_eq!(v["before"]["path"], "<text> (改稿前)");

        // 引数の誤りはツールの実行エラーにする
        for (id, args) in [
            (3, json!({ "before": before })),
            (4, json!({ "before": before, "after": 1 })),
            (
                5,
                json!({ "before": before, "after": after, "format": "brief" }),
            ),
        ] {
            let r = s.handle(call(id, "diff", args)).unwrap();
            assert_eq!(r["result"]["isError"], true, "{r}");
        }
    }

    #[test]
    fn explain_and_rules_use_the_cli_text() {
        let mut s = initialized();
        let r = s
            .handle(call(1, "explain", json!({ "rule": "P01" })))
            .unwrap();
        assert_eq!(r["result"]["isError"], false);
        assert!(tool_text(&r).starts_with("P01 "));
        let r = s
            .handle(call(2, "rules", json!({ "genre": "tech" })))
            .unwrap();
        assert!(tool_text(&r).starts_with("ジャンル: tech"));
    }

    #[test]
    fn engines_are_reused_per_genre_and_experimental_flag() {
        let mut s = initialized();
        for _ in 0..3 {
            s.handle(call(1, "check", json!({ "text": "本文。" })))
                .unwrap();
        }
        s.handle(call(
            2,
            "check",
            json!({ "text": "本文。", "genre": "blog" }),
        ))
        .unwrap();
        s.handle(call(
            3,
            "check",
            json!({ "text": "本文。", "experimental": true }),
        ))
        .unwrap();
        assert_eq!(s.engines.len(), 3);
    }

    #[test]
    fn config_errors_are_reported_by_each_tool_call() {
        let mut s = Server::new(Err("noslop.toml: 3 行目が読めません".to_string()));
        let init = s.handle(initialize_request(1, "2025-11-25")).unwrap();
        assert!(init["result"].is_object(), "設定の誤りでも初期化には答える");
        let r = s
            .handle(call(2, "check", json!({ "text": "本文。" })))
            .unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(tool_text(&r).contains("3 行目"));
    }
}
