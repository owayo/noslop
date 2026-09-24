//! 機械可読な JSON 出力 (スキーマの版は [`SCHEMA_VERSION`])。
//!
//! 位置は原文の UTF-8 バイトオフセット (`offset`) を正とし、行・列 (1 始まり、列は
//! Unicode スカラー値の個数) を併記する。配列の順序はパス・位置・ルール ID で固定する。
//!
//! 版 2 では、ファイルごとの文書全体の点数 (`files[].score`、自然度スコア) をやめ、抑制していない
//! 指摘をレーンごと・重大度ごとに数えた件数 (`files[].counts`) に置き換えた。指摘を足し合わせた
//! 点数は、コーパスで人の文書と AI の文書を見分けられず、指摘があっても満点が出て安心材料と
//! 誤読されていた。1 件ずつの指摘とその件数だけを出し、判断は書き手に委ねる。

use std::collections::BTreeMap;
use std::io::{self, Write};

use serde::Serialize;

use crate::diagnostic::{Diagnostic, Lane, Metric, RuleStatus, Severity, Span};
use crate::document::{Document, SourceFormat};
use crate::engine::{FileError, FileReport, RunReport};
use crate::morph::MorphologyStatus;
use crate::output::text::{Counts, SeverityCounts};
use crate::output::{Position, RenderOptions, toon};

/// JSON のスキーマの版。互換性のない変更をしたら上げる。
pub const SCHEMA_VERSION: u32 = 2;

/// 列の数え方 (`columnUnit`)。列は Unicode スカラー値の個数で数える。
pub(crate) const COLUMN_UNIT: &str = "unicode-scalar";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report<'a> {
    schema_version: u32,
    tool: Tool,
    column_unit: &'static str,
    settings: Settings<'a>,
    files: Vec<File<'a>>,
    summary: Summary,
    errors: Vec<ErrorEntry<'a>>,
}

/// 出力したツールの名前と版 (`noslop diff` の JSON でも使う)。
#[derive(Serialize)]
pub(crate) struct Tool {
    name: &'static str,
    version: &'static str,
}

impl Tool {
    pub(crate) fn current() -> Self {
        Self {
            name: env!("CARGO_PKG_NAME"),
            version: env!("CARGO_PKG_VERSION"),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Settings<'a> {
    genre: &'static str,
    experimental: bool,
    fail_on: &'static str,
    /// 判定の方式 (形態素解析の辞書を使ったか)。
    morphology: &'a MorphologyStatus,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct File<'a> {
    path: &'a str,
    format: &'static str,
    characters: usize,
    sentences: usize,
    counts: LaneCounts,
    diagnostics: Vec<DiagnosticEntry<'a>>,
    warnings: &'a [String],
}

/// 1 ファイルの、抑制していない指摘の件数 (レーンごと・重大度ごと。`noslop diff` の JSON でも
/// 同じ形で出す)。0 件のレーンと重大度も省かずに出す。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LaneCounts {
    slop: SeverityCounts,
    readability: SeverityCounts,
    custom: SeverityCounts,
}

impl LaneCounts {
    pub(crate) fn of(file: &FileReport) -> Self {
        let c = Counts::of_file(file);
        Self {
            slop: c.slop,
            readability: c.readability,
            custom: c.custom,
        }
    }
}

/// 原文上の範囲 (`start` と `end` に行・列・オフセット)。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Range {
    start: Position,
    end: Position,
}

impl Range {
    pub(crate) fn of(doc: &Document, span: Span) -> Self {
        Self {
            start: Position::of(doc, span.start),
            end: Position::of(doc, span.end),
        }
    }
}

/// 指摘 1 件 (`noslop diff` の JSON でも同じ形で出す)。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticEntry<'a> {
    rule_id: &'a str,
    rule_name: &'a str,
    severity: Severity,
    lane: Lane,
    status: RuleStatus,
    message: &'a str,
    hint: Option<&'a str>,
    range: Range,
    context: Option<Range>,
    excerpt: &'a str,
    related: Vec<Range>,
    metrics: &'a BTreeMap<String, Metric>,
    fingerprint: &'a str,
    suppressed: Option<SuppressedEntry<'a>>,
}

#[derive(Serialize)]
struct SuppressedEntry<'a> {
    reason: Option<&'a str>,
    line: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    files: usize,
    files_with_diagnostics: usize,
    diagnostics: usize,
    by_severity: BySeverity,
    by_lane: ByLane,
    suppressed: usize,
    errors: usize,
}

#[derive(Serialize)]
struct BySeverity {
    error: usize,
    warning: usize,
    info: usize,
}

#[derive(Serialize)]
struct ByLane {
    slop: usize,
    readability: usize,
    custom: usize,
}

/// 読めなかったファイル (改稿指示のデータでも同じ形で出す)。
#[derive(Serialize)]
pub(crate) struct ErrorEntry<'a> {
    pub(crate) path: &'a str,
    pub(crate) message: &'a str,
}

impl<'a> ErrorEntry<'a> {
    pub(crate) fn of(e: &'a FileError) -> Self {
        Self {
            path: &e.path,
            message: &e.message,
        }
    }
}

pub(crate) fn format_name(format: SourceFormat) -> &'static str {
    match format {
        SourceFormat::Markdown => "markdown",
        SourceFormat::PlainText => "text",
    }
}

impl<'a> DiagnosticEntry<'a> {
    pub(crate) fn of(doc: &'a Document, d: &'a Diagnostic) -> Self {
        let excerpt = doc.slice(d.context.unwrap_or(d.span));
        DiagnosticEntry {
            rule_id: &d.rule_id,
            rule_name: &d.rule_name,
            severity: d.severity,
            lane: d.lane,
            status: d.status,
            message: &d.message,
            hint: d.hint.as_deref(),
            range: Range::of(doc, d.span),
            context: d.context.map(|c| Range::of(doc, c)),
            excerpt,
            related: d.related.iter().map(|&s| Range::of(doc, s)).collect(),
            metrics: &d.metrics,
            fingerprint: &d.fingerprint,
            suppressed: d.suppressed.as_ref().map(|s| SuppressedEntry {
                reason: s.reason.as_deref(),
                line: s.line,
            }),
        }
    }
}

fn file_entry(file: &FileReport) -> File<'_> {
    File {
        path: file.path(),
        format: format_name(file.doc.format),
        characters: file.doc.char_count(),
        sentences: file.doc.sentences.len(),
        counts: LaneCounts::of(file),
        diagnostics: file
            .diagnostics
            .iter()
            .map(|d| DiagnosticEntry::of(&file.doc, d))
            .collect(),
        warnings: &file.warnings,
    }
}

/// 実行結果を JSON で書き出す。
pub fn render(report: &RunReport, opts: &RenderOptions, out: &mut dyn Write) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut *out, &build(report, opts)).map_err(io::Error::other)?;
    writeln!(out)
}

/// 実行結果を TOON で書き出す (データは JSON と同じ。末尾改行は付けない)。
pub fn render_toon(
    report: &RunReport,
    opts: &RenderOptions,
    out: &mut dyn Write,
) -> io::Result<()> {
    let text = toon::encode(&build(report, opts))?;
    out.write_all(text.as_bytes())
}

/// 出力するレポートを組み立てる (JSON と TOON で共通)。
fn build<'a>(report: &'a RunReport, opts: &RenderOptions) -> Report<'a> {
    let counts = Counts::of(report);
    let visible = || report.files.iter().flat_map(FileReport::visible);
    let summary = Summary {
        files: report.files.len(),
        files_with_diagnostics: report
            .files
            .iter()
            .filter(|f| f.visible().next().is_some())
            .count(),
        diagnostics: visible().count(),
        by_severity: BySeverity {
            error: visible().filter(|d| d.severity == Severity::Error).count(),
            warning: visible()
                .filter(|d| d.severity == Severity::Warning)
                .count(),
            info: visible().filter(|d| d.severity == Severity::Info).count(),
        },
        by_lane: ByLane {
            slop: counts.slop.total(),
            readability: counts.readability.total(),
            custom: counts.custom.total(),
        },
        suppressed: counts.suppressed,
        errors: report.errors.len(),
    };
    Report {
        schema_version: SCHEMA_VERSION,
        tool: Tool::current(),
        column_unit: COLUMN_UNIT,
        settings: Settings {
            genre: opts.genre.as_str(),
            experimental: opts.experimental,
            fail_on: opts.fail_on.as_str(),
            morphology: &report.morphology,
        },
        files: report.files.iter().map(file_entry).collect(),
        summary,
        errors: report.errors.iter().map(ErrorEntry::of).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Engine, EngineOptions, FileError};

    #[test]
    fn schema_has_expected_keys_and_positions() {
        let engine =
            Engine::with_rules(crate::engine::tests::test_rules(), EngineOptions::default())
                .unwrap();
        let report = RunReport {
            files: vec![engine.lint(Document::markdown("前文。\n\nこれは言えるでしょう。\n"))],
            errors: vec![FileError {
                path: "x.md".into(),
                message: "読み込めません".into(),
            }],
            morphology: Default::default(),
        };
        let mut buf = Vec::new();
        render(&report, &RenderOptions::default(), &mut buf).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(v["schemaVersion"], 2);
        assert_eq!(v["tool"]["name"], "noslop");
        assert_eq!(v["columnUnit"], "unicode-scalar");
        assert_eq!(v["settings"]["failOn"], "never");
        let d = &v["files"][0]["diagnostics"][0];
        assert_eq!(d["ruleId"], "T01");
        assert_eq!(d["severity"], "warning");
        assert_eq!(d["lane"], "slop");
        assert_eq!(d["status"], "stable");
        assert_eq!(d["range"]["start"]["line"], 3);
        assert_eq!(d["range"]["start"]["column"], 4);
        assert_eq!(d["excerpt"], "これは言えるでしょう。");
        assert!(d["suppressed"].is_null());
        assert_eq!(d["fingerprint"].as_str().unwrap().len(), 16);
        assert!(
            v["files"][0].get("score").is_none(),
            "文書全体の点数は出さない"
        );
        assert_eq!(v["files"][0]["counts"]["slop"]["warning"], 1);
        assert_eq!(v["summary"]["diagnostics"], 1);
        assert_eq!(v["summary"]["bySeverity"]["warning"], 1);
        assert_eq!(v["errors"][0]["path"], "x.md");
    }

    #[test]
    fn files_count_unsuppressed_findings_by_lane_and_severity() {
        let engine =
            Engine::with_rules(crate::engine::tests::test_rules(), EngineOptions::default())
                .unwrap();
        let report = RunReport {
            files: vec![engine.lint(Document::markdown(
                "<!-- noslop-disable-next-line T01 -- 引用 -->\nこれは言えるでしょう。\n\nそれも言えるでしょう。上限の設定の検討。\n",
            ))],
            errors: Vec::new(),
            morphology: Default::default(),
        };
        let mut buf = Vec::new();
        render(&report, &RenderOptions::default(), &mut buf).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(
            v["files"][0]["counts"],
            serde_json::json!({
                "slop": { "error": 0, "warning": 1, "info": 0 },
                "readability": { "error": 0, "warning": 0, "info": 1 },
                "custom": { "error": 0, "warning": 0, "info": 0 },
            }),
            "抑制した指摘は数えず、0 件のレーンと重大度も省かない"
        );
        assert_eq!(v["summary"]["suppressed"], 1);

        // TOON も同じデータを描く
        let mut buf = Vec::new();
        render_toon(&report, &RenderOptions::default(), &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.starts_with("schemaVersion: 2\n"), "{s}");
        assert!(s.contains("counts:\n"), "{s}");
        assert!(!s.contains("score"), "{s}");
    }
}
