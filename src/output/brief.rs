//! AI (または人間の編集者) に渡す改稿指示 (`--format brief`、`--report brief`)。
//!
//! 語句の置き換えは指示せず、「どこを、なぜ見直すか」と改稿の制約を渡す。指摘は疑いの
//! 提示なので、件数を減らすこと自体を目的にすると、指標に合わせた書き直し (読点を一律に削る、
//! 体言止めを機械的に足すなど) で別の均一さが生まれる。それを防ぐため、改稿のルールを
//! 必ず冒頭に置き、「残す判断」と「再実行は 1 回だけ」を明記する。
//!
//! 改稿指示はいったんデータ ([`Brief`]) に組み立て、Markdown (人と AI が読む)・JSON (機械が
//! 読む)・TOON (同じデータを少ないトークンで AI に渡す) に描き分ける。データには自然度スコア・
//! metrics・fingerprint・抑制した指摘を入れない。指摘の追跡や照合には完全なレポート
//! (`--format json`) を使う。

use std::io::{self, Write};

use serde::Serialize;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::engine::{FileReport, RunReport};
use crate::genre::Genre;
use crate::morph::Method;
use crate::output::json::{COLUMN_UNIT, ErrorEntry, Tool};
use crate::output::text::excerpt;
use crate::output::{RenderOptions, toon};
use crate::rules::RuleMeta;
use crate::score::Score;

/// 改稿指示のデータ (JSON / TOON) のスキーマの版。互換性のない変更をしたら上げる。
pub const BRIEF_SCHEMA_VERSION: u32 = 1;

/// 1 ルールあたりに並べる箇所の既定の上限。
pub const DEFAULT_LIMIT: usize = 5;

/// フックから返す短い改稿指示で、1 ルールあたりに並べる箇所の既定の上限。
const COMPACT_LIMIT: usize = 3;

/// 「指摘のないファイル」に名前を並べる上限。
const CLEAN_FILES_LIMIT: usize = 20;

/// 改稿のルール (完全版)。
const REVISION_RULES: [&str; 6] = [
    "主張・確信度・数字・固有名詞・引用・想定読者を変えないでください。",
    "原文にない事実・数字・体験を足さないでください。材料が足りないときは、推測で埋めずに書き手に確認してください。",
    "同じ種類の直しを全箇所に一律に当てないでください。効果の大きい箇所を選んで直し、ほかはそのままにします。",
    "指摘は疑いです。文脈上必要なら直さずに残してかまいません。残した指摘と理由は、書き手への報告に添えてください。抑制コメント `<!-- noslop-disable-next-line <ID> -- 理由 -->` は、書き手が今後も残すと決めた箇所にだけ書きます。指摘を消すために足さないでください。",
    "指摘の件数を減らすことや、自然度スコアを上げることを目的にしないでください。",
    "直したら noslop を 1 回だけ再実行し、新しく出た指摘だけを確かめてください。直す前の文書と比べる `noslop diff <直す前> <直した後>` (MCP では `diff`) を使うと、新しく出た指摘と、改稿で消えた数字・固有名詞をまとめて確かめられます。再実行はそこで打ち切ります。",
];

/// 改稿のルール (短縮版。フックから返すとき)。
const REVISION_RULES_COMPACT: &str = "主張・確信度・数字・固有名詞・引用・想定読者は変えない / 原文にない事実や体験を足さない / 同じ直しを全箇所に一律に当てない / 文脈上必要なら残してよい (理由は報告に書く。抑制コメントを指摘を消すために足さない) / 見直しと再実行は 1 回だけ";

/// 指摘がないときの文言。指摘がないことは、文章の出来を保証しない。
const NO_FINDINGS: &str = "指摘はありません。noslop の観点では直す必要はありません。内容の正しさや読み手への合い方は、このツールでは確かめていません。";

// ---------------------------------------------------------------------------
// データ
// ---------------------------------------------------------------------------

/// 改稿指示のデータ。Markdown・JSON・TOON はどれもこれを描く。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Brief<'a> {
    schema_version: u32,
    kind: &'static str,
    tool: Tool,
    column_unit: &'static str,
    settings: Settings<'a>,
    revision_rules: &'static [&'static str],
    editorial_questions: Vec<&'static str>,
    /// 指摘がないときの断り書き。指摘があれば `null`。
    note: Option<&'static str>,
    /// 抑制していない指摘のあるファイル。
    files: Vec<BriefFile<'a>>,
    /// 指摘のないファイル (先頭から [`CLEAN_FILES_LIMIT`] 件まで)。
    clean_files: Vec<&'a str>,
    omitted_clean_files: usize,
    /// 抑制コメントについての注意 (未知のルール名など)。
    warnings: Vec<WarningEntry<'a>>,
    /// 読めなかったファイル。
    errors: Vec<ErrorEntry<'a>>,
}

#[derive(Serialize)]
struct Settings<'a> {
    genre: &'static str,
    experimental: bool,
    /// 判定の方式 (`dictionary`: 形態素解析の辞書の品詞で判定した、`surface`: 辞書なしの近似)。
    method: Method,
    /// 使った辞書の名前 (辞書なしなら `null`)。手元のパスは渡さない。
    dictionary: Option<&'a str>,
}

/// 指摘のあるファイル 1 つ。
///
/// ルールと該当箇所は、入れ子にせず 2 つの表に分ける (該当箇所はルール ID で参照する)。
/// どちらも一様な object の配列になるので、TOON では 1 行 1 要素の表 (tabular form) で書ける。
/// 入れ子にすると、ルールごとに項目名の行が並んで TOON がかえって長くなる。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BriefFile<'a> {
    path: &'a str,
    counts: Counts,
    /// 指摘のあったルール。優先して見る順 (AI 臭さ → 独自ルール → 読みやすさ、校正済み →
    /// 実験的、重大度の高い順、件数の多い順) に並ぶ。
    rules: Vec<RuleSummary<'a>>,
    /// 該当箇所。`rules` の順に、ルールの中では文書の順に並ぶ。
    occurrences: Vec<Occurrence<'a>>,
    /// 自然度スコア。Markdown の参考値にだけ使い、データには出さない (数値を目的に
    /// 書き直させないため)。
    #[serde(skip)]
    score: Option<&'a Score>,
}

/// 指摘のあったルール 1 つ。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuleSummary<'a> {
    rule_id: &'a str,
    rule_name: &'a str,
    /// ルールの日本語名 (独自ルールは名前と同じ値)。
    title: Option<&'static str>,
    lane: Lane,
    max_severity: Severity,
    /// 実験的な指摘だけでできているか。
    experimental_only: bool,
    /// 抑制していない指摘の件数 (`occurrences` に載せなかったものも含む)。
    count: usize,
    /// `occurrences` に載せなかった件数 (1 ルールあたり `--brief-limit` 件まで載せる)。
    omitted_count: usize,
    /// なぜ疑わしいか (ルールの説明の「なぜ問題か」の先頭 2 文)。
    why: Option<String>,
    /// 直し方の方向 (重複を除いて 2 つまでを ` / ` でつないだもの)。
    hint: Option<String>,
    /// 直し方の方向の一覧。Markdown が 1 行ずつ書くのに使う。
    #[serde(skip)]
    hints: Vec<String>,
}

/// 該当箇所 1 つ。行・列は 1 始まりで、列は Unicode スカラー値の個数。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Occurrence<'a> {
    rule_id: &'a str,
    line: usize,
    column: usize,
    message: &'a str,
    /// 指摘を含む文の抜粋。文書全体の集計に基づく指摘など、文を持たないものは `null`。
    excerpt: Option<String>,
}

#[derive(Serialize)]
struct WarningEntry<'a> {
    path: &'a str,
    message: &'a str,
}

impl<'a> Brief<'a> {
    /// 実行結果から改稿指示を組み立てる。`limit` は 1 ルールあたりに並べる箇所の上限。
    pub fn build(report: &'a RunReport, opts: &RenderOptions, limit: usize) -> Self {
        let limit = limit.max(1);
        let (with_findings, clean): (Vec<&FileReport>, Vec<&FileReport>) = report
            .files
            .iter()
            .partition(|f| f.visible().next().is_some());
        let files: Vec<BriefFile> = with_findings
            .into_iter()
            .map(|f| BriefFile::of(f, opts, limit))
            .collect();
        Brief {
            schema_version: BRIEF_SCHEMA_VERSION,
            kind: "brief",
            tool: Tool::current(),
            column_unit: COLUMN_UNIT,
            settings: Settings {
                genre: opts.genre.as_str(),
                experimental: opts.experimental,
                method: report.morphology.method,
                dictionary: report
                    .morphology
                    .dictionary
                    .as_ref()
                    .map(|d| d.name.as_str()),
            },
            revision_rules: &REVISION_RULES,
            editorial_questions: questions(opts.genre),
            note: files.is_empty().then_some(NO_FINDINGS),
            files,
            clean_files: clean
                .iter()
                .take(CLEAN_FILES_LIMIT)
                .map(|f| f.path())
                .collect(),
            omitted_clean_files: clean.len().saturating_sub(CLEAN_FILES_LIMIT),
            warnings: report
                .files
                .iter()
                .flat_map(|f| {
                    f.warnings.iter().map(move |w| WarningEntry {
                        path: f.path(),
                        message: w,
                    })
                })
                .collect(),
            errors: report.errors.iter().map(ErrorEntry::of).collect(),
        }
    }

    /// そのファイルの抑制コメントについての注意。
    fn warnings_of(&self, path: &str) -> impl Iterator<Item = &str> {
        self.warnings
            .iter()
            .filter(move |w| w.path == path)
            .map(|w| w.message)
    }
}

impl<'a> BriefFile<'a> {
    fn of(file: &'a FileReport, opts: &RenderOptions, limit: usize) -> Self {
        let mut rules = Vec::new();
        let mut occurrences = Vec::new();
        for g in group(file) {
            occurrences.extend(g.diags.iter().take(limit).map(|d| Occurrence::of(file, d)));
            rules.push(RuleSummary::of(&g, opts, limit));
        }
        BriefFile {
            path: file.path(),
            counts: Counts::of(file),
            rules,
            occurrences,
            score: file.score.as_ref(),
        }
    }

    /// そのルールの該当箇所。
    fn occurrences_of(&self, rule_id: &str) -> impl Iterator<Item = &Occurrence<'a>> {
        self.occurrences
            .iter()
            .filter(move |o| o.rule_id == rule_id)
    }
}

impl<'a> RuleSummary<'a> {
    fn of(g: &Group<'a>, opts: &RenderOptions, limit: usize) -> Self {
        let meta = opts.catalog.get(g.id);
        // 説明文を持たない独自ルールは要約が指摘の文面と同じなので、重ねて出さない
        let why = meta
            .and_then(why_text)
            .filter(|why| g.diags.first().is_none_or(|d| d.message != *why));
        let hints = g.hints();
        RuleSummary {
            rule_id: g.id,
            rule_name: g.name,
            title: meta.map(|m| m.title),
            lane: g.lane,
            max_severity: g.max_severity,
            experimental_only: g.experimental_only,
            count: g.diags.len(),
            omitted_count: g.diags.len().saturating_sub(limit),
            why,
            hint: (!hints.is_empty()).then(|| hints.join(" / ")),
            hints,
        }
    }

    fn count_label(&self) -> String {
        let mut s = format!("{} {} 件", self.max_severity.label_ja(), self.count);
        if self.experimental_only {
            s.push_str("・実験的");
        }
        s
    }
}

impl<'a> Occurrence<'a> {
    fn of(file: &'a FileReport, d: &'a Diagnostic) -> Self {
        let (line, column) = file.doc.line_col(d.span.start);
        Occurrence {
            rule_id: &d.rule_id,
            line,
            column,
            message: &d.message,
            excerpt: d
                .context
                .map(|context| excerpt(&file.doc.source, context, d.span).0),
        }
    }
}

// ---------------------------------------------------------------------------
// 描画
// ---------------------------------------------------------------------------

fn limit(opts: &RenderOptions) -> usize {
    opts.brief_limit.unwrap_or(DEFAULT_LIMIT)
}

/// 改稿指示を JSON で書き出す。
pub fn render_json(
    report: &RunReport,
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> io::Result<()> {
    let brief = Brief::build(report, opts, limit(opts));
    serde_json::to_writer_pretty(&mut *out, &brief).map_err(io::Error::other)?;
    writeln!(out)
}

/// 改稿指示を TOON で書き出す (末尾改行は付けない)。
pub fn render_toon(
    report: &RunReport,
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> io::Result<()> {
    let brief = Brief::build(report, opts, limit(opts));
    let text = toon::encode(&brief)?;
    out.write_all(text.as_bytes())
}

/// 改稿指示を Markdown で書き出す。
pub fn render(report: &RunReport, opts: &RenderOptions, out: &mut dyn Write) -> io::Result<()> {
    if opts.brief_compact {
        return render_compact(report, opts, out);
    }
    let brief = Brief::build(report, opts, limit(opts));
    writeln!(out, "# noslop の改稿指示")?;
    writeln!(out)?;
    writeln!(
        out,
        "noslop が機械的に拾った「AI 臭さ」の疑いを、編集の判断材料として渡します。語句を置き換えるための指示ではありません。"
    )?;
    writeln!(out)?;

    if brief.files.is_empty() {
        writeln!(out, "{NO_FINDINGS}")?;
        render_clean_file_warnings(&brief, out)?;
        render_errors(&brief, out)?;
        return Ok(());
    }

    writeln!(out, "## 改稿のルール")?;
    writeln!(out)?;
    for (i, rule) in brief.revision_rules.iter().enumerate() {
        writeln!(out, "{}. {rule}", i + 1)?;
    }

    for file in &brief.files {
        writeln!(out)?;
        render_file(&brief, file, out)?;
    }

    writeln!(out)?;
    writeln!(out, "## 編集の問い")?;
    writeln!(out)?;
    writeln!(
        out,
        "ルールでは拾えない観点です。答えが「いいえ」でも、書き手に確かめずに内容を足さないでください。"
    )?;
    writeln!(out)?;
    for q in &brief.editorial_questions {
        writeln!(out, "- {q}")?;
    }

    if !brief.clean_files.is_empty() && !opts.quiet {
        writeln!(out)?;
        writeln!(out, "## 指摘のないファイル")?;
        writeln!(out)?;
        for path in &brief.clean_files {
            writeln!(out, "- {path}")?;
        }
        if brief.omitted_clean_files > 0 {
            writeln!(out, "- ほか {} ファイル", brief.omitted_clean_files)?;
        }
    }
    render_clean_file_warnings(&brief, out)?;
    render_errors(&brief, out)
}

fn render_file(brief: &Brief, file: &BriefFile, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "## {}", file.path)?;
    writeln!(out)?;
    // スコアは参考値。上げること自体を目的にしないよう、改稿のルールで断っている
    match file.score {
        Some(score) => writeln!(
            out,
            "- 自然度 (参考): {}/100 ({})",
            score.value,
            score.band.label_ja()
        )?,
        None => writeln!(out, "- 自然度 (参考): 本文が短いため算出していません")?,
    }
    let c = &file.counts;
    writeln!(
        out,
        "- 指摘: AI 臭さ (校正済み) {} 件 / AI 臭さ (実験的) {} 件 / 独自ルール {} 件 / 読みやすさ {} 件",
        c.stable_slop, c.experimental_slop, c.custom, c.readability
    )?;
    for w in brief.warnings_of(file.path) {
        writeln!(out, "- 抑制コメントの注意: {w}")?;
    }

    let (main, readability): (Vec<&RuleSummary>, Vec<&RuleSummary>) =
        file.rules.iter().partition(|r| r.lane != Lane::Readability);

    if !main.is_empty() {
        writeln!(out)?;
        writeln!(out, "### 優先して見る箇所")?;
        for (i, r) in main.iter().enumerate() {
            writeln!(out)?;
            render_rule(file, r, Some(i + 1), out)?;
        }
    }
    if !readability.is_empty() {
        writeln!(out)?;
        writeln!(out, "### 読みやすさの指さし (優先度は低い)")?;
        writeln!(out)?;
        writeln!(
            out,
            "AI 臭さとは別の、読みにくさの指さしです。読んで引っかからない文はそのままにしてください。"
        )?;
        for r in readability {
            writeln!(out)?;
            render_rule(file, r, None, out)?;
        }
    }
    Ok(())
}

/// 指摘のないファイルの、抑制コメントについての注意。
fn render_clean_file_warnings(brief: &Brief, out: &mut dyn Write) -> io::Result<()> {
    let warned: Vec<&WarningEntry> = brief
        .warnings
        .iter()
        .filter(|w| !brief.files.iter().any(|f| f.path == w.path))
        .collect();
    if warned.is_empty() {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(out, "## 抑制コメントの注意")?;
    writeln!(out)?;
    for w in warned {
        writeln!(out, "- {}: {}", w.path, w.message)?;
    }
    Ok(())
}

fn render_rule(
    file: &BriefFile,
    r: &RuleSummary,
    number: Option<usize>,
    out: &mut dyn Write,
) -> io::Result<()> {
    let prefix = number.map(|n| format!("{n}. ")).unwrap_or_default();
    let mut header = format!("#### {prefix}{} {}", r.rule_id, r.rule_name);
    // 独自ルールは日本語名を持たず、名前と同じ値が入っている
    if let Some(title) = r.title.filter(|t| !t.is_empty() && *t != r.rule_name) {
        header.push_str(&format!(" — {title}"));
    }
    header.push_str(&format!(" ({})", r.count_label()));
    writeln!(out, "{header}")?;
    writeln!(out)?;
    if let Some(why) = &r.why {
        writeln!(out, "- なぜ疑わしいか: {why}")?;
    }
    for hint in &r.hints {
        writeln!(out, "- 直し方の方向: {hint}")?;
    }
    writeln!(out, "- 該当箇所:")?;
    let mut previous: Option<&str> = None;
    for o in file.occurrences_of(r.rule_id) {
        writeln!(
            out,
            "  - L{}: {}",
            o.line,
            message(o.message, &mut previous)
        )?;
        if let Some(text) = &o.excerpt {
            writeln!(out, "    - 原文: {text}")?;
        }
    }
    if r.omitted_count > 0 {
        writeln!(out, "  - ほか {} 件", r.omitted_count)?;
    }
    Ok(())
}

/// 指摘の説明。直前の箇所と同じ文面 (文書全体の集計に基づく指摘など) なら「同上」と書く。
fn message<'a>(message: &'a str, previous: &mut Option<&'a str>) -> &'a str {
    let same = *previous == Some(message);
    *previous = Some(message);
    if same { "同上" } else { message }
}

/// フックから AI に返す短い改稿指示。
///
/// 自然度スコアは載せない (編集のたびに数字を見せると、数字を上げること自体が目的になりやすい)。
fn render_compact(report: &RunReport, opts: &RenderOptions, out: &mut dyn Write) -> io::Result<()> {
    let brief = Brief::build(report, opts, opts.brief_limit.unwrap_or(COMPACT_LIMIT));
    for file in &brief.files {
        let c = &file.counts;
        let slop = c.stable_slop + c.experimental_slop + c.custom;
        let mut found = Vec::new();
        if slop > 0 {
            found.push(format!("AI 臭さの疑いを {slop} 件"));
        }
        if c.readability > 0 {
            found.push(format!("読みやすさの指さしを {} 件", c.readability));
        }
        writeln!(
            out,
            "noslop が {} に {}見つけました。直すかどうかは文脈で判断してください。直さない判断もできます。",
            file.path,
            found.join("、")
        )?;
        writeln!(out, "改稿のルール: {REVISION_RULES_COMPACT}")?;
        for w in brief.warnings_of(file.path) {
            writeln!(out, "抑制コメントの注意: {w}")?;
        }
        for r in &file.rules {
            let mut line = format!("- {}", r.rule_id);
            if let Some(title) = r.title {
                line.push(' ');
                line.push_str(title);
            }
            line.push_str(&format!(" ({})", r.count_label()));
            if r.lane == Lane::Readability {
                line.push_str(" [読みやすさ]");
            }
            if let Some(hint) = r.hints.first() {
                line.push_str(": ");
                line.push_str(hint);
            }
            writeln!(out, "{line}")?;
            let mut previous: Option<&str> = None;
            for o in file.occurrences_of(r.rule_id) {
                writeln!(
                    out,
                    "  - L{}: {}",
                    o.line,
                    message(o.message, &mut previous)
                )?;
            }
            if r.omitted_count > 0 {
                writeln!(out, "  - ほか {} 件", r.omitted_count)?;
            }
        }
        writeln!(
            out,
            "材料 (固有名詞・数字・実例) が足りない箇所は、推測で足さず書き手に確認してください。ルールの詳細は `noslop explain <ID>` で確認できます。"
        )?;
    }
    Ok(())
}

fn render_errors(brief: &Brief, out: &mut dyn Write) -> io::Result<()> {
    if brief.errors.is_empty() {
        return Ok(());
    }
    writeln!(out)?;
    writeln!(out, "## 読めなかったファイル")?;
    writeln!(out)?;
    for e in &brief.errors {
        writeln!(out, "- {}: {}", e.path, e.message)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 組み立ての部品
// ---------------------------------------------------------------------------

/// ジャンルに応じた編集の問い。
fn questions(genre: Genre) -> Vec<&'static str> {
    let mut q = vec![
        "この文書を支える固有名詞・数字・実例は足りていますか。足りなければ推測で足さず、書き手に確認してください。",
        "各段落の最初の一文だけを続けて読んで、論旨が通りますか。",
    ];
    q.push(match genre {
        Genre::Essay => {
            "書き手自身の判断や経験が見える箇所はありますか。誰が書いても同じになる一般論だけの段落はありませんか。"
        }
        Genre::Tech => {
            "手順や結果に、読者が再現できる具体 (コマンド・版・数値・エラーメッセージ) がありますか。"
        }
        Genre::Business => "決定事項・担当・期日が、読んだ人がすぐ動ける形で書かれていますか。",
        Genre::General => {
            "重要な箇所は厚く、そうでない箇所は短く、書く量に濃淡がありますか。"
        }
    });
    q
}

/// 説明文の「なぜ問題か」の節から、最初の 2 文を取り出す。
fn why_text(meta: &'static RuleMeta) -> Option<String> {
    let para = section_first_paragraph(meta.explanation, "なぜ問題か")
        .or_else(|| (!meta.summary.is_empty()).then(|| meta.summary.to_string()))?;
    Some(first_sentences(&para, 2))
}

/// Markdown の `### <name>` の節の最初の段落 (空行まで) を 1 行につなげて返す。
fn section_first_paragraph(text: &str, name: &str) -> Option<String> {
    let mut lines = text.lines();
    lines.find(|l| {
        l.trim_start()
            .strip_prefix('#')
            .map(|rest| rest.trim_start_matches('#').trim() == name)
            .unwrap_or(false)
    })?;
    let mut para = String::new();
    for line in lines {
        let t = line.trim();
        if t.starts_with('#') {
            break;
        }
        if t.is_empty() {
            if para.is_empty() {
                continue;
            }
            break;
        }
        para.push_str(t);
    }
    (!para.is_empty()).then_some(para)
}

/// 句点で区切った先頭 `n` 文。
fn first_sentences(text: &str, n: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    for (i, c) in text.char_indices() {
        if c == '。' {
            count += 1;
            if count == n {
                out.push_str(&text[..i + c.len_utf8()]);
                return out;
            }
        }
    }
    text.to_string()
}

/// ルールごとにまとめた指摘。
struct Group<'a> {
    id: &'a str,
    name: &'a str,
    lane: Lane,
    max_severity: Severity,
    /// 実験的な指摘だけでできているか。
    experimental_only: bool,
    diags: Vec<&'a Diagnostic>,
}

impl Group<'_> {
    /// 重複を除いた直し方 (最大 2 つ)。
    fn hints(&self) -> Vec<String> {
        let mut hints: Vec<String> = Vec::new();
        for d in &self.diags {
            if let Some(h) = &d.hint
                && !hints.contains(h)
            {
                hints.push(h.clone());
            }
            if hints.len() == 2 {
                break;
            }
        }
        hints
    }
}

/// 抑制していない指摘をルールごとにまとめ、重大度の高い順・件数の多い順に並べる。
/// 実験的な指摘だけのルールは、校正済みの指摘を含むルールの後ろに置く。
fn group(file: &FileReport) -> Vec<Group<'_>> {
    let mut groups: Vec<Group> = Vec::new();
    for d in file.visible() {
        match groups.iter_mut().find(|g| g.id == d.rule_id) {
            Some(g) => {
                g.max_severity = g.max_severity.max(d.severity);
                g.experimental_only &= d.status == RuleStatus::Experimental;
                g.diags.push(d);
            }
            None => groups.push(Group {
                id: &d.rule_id,
                name: &d.rule_name,
                lane: d.lane,
                max_severity: d.severity,
                experimental_only: d.status == RuleStatus::Experimental,
                diags: vec![d],
            }),
        }
    }
    let lane_order = |lane: Lane| match lane {
        Lane::Slop => 0,
        Lane::Custom => 1,
        Lane::Readability => 2,
    };
    groups.sort_by(|a, b| {
        (
            lane_order(a.lane),
            a.experimental_only,
            std::cmp::Reverse(a.max_severity),
            std::cmp::Reverse(a.diags.len()),
            a.id,
        )
            .cmp(&(
                lane_order(b.lane),
                b.experimental_only,
                std::cmp::Reverse(b.max_severity),
                std::cmp::Reverse(b.diags.len()),
                b.id,
            ))
    });
    groups
}

/// 1 ファイルの未抑制の指摘の件数。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Counts {
    /// AI 臭さのレーンの校正済みの指摘。
    stable_slop: usize,
    /// AI 臭さのレーンの実験的な指摘。
    experimental_slop: usize,
    /// 独自ルールの指摘。
    custom: usize,
    /// 読みやすさのレーンの指摘。
    readability: usize,
}

impl Counts {
    fn of(file: &FileReport) -> Self {
        let mut c = Counts::default();
        for d in file.visible() {
            match (d.lane, d.status) {
                (Lane::Readability, _) => c.readability += 1,
                (Lane::Custom, _) => c.custom += 1,
                (Lane::Slop, RuleStatus::Stable) => c.stable_slop += 1,
                (Lane::Slop, RuleStatus::Experimental) => c.experimental_slop += 1,
            }
        }
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::engine::{Engine, EngineOptions, FileError};
    use crate::output::RuleCatalog;

    fn engine(experimental: bool) -> Engine {
        Engine::with_rules(
            crate::engine::tests::test_rules(),
            EngineOptions {
                experimental,
                ..Default::default()
            },
        )
        .unwrap()
    }

    fn render_str(report: &RunReport, opts: &RenderOptions) -> String {
        let mut buf = Vec::new();
        render(report, opts, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    fn opts(engine: &Engine) -> RenderOptions {
        RenderOptions {
            catalog: RuleCatalog::from_engine(engine),
            ..Default::default()
        }
    }

    #[test]
    fn full_brief_has_rules_groups_readability_and_questions() {
        let e = engine(true);
        let report = RunReport {
            files: vec![e.lint(Document::markdown(
                "これは言えるでしょう。様々な案がある。上限の設定の検討をする。\n\nやはり言えるでしょう。\n",
            ))],
            errors: Vec::new(),
            morphology: Default::default(),
        };
        let s = render_str(&report, &opts(&e));
        assert!(s.starts_with("# noslop の改稿指示"), "{s}");
        assert!(s.contains("## 改稿のルール"), "{s}");
        assert!(
            s.contains("原文にない事実・数字・体験を足さないでください"),
            "{s}"
        );
        assert!(s.contains("再実行はそこで打ち切ります"), "{s}");
        assert!(s.contains("## <input>.md"), "{s}");
        assert!(
            s.contains("AI 臭さ (校正済み) 2 件 / AI 臭さ (実験的) 1 件 / 独自ルール 0 件 / 読みやすさ 1 件"),
            "{s}"
        );
        // 校正済みのルールが先、実験的なルールが後
        let stable = s.find("#### 1. T01 STABLE_SLOP").expect("T01");
        let experimental = s.find("#### 2. T02 EXPERIMENTAL_SLOP").expect("T02");
        assert!(stable < experimental, "{s}");
        assert!(s.contains("(警告 2 件)"), "{s}");
        assert!(s.contains("(情報 1 件・実験的)"), "{s}");
        assert!(s.contains("  - L1: 「言えるでしょう」があります"), "{s}");
        assert!(s.contains("    - 原文: これは言えるでしょう。"), "{s}");
        assert!(
            s.contains("  - L3: 同上\n    - 原文: やはり言えるでしょう。"),
            "同じ文面は繰り返さない: {s}"
        );
        assert!(s.contains("### 読みやすさの指さし (優先度は低い)"), "{s}");
        assert!(s.contains("#### T03 READABILITY"), "{s}");
        assert!(s.contains("## 編集の問い"), "{s}");
        assert!(s.contains("固有名詞・数字・実例は足りていますか"), "{s}");
    }

    #[test]
    fn limit_folds_extra_occurrences() {
        let e = engine(false);
        let text = "言えるでしょう。".repeat(8);
        let report = RunReport {
            files: vec![e.lint(Document::markdown(format!("{text}\n")))],
            errors: Vec::new(),
            morphology: Default::default(),
        };
        let o = RenderOptions {
            brief_limit: Some(3),
            ..opts(&e)
        };
        let s = render_str(&report, &o);
        assert_eq!(s.matches("  - L1: ").count(), 3, "{s}");
        assert!(s.contains("  - ほか 5 件"), "{s}");
    }

    #[test]
    fn no_findings_says_nothing_needs_fixing() {
        let e = engine(false);
        let report = RunReport {
            files: vec![e.lint(Document::markdown("問題のない文。\n"))],
            errors: Vec::new(),
            morphology: Default::default(),
        };
        let s = render_str(&report, &opts(&e));
        assert!(s.contains(NO_FINDINGS), "{s}");
        assert!(
            s.contains("このツールでは確かめていません"),
            "指摘がないことを出来の保証にしない: {s}"
        );
        assert!(!s.contains("## 改稿のルール"), "{s}");
    }

    #[test]
    fn suppressed_findings_are_left_out_and_clean_files_are_listed() {
        let e = engine(false);
        let report = RunReport {
            files: vec![
                e.lint(Document::parse(
                    "a.md",
                    "<!-- noslop-disable-next-line T01 -- 引用 -->\nこれは言えるでしょう。\n\nまた言えるでしょう。\n",
                    crate::document::SourceFormat::Markdown,
                    &Default::default(),
                )),
                e.lint(Document::parse(
                    "b.md",
                    "問題のない文。\n",
                    crate::document::SourceFormat::Markdown,
                    &Default::default(),
                )),
            ],
            errors: Vec::new(),
            morphology: Default::default(),
        };
        let s = render_str(&report, &opts(&e));
        assert!(s.contains("(警告 1 件)"), "{s}");
        assert!(!s.contains("L2: "), "抑制した指摘は出さない: {s}");
        assert!(s.contains("## 指摘のないファイル\n\n- b.md"), "{s}");

        let quiet = RenderOptions {
            quiet: true,
            ..opts(&e)
        };
        assert!(!render_str(&report, &quiet).contains("## 指摘のないファイル"));
    }

    #[test]
    fn broken_suppression_comments_are_pointed_out() {
        let e = engine(false);
        let parse = |name: &str, src: &str| {
            e.lint(Document::parse(
                name,
                src,
                crate::document::SourceFormat::Markdown,
                &Default::default(),
            ))
        };
        let report = RunReport {
            files: vec![
                parse(
                    "a.md",
                    "<!-- noslop-disable-next-line Z99 -- 理由 -->\nこれは言えるでしょう。\n",
                ),
                parse(
                    "b.md",
                    "<!-- noslop-disable-next-line Z98 -- 理由 -->\n問題のない文。\n",
                ),
            ],
            errors: Vec::new(),
            morphology: Default::default(),
        };
        let s = render_str(&report, &opts(&e));
        assert!(
            s.contains("- 抑制コメントの注意: L1: 抑制コメントの未知のルール「Z99」を無視しました"),
            "{s}"
        );
        assert!(
            s.contains("## 抑制コメントの注意\n\n- b.md: L1: 抑制コメントの未知のルール「Z98」"),
            "{s}"
        );
    }

    #[test]
    fn compact_brief_is_short_and_allows_leaving_findings() {
        let e = engine(false);
        let report = RunReport {
            files: vec![e.lint(Document::markdown(
                "これは言えるでしょう。また言えるでしょう。\n",
            ))],
            errors: Vec::new(),
            morphology: Default::default(),
        };
        let o = RenderOptions {
            brief_compact: true,
            brief_limit: Some(1),
            ..opts(&e)
        };
        let s = render_str(&report, &o);
        assert!(s.contains("AI 臭さの疑いを 2 件見つけました"), "{s}");
        assert!(s.contains("直さない判断もできます"), "{s}");
        assert!(s.contains("再実行は 1 回だけ"), "{s}");
        assert!(s.contains("- T01 テスト (警告 2 件)"), "{s}");
        assert!(s.contains("  - ほか 1 件"), "{s}");
        assert!(!s.contains("## "), "短縮版には見出しを付けない: {s}");
    }

    /// JSON と TOON が描くデータ (Markdown と同じ組み立て)。
    fn data_report(e: &Engine) -> RunReport {
        RunReport {
            files: vec![
                e.lint(Document::parse(
                    "draft.md",
                    "これは言えるでしょう。また言えるでしょう。やはり言えるでしょう。\n",
                    crate::document::SourceFormat::Markdown,
                    &Default::default(),
                )),
                e.lint(Document::parse(
                    "clean.md",
                    "<!-- noslop-disable-next-line Z98 -- 理由 -->\n問題のない文。\n",
                    crate::document::SourceFormat::Markdown,
                    &Default::default(),
                )),
            ],
            errors: vec![FileError {
                path: "missing.md".into(),
                message: "読み込めません".into(),
            }],
            morphology: Default::default(),
        }
    }

    #[test]
    fn json_has_the_same_content_as_the_markdown_brief() {
        let e = engine(false);
        let report = data_report(&e);
        let o = RenderOptions {
            brief_limit: Some(2),
            ..opts(&e)
        };
        let mut buf = Vec::new();
        render_json(&report, &o, &mut buf).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(v["schemaVersion"], BRIEF_SCHEMA_VERSION);
        assert_eq!(v["kind"], "brief");
        assert_eq!(v["columnUnit"], "unicode-scalar");
        assert_eq!(v["revisionRules"].as_array().unwrap().len(), 6);
        assert_eq!(v["editorialQuestions"].as_array().unwrap().len(), 3);
        assert!(v["note"].is_null(), "指摘があるので断り書きはない");

        let file = &v["files"][0];
        assert_eq!(file["path"], "draft.md");
        assert!(file.get("score").is_none(), "スコアはデータに出さない");
        assert_eq!(file["counts"]["stableSlop"], 3);
        let rule = &file["rules"][0];
        assert_eq!(rule["ruleId"], "T01");
        assert_eq!(rule["title"], "テスト");
        assert_eq!(rule["lane"], "slop");
        assert_eq!(rule["maxSeverity"], "warning");
        assert_eq!(rule["experimentalOnly"], false);
        assert_eq!(rule["count"], 3);
        assert_eq!(rule["omittedCount"], 1);
        assert!(rule["hint"].is_null(), "直し方を持たないルール");
        assert!(
            rule.get("hints").is_none(),
            "一覧は Markdown 用で、データには出さない"
        );
        let occurrences = file["occurrences"].as_array().unwrap();
        assert_eq!(occurrences.len(), 2, "--brief-limit まで");
        assert_eq!(occurrences[0]["ruleId"], "T01", "ルール ID で参照する");
        assert_eq!(occurrences[0]["line"], 1);
        assert_eq!(occurrences[0]["column"], 4, "「これは」の後の 4 字目");
        assert_eq!(occurrences[0]["message"], "「言えるでしょう」があります");
        assert_eq!(
            occurrences[1]["message"], occurrences[0]["message"],
            "データでは「同上」にしない"
        );
        assert_eq!(occurrences[0]["excerpt"], "これは言えるでしょう。");

        assert_eq!(v["cleanFiles"], serde_json::json!(["clean.md"]));
        assert_eq!(v["omittedCleanFiles"], 0);
        assert_eq!(v["warnings"][0]["path"], "clean.md");
        assert_eq!(v["errors"][0]["path"], "missing.md");
    }

    #[test]
    fn json_without_findings_carries_the_note() {
        let e = engine(false);
        let report = RunReport {
            files: vec![e.lint(Document::markdown("問題のない文。\n"))],
            errors: Vec::new(),
            morphology: Default::default(),
        };
        let mut buf = Vec::new();
        render_json(&report, &opts(&e), &mut buf).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(v["note"], NO_FINDINGS);
        assert_eq!(v["files"], serde_json::json!([]));
    }

    #[test]
    fn toon_lays_occurrences_out_as_a_table() {
        let e = engine(false);
        let report = data_report(&e);
        let mut buf = Vec::new();
        render_toon(&report, &opts(&e), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("schemaVersion: 1\nkind: brief\n"), "{s}");
        assert!(
            s.contains(
                "rules[1]{ruleId,ruleName,title,lane,maxSeverity,experimentalOnly,count,omittedCount,why,hint}:"
            ),
            "ルールは 1 行 1 ルールの表にする: {s}"
        );
        assert!(
            s.contains("occurrences[3]{ruleId,line,column,message,excerpt}:"),
            "該当箇所は 1 行 1 箇所の表にする: {s}"
        );
        assert!(
            s.contains("T01,1,4,「言えるでしょう」があります,これは言えるでしょう。"),
            "{s}"
        );
        assert!(s.contains("cleanFiles[1]: clean.md"), "{s}");
        assert!(s.contains("errors[1]{path,message}:"), "{s}");
        assert!(!s.ends_with('\n'), "TOON は末尾に改行を付けない");
    }

    #[test]
    fn why_text_takes_the_first_sentences_of_the_section() {
        let text = "### 何を見るか\n\n対象。\n\n### なぜ問題か\n\n一文目です。二文目です。\n三文目です。\n\n次の段落。\n\n### 直し方\n\n直す。\n";
        assert_eq!(
            section_first_paragraph(text, "なぜ問題か").as_deref(),
            Some("一文目です。二文目です。三文目です。")
        );
        assert_eq!(
            first_sentences("一文目です。二文目です。三文目です。", 2),
            "一文目です。二文目です。"
        );
        assert_eq!(section_first_paragraph(text, "例"), None);
    }

    #[test]
    fn questions_depend_on_genre() {
        assert!(questions(Genre::Essay).iter().any(|q| q.contains("経験")));
        assert!(
            questions(Genre::Tech)
                .iter()
                .any(|q| q.contains("コマンド"))
        );
        assert!(
            questions(Genre::Business)
                .iter()
                .any(|q| q.contains("期日"))
        );
        assert_eq!(questions(Genre::General).len(), 3);
    }
}
