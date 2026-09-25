//! PreToolUse (Bash): gws で Google ドキュメント・スプレッドシートに書き込む値を、書き込む前に検査する。
//!
//! Bash のコマンド行から gws の書き込みと値を読み ([`crate::gws`])、日本語を含む値を検査する (セルの
//! 数式は除く)。ドキュメントの本文 (文章) は値 1 つを 1 つの文書にしてすべてのルールを当て、セル・
//! タイトル・置き換えの文字列 (短い値) は、gws の呼び出しごとに断片の集まりの文書にして 1 文ずつ判定する
//! ルールだけを当てる。
//!
//! - 本文に警告以上の指摘がある書き込み (`--dry-run` でないもの) は止める (`permissionDecision: "deny"`)。
//!   理由には改稿指示を入れる。書き手が直さないと決めたときは、同じ書き込みをそのままもう一度実行すれば、
//!   30 分以内の 1 回だけ検査せずに通す (セッション・書き込み・書き込み先・値・noslop の版が同じもの)。
//!   止めた記録は、それらのハッシュを名前にしたファイルをキャッシュの置き場所に置き、値やコマンドは
//!   書かない
//! - 短い値だけの指摘・情報だけの指摘・`--dry-run` の値の指摘は止めずに、改稿指示を additionalContext で
//!   渡す (書き込みの後に届く)。セッションの ID やキャッシュの置き場所がなく、通す記録を残せないときも
//!   止めずに知らせる
//! - 指摘がない・gws の書き込みでないときは何も出力しない。gws を含まない Bash は、解析も設定の読み込みも
//!   せずに返す

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{CONTEXT_BUDGET_CHARS, Reviewer, truncate_lines};
use crate::cli::{self, HookArgs};
use crate::diagnostic::{Severity, Span};
use crate::document::{
    Block, BlockKind, Document, DocumentKind, LineIndex, ParseOptions, Sentence, SourceFormat,
    TextMap,
};
use crate::engine::FileReport;
use crate::gws::{self, ValueKind, Write};
use crate::{segment, text};

/// 止めた書き込みを、同じ内容でもう一度実行したときに通す期間 (秒)。
const RETRY_WINDOW_SECS: u64 = 30 * 60;

/// 記録の時刻が今より先でも期限内とみなす幅 (秒)。時計の小さな揺れと、並んで動いたフックの時刻の差を許す。
const CLOCK_SKEW_SECS: u64 = 60;

/// 止めた記録を置くディレクトリ (キャッシュの置き場所の下)。
const STATE_DIR: &str = "hook-state";

/// 短い値の文書で、行と値の対応を示す数の上限。
const LEGEND_LIMIT: usize = 20;

/// 書き込みを止めたときの理由の頭。
const DENY_HEAD: &str = "noslop: gws で書き込む文章に指摘があるので、この書き込みを止めました。まだ書き込んでいません。\n\
直す箇所を直してから、書き込み直してください。直さずにこのまま書き込むと決めたときは、同じコマンドをそのままもう一度実行してください。30 分以内の 1 回だけ、検査せずに通します。\n";

/// 書き込みを止めずに知らせるときの頭。
const CONTEXT_HEAD: &str = "noslop: gws で書き込んだ値に指摘があります。書き込みは止めていません。直すときは、書き込んだ先を改めて更新してください。\n";

/// `--dry-run` の書き込みだけのときの頭。
const DRY_RUN_HEAD: &str = "noslop: gws の --dry-run に渡した値に指摘があります。本番の書き込みの前に、直すかどうかを判断してください。\n";

/// 本文の文書の行番号の注記。
const PROSE_NOTE: &str = "行番号 (L) は、書き込む文章の中の行です。\n";

/// 短い値の検査の注記。
const FRAGMENT_NOTE: &str = "セル・タイトル・置き換えの文字列のような短い値は、1 文ずつ判定するルールだけで検査しています。ルールはひと続きの文章で校正したもので、短い値での精度は確かめていないので、参考として扱ってください。\n";

/// PreToolUse の入力に対して標準出力に書く JSON。何も書かないなら `None`。
pub(super) fn pre_tool_use(
    event: &Value,
    args: &HookArgs,
    env: &cli::Environment,
) -> Result<Option<String>, String> {
    pre_tool_use_at(event, args, env, SystemTime::now())
}

/// [`pre_tool_use`] の本体 (今の時刻を受け取る。テストでは時刻を動かして、通す期間を確かめる)。
fn pre_tool_use_at(
    event: &Value,
    args: &HookArgs,
    env: &cli::Environment,
    now: SystemTime,
) -> Result<Option<String>, String> {
    if event.get("tool_name").and_then(Value::as_str) != Some("Bash") {
        return Ok(None);
    }
    let Some(command) = event.pointer("/tool_input/command").and_then(Value::as_str) else {
        return Ok(None);
    };
    // Bash のたびに呼ばれるので、gws を含まないコマンドは解析も設定の読み込みもせずに返す
    if !command.contains("gws") {
        return Ok(None);
    }
    let writes: Vec<Write> = gws::writes(command)
        .into_iter()
        .filter(|w| w.values.iter().any(checks))
        .collect();
    if writes.is_empty() {
        return Ok(None);
    }
    let now = unix_secs(now);

    // 止めた書き込みをそのまま実行し直したなら、検査せずに 1 回だけ通す
    let record = retry_record(event, env, &writes);
    if record
        .as_ref()
        .is_some_and(|(store, key)| store.take(key, now))
    {
        return Ok(None);
    }

    let cwd = event.get("cwd").and_then(Value::as_str).map(Path::new);
    let reviewer = Reviewer::new(args, cwd, env)?;
    let checked: Vec<Checked> = writes.iter().map(|w| check(&reviewer, w)).collect();
    let blocks = checked.iter().any(Checked::blocks);
    let dry_run_only = checked
        .iter()
        .filter(|c| c.has_prose_findings() || c.has_fragment_findings())
        .all(|c| c.write.dry_run);
    let notes = notes(&checked);
    let reports: Vec<FileReport> = checked
        .into_iter()
        .flat_map(Checked::into_reports)
        .collect();
    let Some(brief) = reviewer.brief(reports)? else {
        return Ok(None);
    };

    if blocks {
        // 止めるのは、実行し直したときに通す記録を残せたときだけ
        match &record {
            Some((store, key)) => match store.put(key, now) {
                Ok(()) => return Ok(Some(deny(&brief, &notes))),
                Err(e) => eprintln!(
                    "noslop: 止めた記録を {} に置けないので、gws の書き込みを止めずに知らせます: {e}",
                    store.dir.display()
                ),
            },
            // 終了コード 0 の標準エラーは Claude Code のデバッグログにだけ残る
            None => eprintln!(
                "noslop: セッションの ID かキャッシュの置き場所がないので、gws の書き込みを止めずに知らせます"
            ),
        }
    }
    let head = if dry_run_only {
        DRY_RUN_HEAD
    } else {
        CONTEXT_HEAD
    };
    Ok(Some(context(head, &brief, &notes)))
}

/// 改稿指示の後ろに添える注記 (行番号の読み方と、短い値の検査の注意)。
fn notes(checked: &[Checked]) -> Vec<String> {
    let mut notes = Vec::new();
    if checked.iter().any(Checked::has_prose_findings) {
        notes.push(PROSE_NOTE.to_string());
    }
    notes.extend(checked.iter().filter_map(Checked::legend));
    if checked.iter().any(Checked::has_fragment_findings) {
        notes.push(FRAGMENT_NOTE.to_string());
    }
    notes
}

/// 検査する値か (日本語を含み、セルの数式でない)。
fn checks(value: &gws::Value) -> bool {
    text::contains_japanese(&value.text) && !(value.cell && value.text.starts_with('='))
}

/// 止めることがありうる書き込みか (`--dry-run` でなく、検査する本文がある)。
fn may_block(write: &Write) -> bool {
    !write.dry_run
        && write
            .values
            .iter()
            .any(|v| v.kind == ValueKind::Prose && checks(v))
}

/// 書き込み 1 つの検査の結果。
struct Checked<'a> {
    write: &'a Write,
    /// 本文 (文章) の値ごとの結果。
    prose: Vec<FileReport>,
    /// 短い値をまとめた文書の結果。
    fragments: Option<Fragments>,
}

/// 短い値をまとめた文書の結果と、値ごとの原文上の範囲と場所。
struct Fragments {
    report: FileReport,
    places: Vec<(Span, String)>,
}

impl Checked<'_> {
    /// 書き込みを止めるか (本文に警告以上の指摘がある、`--dry-run` でない書き込み)。
    fn blocks(&self) -> bool {
        !self.write.dry_run
            && self
                .prose
                .iter()
                .any(|r| r.visible().any(|d| d.severity >= Severity::Warning))
    }

    /// 本文に見せる指摘があるか。
    fn has_prose_findings(&self) -> bool {
        self.prose.iter().any(|r| r.visible().next().is_some())
    }

    /// 短い値に見せる指摘があるか。
    fn has_fragment_findings(&self) -> bool {
        self.fragments
            .as_ref()
            .is_some_and(|f| f.report.visible().next().is_some())
    }

    /// 短い値の文書で、指摘のある行と値の場所の対応 (改稿指示の行番号から値を探せるように)。
    fn legend(&self) -> Option<String> {
        let f = self.fragments.as_ref()?;
        let doc = &f.report.doc;
        let mut lines: BTreeMap<usize, &str> = BTreeMap::new();
        for d in f.report.visible() {
            let at = d.span.start;
            if let Some((_, label)) = f
                .places
                .iter()
                .find(|(span, _)| span.start <= at && at <= span.end)
            {
                lines.entry(doc.lines.line(at)).or_insert(label);
            }
        }
        if lines.is_empty() {
            return None;
        }
        let mut pairs: Vec<String> = lines
            .iter()
            .take(LEGEND_LIMIT)
            .map(|(line, label)| format!("L{line} = {label}"))
            .collect();
        if lines.len() > LEGEND_LIMIT {
            pairs.push(format!("ほか {} 行", lines.len() - LEGEND_LIMIT));
        }
        Some(format!(
            "{} の行と値の対応: {}\n",
            doc.name,
            pairs.join("、")
        ))
    }

    fn into_reports(self) -> impl Iterator<Item = FileReport> {
        self.prose
            .into_iter()
            .chain(self.fragments.map(|f| f.report))
    }
}

/// 書き込み 1 つの値を検査する。
fn check<'a>(reviewer: &Reviewer, write: &'a Write) -> Checked<'a> {
    let suffix = if write.dry_run { " (--dry-run)" } else { "" };
    let values: Vec<&gws::Value> = write.values.iter().filter(|v| checks(v)).collect();
    let prose = values
        .iter()
        .filter(|v| v.kind == ValueKind::Prose)
        .map(|v| {
            let name = format!("gws {} の {}{suffix}", write.method, v.label);
            reviewer.lint(Document::parse(
                name,
                v.text.clone(),
                SourceFormat::PlainText,
                reviewer.parse_options(),
            ))
        })
        .collect();
    let short: Vec<&gws::Value> = values
        .iter()
        .copied()
        .filter(|v| v.kind == ValueKind::Fragment)
        .collect();
    let fragments = (!short.is_empty()).then(|| {
        let mut sources: Vec<&str> = Vec::new();
        for v in &short {
            if !sources.contains(&v.source.as_str()) {
                sources.push(&v.source);
            }
        }
        let name = format!("gws {} の {}{suffix}", write.method, sources.join("・"));
        let (doc, places) = fragments_document(name, &short, reviewer.parse_options());
        Fragments {
            report: reviewer.lint(doc),
            places,
        }
    });
    Checked {
        write,
        prose,
        fragments,
    }
}

/// 短い値を、断片の集まりの文書にする。原文は値を改行でつないだもので、値ごとに 1 つの段落にする
/// (値の中の改行は文の区切り)。値ごとの原文上の範囲と場所も返す。
fn fragments_document(
    name: String,
    values: &[&gws::Value],
    options: &ParseOptions,
) -> (Document, Vec<(Span, String)>) {
    let mut source = String::new();
    let mut places = Vec::with_capacity(values.len());
    for value in values {
        if !source.is_empty() {
            source.push('\n');
        }
        let start = source.len();
        source.push_str(&value.text);
        places.push((Span::new(start, source.len()), value.label.clone()));
    }
    let mut blocks: Vec<Block> = places
        .iter()
        .filter_map(|(span, _)| fragment_block(&source, *span))
        .collect();
    // 文に分ける (Document::parse と同じ)
    let mut sentences = Vec::new();
    for (idx, block) in blocks.iter_mut().enumerate() {
        let begin = sentences.len();
        for piece in segment::split(
            &block.text,
            &block.line_breaks,
            &block.sentence_breaks,
            options.line_breaks,
        ) {
            let body = &block.text[piece.range.clone()];
            sentences.push(Sentence {
                block: idx,
                span: block.to_source(piece.range.clone()),
                length: text::reading_length(body),
                japanese: text::contains_japanese(body),
                embedded_enders: piece.embedded_enders,
                range: piece.range,
            });
        }
        block.sentences = begin..sentences.len();
    }
    let doc = Document {
        name,
        format: SourceFormat::PlainText,
        kind: DocumentKind::Fragments,
        lines: LineIndex::new(&source),
        source,
        bom_len: 0,
        blocks,
        sentences,
        directives: Vec::new(),
    };
    (doc, places)
}

/// 値 1 つ (原文の `span`) の段落。行の前後の空白を除いて行をつなぎ、値の中の改行は文の区切りにする。
/// 空の値は `None`。
fn fragment_block(source: &str, span: Span) -> Option<Block> {
    const BLANKS: [char; 4] = [' ', '\t', '\r', '\u{3000}'];
    let mut text = String::new();
    let mut map = TextMap::default();
    let mut breaks = Vec::new();
    let mut covered: Option<Span> = None;
    let mut offset = span.start;
    for line in source[span.range()].split('\n') {
        let line_start = offset;
        offset += line.len() + 1;
        let rest = line.trim_start_matches(BLANKS);
        let body = rest.trim_end_matches(BLANKS);
        if body.is_empty() {
            continue;
        }
        let start = line_start + (line.len() - rest.len());
        if !text.is_empty() {
            breaks.push(text.len());
            // 日本語どうしでなければ空白でつなぐ (テキストの段落と同じ)
            if let (Some(p), Some(n)) = (text.chars().next_back(), body.chars().next())
                && !(text::is_cjk_like(p) && text::is_cjk_like(n))
            {
                let at = text.len();
                text.push(' ');
                map.push_opaque(at, 1, Span::new(start, start));
            }
        }
        let at = text.len();
        text.push_str(body);
        map.push_exact(at, start, body.len());
        let end = start + body.len();
        covered = Some(covered.map_or(Span::new(start, end), |c| Span::new(c.start, end)));
    }
    Some(Block {
        kind: BlockKind::Paragraph,
        text,
        map,
        span: covered?,
        in_quote: false,
        in_footnote: false,
        line_breaks: breaks.clone(),
        sentence_breaks: breaks,
        marks: Vec::new(),
        sentences: 0..0,
    })
}

/// 書き込みを止める出力。
fn deny(brief: &str, notes: &[String]) -> String {
    let reason = compose(DENY_HEAD, brief, notes);
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": reason,
        }
    })
    .to_string()
}

/// 止めずに知らせる出力。
fn context(head: &str, brief: &str, notes: &[String]) -> String {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "additionalContext": compose(head, brief, notes),
        }
    })
    .to_string()
}

/// 頭・改稿指示・注記をつなぎ、上限の文字数に収める (頭を先に置き、長いときは後ろを省く)。
fn compose(head: &str, brief: &str, notes: &[String]) -> String {
    let mut text = String::from(head);
    text.push_str(brief);
    for note in notes {
        text.push_str(note);
    }
    truncate_lines(&text, CONTEXT_BUDGET_CHARS)
}

/// 止めた記録の置き場所と、この書き込みの記録の名前。止めることのない書き込みだけのときと、セッションの
/// ID かキャッシュの置き場所がないとき (止めずに知らせる) は `None`。
fn retry_record(
    event: &Value,
    env: &cli::Environment,
    writes: &[Write],
) -> Option<(Approvals, String)> {
    if !writes.iter().any(may_block) {
        return None;
    }
    let session = event
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let store = Approvals::new(env.cache_dir.as_deref()?);
    Some((store, approval_key(session, writes)))
}

/// 止めた記録の名前。セッション・noslop の版と、書き込みごとのコマンド (サービス・リソース・メソッド)・
/// `--dry-run` か・書き込み先・値から作るハッシュ (16 進)。
fn approval_key(session: &str, writes: &[Write]) -> String {
    let writes: Vec<Value> = writes
        .iter()
        .map(|w| {
            json!({
                "method": w.method,
                "dryRun": w.dry_run,
                "destination": w.destination,
                "values": w
                    .values
                    .iter()
                    .map(|v| [v.label.as_str(), v.text.as_str()])
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    let material = json!({
        "noslop": env!("CARGO_PKG_VERSION"),
        "session": session,
        "writes": writes,
    });
    Sha256::digest(material.to_string().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 止めた書き込みの記録 (キャッシュの置き場所の `hook-state`)。
///
/// 記録 1 つは、[`approval_key`] を名前にしたファイルで、中身は作った時刻 (UNIX 時間の秒) だけ。
struct Approvals {
    dir: PathBuf,
}

impl Approvals {
    /// キャッシュの置き場所の下の記録 (ディレクトリは記録を作るときに作る)。
    fn new(cache_dir: &Path) -> Self {
        Self {
            dir: cache_dir.join(STATE_DIR),
        }
    }

    /// `key` の記録が期限内にあれば、消して `true` (1 回だけ通す)。期限の切れた記録は消して `false`。
    fn take(&self, key: &str, now: u64) -> bool {
        let path = self.dir.join(key);
        let Ok(content) = fs::read_to_string(&path) else {
            return false;
        };
        let fresh = created(&content).is_some_and(|t| is_fresh(t, now));
        // 消せたときだけ通す (同じ書き込みを同時に 2 回実行しても、通すのは 1 回)
        fs::remove_file(&path).is_ok() && fresh
    }

    /// `key` の記録を作る (一時ファイルに書いてから置き換える)。ついでに期限の切れた記録を消す。
    fn put(&self, key: &str, now: u64) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        self.sweep(now);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let tmp = self
            .dir
            .join(format!(".{key}.{}.{nanos}.tmp", std::process::id()));
        let target = self.dir.join(key);
        let result = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .and_then(|mut f| f.write_all(now.to_string().as_bytes()))
            .and_then(|()| fs::rename(&tmp, &target));
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
            // 同じ書き込みを同時に止めたフックが先に記録を置いた (置き換えられない環境がある) なら、
            // その記録で実行し直しを通せるので、置けたのと同じに扱う
            if target.is_file() {
                return Ok(());
            }
        }
        result
    }

    /// 期限の切れた記録・読めない記録と、残った一時ファイルを消す。
    fn sweep(&self, now: u64) {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let stale = if is_key(name) {
                fs::read_to_string(entry.path())
                    .ok()
                    .and_then(|c| created(&c))
                    .is_none_or(|t| !is_fresh(t, now))
            } else if name.starts_with('.') && name.ends_with(".tmp") {
                // 書いている途中の一時ファイルは消さない (更新時刻が期限より古いものだけ)
                entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .map(unix_secs)
                    .is_none_or(|t| now.saturating_sub(t) > RETRY_WINDOW_SECS)
            } else {
                false
            };
            if stale {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// 記録の中身 (作った時刻)。
fn created(content: &str) -> Option<u64> {
    content.trim().parse().ok()
}

/// `created` に作った記録が、`now` に期限内か。時計が戻ったときに記録を長く効かせないよう、今より先の
/// 時刻は、時計の小さな揺れ ([`CLOCK_SKEW_SECS`]) を超えれば期限切れとみなす。
fn is_fresh(created: u64, now: u64) -> bool {
    created <= now.saturating_add(CLOCK_SKEW_SECS)
        && now.saturating_sub(created) <= RETRY_WINDOW_SECS
}

/// 記録の名前 (SHA-256 の 16 進 64 字) か。
fn is_key(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// UNIX 時間の秒。
fn unix_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::super::testing::parse;
    use super::*;

    /// 本文に「ユーザー様」(警告) と「お知らせ」(情報) を指摘する設定。
    const CONFIG: &str = r#"
[[custom]]
id = "X01"
name = "TEAM_TERM"
pattern = "ユーザー様"
message = "「ユーザー様」ではなく「利用者」と書きます"
severity = "warning"

[[custom]]
id = "X02"
name = "NOTICE_TERM"
pattern = "お知らせ"
message = "「お知らせ」の多用に注意します"
severity = "info"
"#;

    /// 設定を置いた作業ディレクトリと、キャッシュの置き場所。
    struct Workspace {
        dir: tempfile::TempDir,
        cache: tempfile::TempDir,
    }

    impl Workspace {
        fn new() -> Self {
            Self::with_config(CONFIG)
        }

        fn with_config(config: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("noslop.toml"), config).unwrap();
            Self {
                dir,
                cache: tempfile::tempdir().unwrap(),
            }
        }

        /// キャッシュの置き場所だけを持つ環境 (手元の設定と辞書には左右されない)。
        fn env(&self) -> cli::Environment {
            cli::Environment {
                cache_dir: Some(self.cache.path().to_path_buf()),
                ..Default::default()
            }
        }

        fn event(&self, command: &str) -> Value {
            self.event_in("s1", command)
        }

        fn event_in(&self, session: &str, command: &str) -> Value {
            json!({
                "session_id": session,
                "cwd": self.dir.path().to_string_lossy(),
                "hook_event_name": "PreToolUse",
                "tool_name": "Bash",
                "tool_input": { "command": command, "description": "書き込む" }
            })
        }

        fn run_at(&self, event: &Value, now: SystemTime) -> Option<Output> {
            pre_tool_use_at(event, &args(&[]), &self.env(), now)
                .unwrap()
                .map(|o| Output::parse(&o))
        }

        fn run(&self, event: &Value) -> Option<Output> {
            self.run_at(event, t0())
        }

        /// 止めた記録のファイル。
        fn records(&self) -> Vec<PathBuf> {
            let dir = self.cache.path().join(STATE_DIR);
            let mut files: Vec<PathBuf> = fs::read_dir(dir)
                .map(|entries| entries.flatten().map(|e| e.path()).collect())
                .unwrap_or_default();
            files.sort();
            files
        }
    }

    /// フックの出力。
    #[derive(Debug)]
    enum Output {
        Deny(String),
        Context(String),
    }

    impl Output {
        fn parse(output: &str) -> Self {
            let v: Value = serde_json::from_str(output).unwrap();
            let o = &v["hookSpecificOutput"];
            assert_eq!(o["hookEventName"], "PreToolUse", "{output}");
            assert_eq!(
                v.as_object().unwrap().len(),
                1,
                "hookSpecificOutput だけを書く: {output}"
            );
            match o.get("permissionDecision").and_then(Value::as_str) {
                Some("deny") => {
                    assert!(o.get("additionalContext").is_none(), "{output}");
                    Output::Deny(o["permissionDecisionReason"].as_str().unwrap().to_string())
                }
                None => {
                    assert!(o.get("permissionDecisionReason").is_none(), "{output}");
                    Output::Context(o["additionalContext"].as_str().unwrap().to_string())
                }
                Some(other) => panic!("想定しない決定 {other}: {output}"),
            }
        }

        fn deny(self) -> String {
            match self {
                Output::Deny(s) => s,
                other => panic!("止めるはず: {other:?}"),
            }
        }

        fn context(self) -> String {
            match self {
                Output::Context(s) => s,
                other => panic!("知らせるだけのはず: {other:?}"),
            }
        }
    }

    fn args(extra: &[&str]) -> HookArgs {
        let mut argv = vec!["claude-code"];
        argv.extend_from_slice(extra);
        match parse(&argv) {
            cli::HookCommand::ClaudeCode(a) => a,
            _ => panic!("hook claude-code"),
        }
    }

    fn t0() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    const WRITE: &str = "gws docs +write --document D1 --text 'ユーザー様の声を集めました。'";

    #[test]
    fn prose_warnings_deny_the_write_and_an_identical_retry_passes_once() {
        let ws = Workspace::new();
        let event = ws.event(WRITE);
        let reason = ws.run(&event).unwrap().deny();
        assert!(reason.starts_with(DENY_HEAD), "{reason}");
        assert!(
            reason.contains("noslop が gws docs +write の --text に"),
            "{reason}"
        );
        assert!(
            reason.contains("L1: 「ユーザー様」ではなく「利用者」と書きます"),
            "{reason}"
        );
        assert!(reason.contains(PROSE_NOTE), "{reason}");
        assert!(!reason.contains(FRAGMENT_NOTE), "{reason}");

        // 同じコマンドをそのまま実行し直すと 1 回だけ通し、記録を消す
        assert_eq!(ws.records().len(), 1);
        assert!(ws.run(&event).is_none());
        assert!(ws.records().is_empty());
        // 通した後の同じ書き込みは、また検査する
        ws.run(&event).unwrap().deny();
    }

    #[test]
    fn the_same_write_in_another_quoting_is_the_same_retry() {
        let ws = Workspace::new();
        ws.run(&ws.event(WRITE)).unwrap().deny();
        let heredoc = "gws docs +write --text \"$(cat <<'EOF'\nユーザー様の声を集めました。\nEOF\n)\" --document D1 --format json";
        assert!(ws.run(&ws.event(heredoc)).is_none());
    }

    #[test]
    fn retries_pass_only_within_thirty_minutes() {
        let ws = Workspace::new();
        let event = ws.event(WRITE);
        ws.run_at(&event, t0()).unwrap().deny();
        let window = Duration::from_secs(RETRY_WINDOW_SECS);
        assert!(
            ws.run_at(&event, t0() + window).is_none(),
            "30 分ちょうどは通す"
        );

        ws.run_at(&event, t0()).unwrap().deny();
        let late = t0() + window + Duration::from_secs(1);
        ws.run_at(&event, late).unwrap().deny();
        // 期限の切れた記録は消え、止め直した記録から数え直す
        assert_eq!(ws.records().len(), 1);
        assert!(ws.run_at(&event, late + Duration::from_secs(60)).is_none());
    }

    #[test]
    fn records_dated_in_the_future_are_not_trusted() {
        let ws = Workspace::new();
        let event = ws.event(WRITE);
        // 止めた後に時計が 1 時間戻った記録は使わない
        ws.run_at(&event, t0() + Duration::from_secs(3_600))
            .unwrap()
            .deny();
        ws.run_at(&event, t0()).unwrap().deny();
        // 1 分以内の揺れなら通す
        assert!(ws.run_at(&event, t0() - Duration::from_secs(30)).is_none());

        let now = unix_secs(t0());
        assert!(is_fresh(now, now));
        assert!(is_fresh(now - RETRY_WINDOW_SECS, now));
        assert!(!is_fresh(now - RETRY_WINDOW_SECS - 1, now));
        assert!(is_fresh(now + CLOCK_SKEW_SECS, now));
        assert!(!is_fresh(now + CLOCK_SKEW_SECS + 1, now));
    }

    #[test]
    fn a_different_text_destination_session_or_dry_run_is_checked_again() {
        let ws = Workspace::new();
        let event = ws.event(WRITE);
        ws.run(&event).unwrap().deny();
        for other in [
            // 値が違う
            ws.event("gws docs +write --document D1 --text 'ユーザー様の声を集めました!'"),
            // 書き込み先が違う
            ws.event("gws docs +write --document D2 --text 'ユーザー様の声を集めました。'"),
            // セッションが違う
            ws.event_in("s2", WRITE),
        ] {
            ws.run(&other).unwrap().deny();
        }
        // --dry-run は止めず、止めた記録も使わない
        let dry = ws.event(&format!("{WRITE} --dry-run"));
        let context = ws.run(&dry).unwrap().context();
        assert!(context.starts_with(DRY_RUN_HEAD), "{context}");
        assert!(
            context.contains("gws docs +write の --text (--dry-run)"),
            "{context}"
        );
        // 最初の書き込みの記録は残っている
        assert!(ws.run(&event).is_none());
    }

    #[test]
    fn without_a_session_or_a_cache_dir_the_findings_are_only_reported() {
        let ws = Workspace::new();
        let mut no_session = ws.event(WRITE);
        no_session["session_id"] = json!("");
        let context = ws.run(&no_session).unwrap().context();
        assert!(context.starts_with(CONTEXT_HEAD), "{context}");
        assert!(context.contains("X01"), "{context}");
        no_session.as_object_mut().unwrap().remove("session_id");
        ws.run(&no_session).unwrap().context();

        let no_cache = pre_tool_use_at(
            &ws.event(WRITE),
            &args(&[]),
            &cli::Environment::default(),
            t0(),
        )
        .unwrap()
        .map(|o| Output::parse(&o))
        .unwrap();
        no_cache.context();
        assert!(ws.records().is_empty());
    }

    #[test]
    fn an_unwritable_cache_dir_falls_back_to_reporting() {
        let ws = Workspace::new();
        // キャッシュの置き場所がファイルで、ディレクトリを作れない
        let file = ws.cache.path().join("file");
        fs::write(&file, "").unwrap();
        let env = cli::Environment {
            cache_dir: Some(file),
            ..Default::default()
        };
        let out = pre_tool_use_at(&ws.event(WRITE), &args(&[]), &env, t0())
            .unwrap()
            .unwrap();
        Output::parse(&out).context();
    }

    #[test]
    fn info_findings_and_short_values_are_reported_without_denying() {
        let ws = Workspace::new();
        let info = ws
            .run(&ws.event("gws docs +write --document D1 --text 'お知らせです。'"))
            .unwrap()
            .context();
        assert!(info.starts_with(CONTEXT_HEAD), "{info}");
        assert!(info.contains("X02"), "{info}");

        let cells = r#"gws sheets spreadsheets values update --params '{"spreadsheetId":"S","range":"A1"}' --json '{"values":[["名前","ユーザー様の区分"],["=ユーザー様()","ユーザー様"]]}'"#;
        let context = ws.run(&ws.event(cells)).unwrap().context();
        assert!(context.starts_with(CONTEXT_HEAD), "{context}");
        assert!(
            context.contains("noslop が gws sheets spreadsheets values update の values[*][*] に"),
            "{context}"
        );
        assert!(context.contains(FRAGMENT_NOTE), "{context}");
        assert!(!context.contains(PROSE_NOTE), "{context}");
        // 値を 1 行に 1 つ並べた行番号と、値の場所の対応 (数式は検査しない)
        assert!(context.contains("L2:"), "{context}");
        assert!(context.contains("L3:"), "{context}");
        assert!(
            context.contains(
                "gws sheets spreadsheets values update の values[*][*] の行と値の対応: L2 = values[0][1]、L3 = values[1][1]"
            ),
            "{context}"
        );
        assert!(ws.records().is_empty(), "止めないときは記録を残さない");
    }

    #[test]
    fn prose_and_short_values_in_one_command_are_reported_together() {
        let ws = Workspace::new();
        let command = format!(
            "{WRITE} && gws docs documents batchUpdate --params '{{\"documentId\":\"D1\"}}' --json '{}'",
            r#"{"requests":[{"replaceAllText":{"containsText":{"text":"旧"},"replaceText":"ユーザー様"}},{"insertText":{"text":"次の段落です。"}}]}"#
        );
        let event = ws.event(&command);
        let reason = ws.run(&event).unwrap().deny();
        // 複数の値の文書をまとめた改稿指示は、文書ごとに「名前: 件数」の行を並べる
        assert!(reason.contains("\ngws docs +write の --text: "), "{reason}");
        assert!(
            reason.contains(
                "\ngws docs documents batchUpdate の requests[*].replaceAllText.replaceText: "
            ),
            "{reason}"
        );
        assert!(
            reason.contains("の行と値の対応: L1 = requests[0].replaceAllText.replaceText"),
            "{reason}"
        );
        assert!(reason.contains(FRAGMENT_NOTE), "{reason}");
        assert!(ws.run(&event).is_none());
    }

    #[test]
    fn batch_update_insert_texts_are_prose_documents() {
        let ws = Workspace::new();
        let json = r#"{"requests":[{"insertText":{"text":"一行目です。\nユーザー様に届けます。","location":{"index":1}}}]}"#;
        let reason = ws
            .run(&ws.event(&format!(
                "gws docs documents batchUpdate --params '{{\"documentId\":\"D1\"}}' --json '{json}'"
            )))
            .unwrap()
            .deny();
        assert!(
            reason.contains(
                "noslop が gws docs documents batchUpdate の requests[0].insertText.text に"
            ),
            "{reason}"
        );
        assert!(reason.contains("L2:"), "値の中の行番号: {reason}");
    }

    #[test]
    fn clean_values_formulas_and_other_commands_produce_no_output() {
        let ws = Workspace::new();
        for command in [
            "gws docs +write --document D1 --text '今日は晴れた。散歩に出かけた。'",
            "gws docs +write --document D1 --text 'Hello, users.'",
            "gws sheets +append --spreadsheet S --values '=ユーザー様(),42'",
            "gws docs documents get --params '{\"documentId\":\"D1\"}'",
            "gws docs +write --document D1 --text \"$(cat notes.txt)\"",
            "echo ユーザー様",
        ] {
            assert!(ws.run(&ws.event(command)).is_none(), "{command}");
        }
        let mut read = ws.event(WRITE);
        read["tool_name"] = json!("Read");
        assert!(ws.run(&read).is_none());
        let mut no_command = ws.event(WRITE);
        no_command["tool_input"] = json!({});
        assert!(ws.run(&no_command).is_none());
        assert!(
            !ws.cache.path().join(STATE_DIR).exists(),
            "止めなければ記録の置き場所も作らない"
        );
    }

    #[test]
    fn the_config_is_read_only_for_gws_writes() {
        // 壊れた設定は、検査する値があるときだけ読む (読めば誤り)
        let ws = Workspace::with_config("これは TOML ではない [");
        for command in [
            "ls -la",
            "gws docs documents get --params '{\"documentId\":\"D1\"}'",
            "gws docs +write --document D1 --text \"$BODY\"",
            "gws docs +write --document D1 --text 'English only.'",
        ] {
            let out = pre_tool_use_at(&ws.event(command), &args(&[]), &ws.env(), t0());
            assert_eq!(out, Ok(None), "{command}");
        }
        assert!(pre_tool_use_at(&ws.event(WRITE), &args(&[]), &ws.env(), t0()).is_err());
    }

    #[test]
    fn records_hold_only_a_hash_and_the_time() {
        let ws = Workspace::new();
        ws.run(&ws.event(WRITE)).unwrap().deny();
        let records = ws.records();
        assert_eq!(records.len(), 1);
        let name = records[0]
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(is_key(&name), "{name}");
        let content = fs::read_to_string(&records[0]).unwrap();
        assert_eq!(content, unix_secs(t0()).to_string());
    }

    #[test]
    fn stale_records_and_temporary_files_are_swept() {
        let ws = Workspace::new();
        let dir = ws.cache.path().join(STATE_DIR);
        fs::create_dir_all(&dir).unwrap();
        // 一時ファイルは更新時刻で見るので、このテストは実際の時刻で数える
        let now = unix_secs(SystemTime::now());
        let (old, fresh, broken, future, added) = (
            "a".repeat(64),
            "b".repeat(64),
            "c".repeat(64),
            "e".repeat(64),
            "d".repeat(64),
        );
        fs::write(dir.join(&old), (now - RETRY_WINDOW_SECS - 1).to_string()).unwrap();
        fs::write(dir.join(&fresh), (now - 10).to_string()).unwrap();
        fs::write(dir.join(&broken), "壊れた中身").unwrap();
        fs::write(dir.join(&future), (now + 3_600).to_string()).unwrap();
        fs::write(dir.join("unrelated.txt"), "").unwrap();
        fs::write(dir.join(".x.1.2.tmp"), "").unwrap();
        let names = || {
            let mut names: Vec<String> = fs::read_dir(&dir)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };

        // 記録を作るときに、期限の切れた記録・読めない記録・時計が戻る前の先の時刻の記録を消す。
        // 新しい一時ファイルとほかのファイルは残す
        let store = Approvals::new(ws.cache.path());
        store.put(&added, now).unwrap();
        assert_eq!(
            names(),
            vec![
                ".x.1.2.tmp".to_string(),
                fresh.clone(),
                added.clone(),
                "unrelated.txt".to_string()
            ]
        );
        // 期限を過ぎれば、記録も一時ファイルも消す
        store.sweep(now + RETRY_WINDOW_SECS + 60);
        assert_eq!(names(), vec!["unrelated.txt".to_string()]);
    }

    #[test]
    fn long_output_is_cut_within_the_budget() {
        let ws = Workspace::new();
        let text: String = (0..2_000)
            .map(|i| format!("{i} 番目のユーザー様です。\\n"))
            .collect();
        let command = format!("gws docs +write --document D1 --text $'{text}'");
        let event = ws.event(&command);
        let out = pre_tool_use_at(&event, &args(&["--brief-limit", "10000"]), &ws.env(), t0())
            .unwrap()
            .unwrap();
        let reason = Output::parse(&out).deny();
        assert!(
            reason.chars().count() <= CONTEXT_BUDGET_CHARS,
            "{}",
            reason.chars().count()
        );
        assert!(reason.starts_with(DENY_HEAD), "頭は残す");
        assert!(reason.contains("行を省きました"), "{reason}");
    }

    #[test]
    fn fragments_are_one_paragraph_per_value_with_line_breaks() {
        let values = [
            gws::Value {
                label: "values[0][0]".into(),
                source: "values[*][*]".into(),
                kind: ValueKind::Fragment,
                cell: true,
                text: "一行目\n\u{3000}二行目 ".into(),
            },
            gws::Value {
                label: "values[0][1]".into(),
                source: "values[*][*]".into(),
                kind: ValueKind::Fragment,
                cell: true,
                text: "  ".into(),
            },
            gws::Value {
                label: "values[0][2]".into(),
                source: "values[*][*]".into(),
                kind: ValueKind::Fragment,
                cell: true,
                text: "abc\ndef".into(),
            },
        ];
        let refs: Vec<&gws::Value> = values.iter().collect();
        let (doc, places) = fragments_document("断片".to_string(), &refs, &ParseOptions::default());
        assert_eq!(doc.kind, DocumentKind::Fragments);
        assert_eq!(doc.source, "一行目\n\u{3000}二行目 \n  \nabc\ndef");
        assert_eq!(places.len(), 3);
        // 空の値は段落にしない。値の中の改行は文の区切り
        let texts: Vec<&str> = doc.blocks.iter().map(|b| b.text.as_str()).collect();
        assert_eq!(texts, vec!["一行目二行目", "abc def"]);
        let sentences: Vec<&str> = doc.sentences.iter().map(|s| doc.slice(s.span)).collect();
        assert_eq!(sentences, vec!["一行目", "二行目", "abc", "def"]);
        assert!(doc.blocks.iter().all(|b| b.kind == BlockKind::Paragraph));
    }
}
