//! 比較結果の出力 (text / json)。
//!
//! ```text
//! 🔁 draft.md → draft-v2.md
//!   AI 臭さの指摘 5 → 2 件、読みやすさの指摘 1 → 1 件
//!   指摘: 新規 1・書き換えても残った 0・解消 4・継続 2・抑制して残した 0
//!
//! 新しく出た指摘 (1 件。優先して確認してください)
//!   12:5  警告  [AI 臭さ]  P03 AI_CONJUNCTION
//!     「さらに」で文をつないでいます
//!     │ さらに、手順を見直した。
//!
//! 事実の変化
//!   消えたもの (1 件。削ってよい情報か確かめてください)
//!     数値「40件」 1 → 0 回 (前 5 行)
//!   増えたもの (1 件)
//!     ⚠ 数値「35%」は改稿前にありません。出典のない数値を足していないか確かめてください (後 7 行)
//! ```
//!
//! text は `noslop check` の端末出力と同じく ANSI の装飾つきで書き、端末でなければ
//! 呼び出し側 (anstream) が取り除く。json のキーは既存の JSON 出力と同じく camelCase で、
//! 位置は原文の UTF-8 バイトオフセット (`offset`) に行・列 (1 始まり、列は Unicode
//! スカラー値の個数) を併記する。指摘と件数 (`counts`) の形は `noslop check --format json`
//! と同じ。
//!
//! 版 2 では、前後の文書全体の点数 (`before.score`・`after.score`、自然度スコア) をやめ、
//! `noslop check --format json` と同じ形の件数 (`before.counts`・`after.counts`) に置き換えた。

use std::collections::BTreeMap;
use std::io::{self, Write};

use anstyle::{AnsiColor, Style};
use serde::Serialize;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::document::Document;
use crate::engine::FileReport;
use crate::output::json::{COLUMN_UNIT, DiagnosticEntry, LaneCounts, Range, Tool, format_name};
use crate::output::text::{Counts, LANES, excerpt};
use crate::output::toon;

use super::DiffReport;
use super::facts::{FactChange, FactKind};
use super::shifts::{Shift, ShiftKind};

/// `noslop diff --format json` のスキーマの版。互換性のない変更をしたら上げる。
pub const DIFF_SCHEMA_VERSION: u32 = 2;

/// 事実・偏りの位置として並べる行番号の上限。
const MAX_LINES_SHOWN: usize = 5;

fn severity_style(s: Severity) -> Style {
    match s {
        Severity::Error => AnsiColor::Red.on_default().bold(),
        Severity::Warning => AnsiColor::Yellow.on_default().bold(),
        Severity::Info => AnsiColor::Cyan.on_default(),
    }
}

fn dim() -> Style {
    Style::new().dimmed()
}

fn bold() -> Style {
    Style::new().bold()
}

/// レーンごとの指摘の件数の前後 (`AI 臭さの指摘 3 → 1 件、読みやすさの指摘 13 → 13 件`)。
///
/// `noslop check` の要約と同じく、AI 臭さは 0 件でも出し、独自ルールと読みやすさは前後の
/// どちらかが 1 件以上のときだけ出す。抑制した指摘は数えない。
fn lane_count_changes(before: &FileReport, after: &FileReport) -> String {
    let (b, a) = (Counts::of_file(before), Counts::of_file(after));
    LANES
        .into_iter()
        .filter_map(|lane| {
            let (nb, na) = (b.lane(lane).total(), a.lane(lane).total());
            (lane == Lane::Slop || nb + na > 0)
                .then(|| format!("{}の指摘 {nb} → {na} 件", lane.label_ja()))
        })
        .collect::<Vec<_>>()
        .join("、")
}

/// 位置の行番号の一覧 (`3・7 行`、多いときは `3・7・9・12・15 行ほか 4 行`)。
fn line_list(doc: &Document, spans: &[Span]) -> Option<String> {
    let mut lines: Vec<usize> = spans.iter().map(|s| doc.lines.line(s.start)).collect();
    lines.sort_unstable();
    lines.dedup();
    if lines.is_empty() {
        return None;
    }
    let shown = lines
        .iter()
        .take(MAX_LINES_SHOWN)
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join("・");
    Some(match lines.len().saturating_sub(MAX_LINES_SHOWN) {
        0 => format!("{shown} 行"),
        more => format!("{shown} 行ほか {more} 行"),
    })
}

/// 比較結果を text で書き出す。
pub fn render_text(report: &DiffReport, out: &mut dyn Write) -> io::Result<()> {
    let (b, dm) = (bold(), dim());
    let (before, after) = (&report.before, &report.after);
    let f = &report.findings;

    writeln!(
        out,
        "🔁 {b}{}{b:#} → {b}{}{b:#}",
        before.path(),
        after.path()
    )?;
    writeln!(out, "  {}", lane_count_changes(before, after))?;
    writeln!(
        out,
        "  指摘: 新規 {}・書き換えても残った {}・解消 {}・継続 {}・抑制して残した {}",
        f.new.len(),
        f.carried_over.len(),
        f.resolved.len(),
        f.persisting.len(),
        f.suppressed.len()
    )?;
    for (side, file) in [("前", before), ("後", after)] {
        for w in &file.warnings {
            let s = AnsiColor::Yellow.on_default();
            writeln!(out, "  {s}⚠ {side}: {w}{s:#}")?;
        }
    }
    writeln!(out, "  {dm}改稿の合否ではなく、見直す箇所の一覧です{dm:#}")?;

    if !f.new.is_empty() {
        writeln!(out)?;
        writeln!(
            out,
            "{b}新しく出た指摘{b:#} ({} 件。優先して確認してください)",
            f.new.len()
        )?;
        for &j in &f.new {
            write_diagnostic(&after.doc, &after.diagnostics[j], "", "", true, out)?;
        }
    }
    if !f.carried_over.is_empty() {
        writeln!(out)?;
        writeln!(
            out,
            "{b}文を書き換えても残った指摘{b:#} ({} 件。直したつもりの文に残っています)",
            f.carried_over.len()
        )?;
        for &(i, j) in &f.carried_over {
            let (line, col) = before.doc.line_col(before.diagnostics[i].span.start);
            let was = format!(" {dm}(前 {line}:{col}){dm:#}");
            write_diagnostic(&after.doc, &after.diagnostics[j], "", &was, true, out)?;
        }
    }
    if !f.resolved.is_empty() {
        writeln!(out)?;
        writeln!(out, "{b}解消した指摘{b:#} ({} 件)", f.resolved.len())?;
        for &i in &f.resolved {
            write_diagnostic(&before.doc, &before.diagnostics[i], "前 ", "", false, out)?;
        }
    }
    if !f.suppressed.is_empty() {
        writeln!(out)?;
        writeln!(
            out,
            "{b}抑制コメントで残した指摘{b:#} ({} 件)",
            f.suppressed.len()
        )?;
        for &(_, j) in &f.suppressed {
            let d = &after.diagnostics[j];
            let (line, col) = after.doc.line_col(d.span.start);
            let reason = d
                .suppressed
                .as_ref()
                .and_then(|s| s.reason.as_deref())
                .unwrap_or("理由の記載なし");
            writeln!(
                out,
                "  {dm}{line}:{col}{dm:#}  {b}{}{b:#} {}  {dm}理由: {reason}{dm:#}",
                d.rule_id, d.rule_name
            )?;
        }
    }
    if !f.persisting.is_empty() {
        writeln!(out)?;
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for &(_, j) in &f.persisting {
            *counts
                .entry(after.diagnostics[j].rule_id.as_str())
                .or_default() += 1;
        }
        let list = counts
            .iter()
            .map(|(id, n)| format!("{id}×{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(
            out,
            "{b}継続している指摘{b:#} ({} 件): {list}",
            f.persisting.len()
        )?;
    }

    let facts = &report.facts;
    writeln!(out)?;
    writeln!(out, "{b}事実の変化{b:#}")?;
    if facts.removed.is_empty() && facts.added.is_empty() {
        writeln!(
            out,
            "  {dm}数値・日付・英字の語・カタカナ語・括弧の語・URL の増減はありません{dm:#}"
        )?;
    }
    if !facts.removed.is_empty() {
        writeln!(
            out,
            "  消えたもの ({} 件。削ってよい情報か確かめてください)",
            facts.removed.len()
        )?;
        for c in &facts.removed {
            write_fact(&before.doc, c, "前", out)?;
        }
    }
    if !facts.added.is_empty() {
        writeln!(out, "  増えたもの ({} 件)", facts.added.len())?;
        for c in &facts.added {
            write_fact(&after.doc, c, "後", out)?;
        }
    }

    writeln!(out)?;
    writeln!(out, "{b}改稿の偏り{b:#}")?;
    if report.shifts.is_empty() {
        writeln!(
            out,
            "  {dm}一律に当てた直しの形跡は見つかりませんでした{dm:#}"
        )?;
    }
    for s in &report.shifts {
        writeln!(out, "  - {}", s.message)?;
        if let Some(lines) = line_list(&after.doc, &s.spans) {
            writeln!(out, "    {dm}後 {lines}{dm:#}")?;
        }
    }
    Ok(())
}

/// 指摘 1 件を、位置の行と抜粋で書く。`detailed` なら説明と直し方の案も書く。
///
/// 見出し行は `12:5  警告  [AI 臭さ]  P03 AI_CONJUNCTION` の形で、どのレーンでも重大度の
/// 隣にレーン名を出す (`noslop check` の端末出力と同じ呼び名)。実験的なら末尾に淡色で
/// `[実験的]` を付ける。`prefix` は位置の前 (`前 ` など)、`suffix` は見出し行の末尾に付ける。
fn write_diagnostic(
    doc: &Document,
    d: &Diagnostic,
    prefix: &str,
    suffix: &str,
    detailed: bool,
    out: &mut dyn Write,
) -> io::Result<()> {
    let (line, col) = doc.line_col(d.span.start);
    let (b, dm, sev) = (bold(), dim(), severity_style(d.severity));
    write!(
        out,
        "  {dm}{prefix}{line}:{col}{dm:#}  {sev}{}{sev:#}  [{}]  {b}{}{b:#} {}",
        d.severity.label_ja(),
        d.lane.label_ja(),
        d.rule_id,
        d.rule_name
    )?;
    if d.status == RuleStatus::Experimental {
        write!(out, " {dm}[実験的]{dm:#}")?;
    }
    writeln!(out, "{suffix}")?;
    if detailed {
        writeln!(out, "    {}", d.message)?;
    }
    // 文脈を持たない指摘 (文書・見出し単位) は、指摘の始まる行を抜粋にする
    let context = d
        .context
        .unwrap_or_else(|| doc.lines.line_span(&doc.source, line));
    let (text, _) = excerpt(&doc.source, context, d.span);
    if !text.trim().is_empty() {
        writeln!(out, "    {dm}│{dm:#} {text}")?;
    }
    if detailed && let Some(hint) = &d.hint {
        let g = AnsiColor::Green.on_default();
        writeln!(out, "    {g}💡 {hint}{g:#}")?;
    }
    Ok(())
}

fn write_fact(doc: &Document, c: &FactChange, side: &str, out: &mut dyn Write) -> io::Result<()> {
    let dm = dim();
    let place =
        line_list(doc, &c.spans).map_or_else(String::new, |l| format!(" {dm}({side} {l}){dm:#}"));
    let kind = c.kind.label_ja();
    if c.suspicious {
        let w = AnsiColor::Yellow.on_default().bold();
        writeln!(
            out,
            "    {w}⚠ {kind}「{}」は改稿前にありません。{w:#}出典のない{kind}を足していないか確かめてください{place}",
            c.text
        )
    } else {
        writeln!(
            out,
            "    {kind}「{}」 {} → {} 回{place}",
            c.text, c.before_count, c.after_count
        )
    }
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report<'a> {
    schema_version: u32,
    kind: &'static str,
    tool: Tool,
    column_unit: &'static str,
    /// 確認事項 (新しい指摘・事実の変化・改稿の偏り) があるか。
    has_concerns: bool,
    before: FileEntry<'a>,
    after: FileEntry<'a>,
    findings: Findings<'a>,
    facts: Facts,
    shifts: Vec<ShiftEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileEntry<'a> {
    path: &'a str,
    format: &'static str,
    characters: usize,
    sentences: usize,
    /// 抑制していない指摘の件数 (`noslop check --format json` の `files[].counts` と同じ形)。
    counts: LaneCounts,
    warnings: &'a [String],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Findings<'a> {
    summary: FindingSummary,
    /// 改稿後の指摘。
    new: Vec<DiagnosticEntry<'a>>,
    /// 文を書き換えても同じルールの同じ語句で残った指摘。
    carried_over: Vec<Pair<'a>>,
    /// 改稿前の指摘。
    resolved: Vec<DiagnosticEntry<'a>>,
    persisting: Vec<Pair<'a>>,
    /// `after` の `suppressed` に抑制コメントの理由と行が入る。
    suppressed: Vec<Pair<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FindingSummary {
    new: usize,
    carried_over: usize,
    resolved: usize,
    persisting: usize,
    suppressed: usize,
}

/// 改稿前と改稿後で対応づけた指摘。
#[derive(Serialize)]
struct Pair<'a> {
    before: DiagnosticEntry<'a>,
    after: DiagnosticEntry<'a>,
}

#[derive(Serialize)]
struct Facts {
    removed: Vec<FactEntry>,
    added: Vec<FactEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FactEntry {
    kind: FactKind,
    /// 代表の表記。
    text: String,
    /// 比べるときの正規化した値 (全角数字・単位の表記ゆれ・漢数字をそろえたもの)。
    key: String,
    before_count: usize,
    after_count: usize,
    /// 改稿前に 1 度も出ない数値・日付が増えた。
    suspicious: bool,
    /// `locations` の文書 (`before` / `after`)。
    locations_in: &'static str,
    locations: Vec<Range>,
}

impl FactEntry {
    fn of(doc: &Document, c: &FactChange, locations_in: &'static str) -> Self {
        Self {
            kind: c.kind,
            text: c.text.clone(),
            key: c.key.clone(),
            before_count: c.before_count,
            after_count: c.after_count,
            suspicious: c.suspicious,
            locations_in,
            locations: c.spans.iter().map(|&s| Range::of(doc, s)).collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShiftEntry {
    kind: ShiftKind,
    message: String,
    before: Option<f64>,
    after: Option<f64>,
    count: Option<usize>,
    /// `locations` の文書 (常に `after`)。
    locations_in: &'static str,
    locations: Vec<Range>,
}

impl ShiftEntry {
    fn of(doc: &Document, s: &Shift) -> Self {
        let round = |v: f64| (v * 10_000.0).round() / 10_000.0;
        Self {
            kind: s.kind,
            message: s.message.clone(),
            before: s.before.map(round),
            after: s.after.map(round),
            count: s.count,
            locations_in: "after",
            locations: s.spans.iter().map(|&sp| Range::of(doc, sp)).collect(),
        }
    }
}

fn file_entry(file: &FileReport) -> FileEntry<'_> {
    FileEntry {
        path: file.path(),
        format: format_name(file.doc.format),
        characters: file.doc.char_count(),
        sentences: file.doc.sentences.len(),
        counts: LaneCounts::of(file),
        warnings: &file.warnings,
    }
}

/// 比較結果を JSON で書き出す。
pub fn render_json(report: &DiffReport, out: &mut dyn Write) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut *out, &build(report))?;
    writeln!(out)
}

/// 比較結果を TOON で書き出す (データは JSON と同じ。末尾改行は付けない)。
pub fn render_toon(report: &DiffReport, out: &mut dyn Write) -> io::Result<()> {
    out.write_all(toon::encode(&build(report))?.as_bytes())
}

/// 出力するデータを組み立てる (JSON と TOON で共通)。
fn build(report: &DiffReport) -> Report<'_> {
    let (before, after) = (&report.before, &report.after);
    let f = &report.findings;
    let pair = |&(i, j): &(usize, usize)| Pair {
        before: DiagnosticEntry::of(&before.doc, &before.diagnostics[i]),
        after: DiagnosticEntry::of(&after.doc, &after.diagnostics[j]),
    };
    Report {
        schema_version: DIFF_SCHEMA_VERSION,
        kind: "diff",
        tool: Tool::current(),
        column_unit: COLUMN_UNIT,
        has_concerns: report.has_concerns(),
        before: file_entry(before),
        after: file_entry(after),
        findings: Findings {
            summary: FindingSummary {
                new: f.new.len(),
                carried_over: f.carried_over.len(),
                resolved: f.resolved.len(),
                persisting: f.persisting.len(),
                suppressed: f.suppressed.len(),
            },
            new: f
                .new
                .iter()
                .map(|&j| DiagnosticEntry::of(&after.doc, &after.diagnostics[j]))
                .collect(),
            carried_over: f.carried_over.iter().map(pair).collect(),
            resolved: f
                .resolved
                .iter()
                .map(|&i| DiagnosticEntry::of(&before.doc, &before.diagnostics[i]))
                .collect(),
            persisting: f.persisting.iter().map(pair).collect(),
            suppressed: f.suppressed.iter().map(pair).collect(),
        },
        facts: Facts {
            removed: report
                .facts
                .removed
                .iter()
                .map(|c| FactEntry::of(&before.doc, c, "before"))
                .collect(),
            added: report
                .facts
                .added
                .iter()
                .map(|c| FactEntry::of(&after.doc, c, "after"))
                .collect(),
        },
        shifts: report
            .shifts
            .iter()
            .map(|s| ShiftEntry::of(&after.doc, s))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CustomRuleConfig;
    use crate::diagnostic::Lane;
    use crate::diff::{compare, lint_pair};
    use crate::document::SourceFormat;
    use crate::engine::{Engine, EngineOptions, Input};

    fn rule(id: &str, name: &str, pattern: &str, message: &str, hint: &str) -> CustomRuleConfig {
        CustomRuleConfig {
            id: id.to_string(),
            name: Some(name.to_string()),
            pattern: pattern.to_string(),
            regex: false,
            message: message.to_string(),
            hint: Some(hint.to_string()),
            severity: None,
            lane: None,
        }
    }

    fn report(before: &str, after: &str) -> DiffReport {
        let options = EngineOptions {
            custom: vec![
                rule(
                    "X01",
                    "CANNED_CLOSING",
                    "と言えるでしょう",
                    "定型の締めです",
                    "言い切るか根拠を書いてください",
                ),
                rule(
                    "X02",
                    "OPENING_WORD",
                    "さて、",
                    "前置きの語です",
                    "削ってください",
                ),
                rule(
                    "X03",
                    "HEAVY_WORD",
                    "非常に重要",
                    "重みだけを足す語です",
                    "何が重要かを書いてください",
                ),
            ],
            ..EngineOptions::default()
        };
        let engine = Engine::with_rules(Vec::new(), options).expect("engine");
        let text = |name: &str, source: &str| Input::Text {
            name: name.to_string(),
            source: source.to_string(),
            format: SourceFormat::Markdown,
        };
        let (b, a) =
            lint_pair(&engine, text("before.md", before), text("after.md", after)).expect("lint");
        compare(b, a)
    }

    fn text_of(r: &DiffReport) -> String {
        let mut buf = Vec::new();
        render_text(r, &mut buf).unwrap();
        anstream::adapter::strip_str(&String::from_utf8(buf).unwrap()).to_string()
    }

    fn json_of(r: &DiffReport) -> serde_json::Value {
        let mut buf = Vec::new();
        render_json(r, &mut buf).unwrap();
        serde_json::from_slice(&buf).unwrap()
    }

    #[test]
    fn text_output_has_every_section_with_lines_and_excerpts() {
        let r = report(
            "さて、問い合わせは40件あった。効果があると言えるでしょう。\n",
            "問い合わせは35件あった。\n\n満足度は8割だと言えるでしょう。\n\n検証は非常に重要だ。\n",
        );
        let text = text_of(&r);
        for part in [
            // AI 臭さは 0 件でも出し、件数の前後を指摘の変化の前に置く
            "🔁 before.md → after.md\n  AI 臭さの指摘 0 → 0 件、独自ルールの指摘 2 → 2 件\n  指摘: ",
            "指摘: 新規 1・書き換えても残った 1・解消 1・継続 0・抑制して残した 0",
            "新しく出た指摘 (1 件。優先して確認してください)",
            "  5:4  警告  [独自ルール]  X03 HEAVY_WORD\n    重みだけを足す語です\n    │ 検証は非常に重要だ。\n    💡 何が重要かを書いてください\n",
            "文を書き換えても残った指摘 (1 件。直したつもりの文に残っています)",
            "  3:8  警告  [独自ルール]  X01 CANNED_CLOSING (前 1:22)\n    定型の締めです\n    │ 満足度は8割だと言えるでしょう。\n    💡 言い切るか根拠を書いてください\n",
            "解消した指摘 (1 件)",
            "  前 1:1  警告  [独自ルール]  X02 OPENING_WORD\n    │ さて、問い合わせは40件あった。\n",
            "事実の変化",
            "消えたもの (1 件。削ってよい情報か確かめてください)",
            "数値「40件」 1 → 0 回 (前 1 行)",
            "⚠ 数値「35件」は改稿前にありません。出典のない数値を足していないか確かめてください (後 1 行)",
            "⚠ 数値「8割」は改稿前にありません。",
            "改稿の偏り",
            "一律に当てた直しの形跡は見つかりませんでした",
        ] {
            assert!(text.contains(part), "「{part}」がない:\n{text}");
        }
        // 解消した指摘には説明と直し方の案を出さない
        assert!(!text.contains("前置きの語です"), "{text}");
        assert!(!text.contains("削ってください"), "{text}");
    }

    #[test]
    fn every_finding_names_its_lane_next_to_the_severity() {
        let doc = Document::markdown("さらに、手順を見直した。\n");
        let heading = |lane: Lane, status: RuleStatus, suffix: &str| {
            let d = Diagnostic::new(
                "P03",
                "AI_CONJUNCTION",
                Severity::Warning,
                lane,
                status,
                Span::new(0, "さらに".len()),
                "「さらに」で文をつないでいます",
            );
            let mut buf = Vec::new();
            write_diagnostic(&doc, &d, "", suffix, false, &mut buf).unwrap();
            let text = anstream::adapter::strip_str(&String::from_utf8(buf).unwrap()).to_string();
            text.lines().next().unwrap().to_string()
        };
        assert_eq!(
            heading(Lane::Slop, RuleStatus::Stable, ""),
            "  1:1  警告  [AI 臭さ]  P03 AI_CONJUNCTION"
        );
        assert_eq!(
            heading(Lane::Readability, RuleStatus::Stable, ""),
            "  1:1  警告  [読みやすさ]  P03 AI_CONJUNCTION"
        );
        // 実験的の印はレーン名と別に末尾へ、改稿前の位置はさらにその後ろ
        assert_eq!(
            heading(Lane::Slop, RuleStatus::Experimental, " (前 2:1)"),
            "  1:1  警告  [AI 臭さ]  P03 AI_CONJUNCTION [実験的] (前 2:1)"
        );
    }

    #[test]
    fn text_output_says_so_when_nothing_changed() {
        let source = "効果があると言えるでしょう。\n";
        let text = text_of(&report(source, source));
        assert!(text.contains("継続している指摘 (1 件): X01×1"), "{text}");
        assert!(text.contains("増減はありません"), "{text}");
        assert!(!text.contains("新しく出た指摘"), "{text}");
        assert!(!text.contains("解消した指摘"), "{text}");
    }

    #[test]
    fn json_output_has_stable_keys_and_positions() {
        let r = report(
            "前置き。\n\n効果があると言えるでしょう。\n",
            "効果があると言えるでしょう。\n\n数値は１２件。\n",
        );
        let v = json_of(&r);
        assert_eq!(v["schemaVersion"], 2);
        assert_eq!(v["kind"], "diff");
        assert_eq!(v["tool"]["name"], "noslop");
        assert_eq!(v["columnUnit"], "unicode-scalar");
        assert_eq!(v["hasConcerns"], true);
        assert_eq!(v["before"]["path"], "before.md");
        assert_eq!(v["after"]["format"], "markdown");
        assert!(
            v["before"].get("score").is_none(),
            "文書全体の点数は出さない"
        );
        let counts = serde_json::json!({
            "slop": { "error": 0, "warning": 0, "info": 0 },
            "readability": { "error": 0, "warning": 0, "info": 0 },
            "custom": { "error": 0, "warning": 1, "info": 0 },
        });
        assert_eq!(v["before"]["counts"], counts);
        assert_eq!(v["after"]["counts"], counts);

        let summary = &v["findings"]["summary"];
        assert_eq!(summary["new"], 0);
        assert_eq!(summary["resolved"], 0);
        assert_eq!(summary["persisting"], 1);
        assert_eq!(summary["suppressed"], 0);
        let p = &v["findings"]["persisting"][0];
        assert_eq!(p["after"]["ruleId"], "X01");
        assert_eq!(p["after"]["range"]["start"]["line"], 1);
        assert_eq!(p["before"]["range"]["start"]["line"], 3);
        assert_eq!(p["after"]["excerpt"], "効果があると言えるでしょう。");
        assert_eq!(p["before"]["fingerprint"], p["after"]["fingerprint"]);

        let added = &v["facts"]["added"][0];
        assert_eq!(added["kind"], "number");
        assert_eq!(added["text"], "１２件");
        assert_eq!(added["key"], "12件");
        assert_eq!(added["beforeCount"], 0);
        assert_eq!(added["afterCount"], 1);
        assert_eq!(added["suspicious"], true);
        assert_eq!(added["locationsIn"], "after");
        let start = &added["locations"][0]["start"];
        assert_eq!(start["line"], 3);
        assert_eq!(start["column"], 4);
        // 「効果が…。」14 文字 (42 バイト) + 空行 2 バイト + 「数値は」9 バイト
        assert_eq!(start["offset"], 53);
        assert_eq!(v["facts"]["removed"].as_array().unwrap().len(), 0);
        assert!(v["shifts"].is_array());
    }

    #[test]
    fn suppressed_findings_carry_the_reason_in_both_formats() {
        let r = report(
            "効果があると言えるでしょう。\n",
            "<!-- noslop-disable-next-line X01 -- 引用のため -->\n効果があると言えるでしょう。\n",
        );
        let text = text_of(&r);
        assert!(text.contains("抑制コメントで残した指摘 (1 件)"), "{text}");
        assert!(
            text.contains("2:6  X01 CANNED_CLOSING  理由: 引用のため"),
            "{text}"
        );
        assert!(
            text.contains("  AI 臭さの指摘 0 → 0 件、独自ルールの指摘 1 → 0 件\n"),
            "抑制した指摘は件数に数えない: {text}"
        );
        let v = json_of(&r);
        let s = &v["findings"]["suppressed"][0];
        assert_eq!(s["after"]["suppressed"]["reason"], "引用のため");
        assert_eq!(s["after"]["suppressed"]["line"], 1);
        assert!(s["before"]["suppressed"].is_null());
        assert_eq!(v["hasConcerns"], false);
    }

    #[test]
    fn lane_counts_skip_empty_lanes_and_leave_out_suppressed_findings() {
        let in_lane = |id: &str, name: &str, pattern: &str, lane: &str| CustomRuleConfig {
            lane: Some(lane.to_string()),
            ..rule(id, name, pattern, "指摘", "直し方")
        };
        let options = EngineOptions {
            custom: vec![
                in_lane("X01", "CANNED_CLOSING", "と言えるでしょう", "slop"),
                in_lane("X02", "NO_CHAIN", "の設定の", "readability"),
            ],
            ..EngineOptions::default()
        };
        let engine = Engine::with_rules(Vec::new(), options).expect("engine");
        let text = |name: &str, source: &str| Input::Text {
            name: name.to_string(),
            source: source.to_string(),
            format: SourceFormat::Markdown,
        };
        let (b, a) = lint_pair(
            &engine,
            text(
                "before.md",
                "効果があると言えるでしょう。上限の設定の検討。\n\n手順は簡単だと言えるでしょう。\n",
            ),
            text(
                "after.md",
                "<!-- noslop-disable-next-line X01 -- 引用のため -->\n効果があると言えるでしょう。上限の設定の検討。\n\n手順は簡単です。\n",
            ),
        )
        .expect("lint");
        let r = compare(b, a);
        // 独自ルールは前後とも 0 件なので出さない。抑制した X01 は改稿後の件数に入れない
        let text = text_of(&r);
        assert!(
            text.contains("\n  AI 臭さの指摘 2 → 0 件、読みやすさの指摘 1 → 1 件\n"),
            "{text}"
        );
        let v = json_of(&r);
        assert_eq!(v["before"]["counts"]["slop"]["warning"], 2);
        assert_eq!(v["after"]["counts"]["slop"]["warning"], 0);
        assert_eq!(v["after"]["counts"]["readability"]["warning"], 1);
        assert_eq!(
            v["after"]["counts"]["custom"],
            serde_json::json!({ "error": 0, "warning": 0, "info": 0 })
        );
    }

    #[test]
    fn line_lists_are_deduplicated_and_capped() {
        let source: String = (1..=8).map(|i| format!("行{i}。\n")).collect();
        let doc = Document::markdown(&source);
        let span_at = |line: usize| {
            let s = doc.lines.line_span(&doc.source, line);
            Span::new(s.start, s.start)
        };
        let spans: Vec<Span> = [3, 1, 3].iter().map(|&l| span_at(l)).collect();
        assert_eq!(line_list(&doc, &spans).unwrap(), "1・3 行");
        let spans: Vec<Span> = (1..=8).map(span_at).collect();
        assert_eq!(
            line_list(&doc, &spans).unwrap(),
            "1・2・3・4・5 行ほか 3 行"
        );
        assert!(line_list(&doc, &[]).is_none());
    }
}
