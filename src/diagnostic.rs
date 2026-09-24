//! 診断 (lint の指摘 1 件) と、その重大度・レーン・ステータス。

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Range;

use serde::Serialize;

/// 指摘の重大度。
///
/// 並び順は `Info < Warning < Error`。`--fail-on` の比較に使う。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }

    /// 端末表示用の日本語ラベル。
    pub fn label_ja(self) -> &'static str {
        match self {
            Severity::Info => "情報",
            Severity::Warning => "警告",
            Severity::Error => "重大",
        }
    }

    /// `info` / `warning` / `error` (と `warn` / `critical` の別名) を解釈する。
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "info" | "information" | "notice" => Some(Severity::Info),
            "warning" | "warn" => Some(Severity::Warning),
            "error" | "critical" => Some(Severity::Error),
            _ => None,
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 指摘の目的の区別。
///
/// `Slop` は AI 臭さの検出で、自然度スコアに入る。`Readability` は読みやすさの
/// 指摘で、AI らしさとは無関係なのでスコアに入れない (混ぜると両方の判断が濁る)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Lane {
    Slop,
    Readability,
    Custom,
}

impl Lane {
    pub fn as_str(self) -> &'static str {
        match self {
            Lane::Slop => "slop",
            Lane::Readability => "readability",
            Lane::Custom => "custom",
        }
    }

    /// 利用者に見せる日本語の呼び名。どの出力でもこの名前で呼び、1 件ずつは「指摘」と数える
    /// (「AI 臭さの指摘 1 件」)。出力ごとに別の呼び方を作ると、要約の件数と一覧が対応しなくなる。
    pub fn label_ja(self) -> &'static str {
        match self {
            Lane::Slop => "AI 臭さ",
            Lane::Readability => "読みやすさ",
            Lane::Custom => "独自ルール",
        }
    }
}

/// ルール (または辞書の 1 項目) の校正状況。
///
/// `Stable` はコーパス校正で誤検知率を確かめた検出器・語句で、既定で有効。
/// `Experimental` は未校正か、辞書なしの近似で校正条件から外れるもので、
/// `--experimental` か設定で明示したときだけ動く。スコアには入れない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleStatus {
    Stable,
    Experimental,
}

impl RuleStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleStatus::Stable => "stable",
            RuleStatus::Experimental => "experimental",
        }
    }
}

/// 原文上の UTF-8 バイト範囲。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        debug_assert!(start <= end, "span start {start} > end {end}");
        Self { start, end }
    }

    pub fn range(self) -> Range<usize> {
        self.start..self.end
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// 診断に添える数値などの付帯情報 (JSON の `metrics` に出る)。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Metric {
    Int(i64),
    Float(f64),
    Text(String),
}

impl From<usize> for Metric {
    fn from(v: usize) -> Self {
        Metric::Int(v as i64)
    }
}

impl From<i64> for Metric {
    fn from(v: i64) -> Self {
        Metric::Int(v)
    }
}

impl From<f64> for Metric {
    fn from(v: f64) -> Self {
        // JSON で読みやすいよう小数第 4 位で丸める (比較に使う値ではない)
        Metric::Float((v * 10_000.0).round() / 10_000.0)
    }
}

impl From<&str> for Metric {
    fn from(v: &str) -> Self {
        Metric::Text(v.to_string())
    }
}

impl From<String> for Metric {
    fn from(v: String) -> Self {
        Metric::Text(v)
    }
}

/// 抑制コメントで「直さずに残す」と判断された記録。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suppression {
    /// 抑制コメントに書かれた理由 (`-- 理由`)。
    pub reason: Option<String>,
    /// 抑制コメントの行 (1 始まり)。ファイル全体の抑制でも記録する。
    pub line: usize,
}

/// lint の指摘 1 件。
///
/// 位置は原文の UTF-8 バイトオフセット (`span`) を正とし、行・列は出力時に
/// [`crate::document::LineIndex`] で計算する。
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// ルール ID (`P01` など)。独自ルールは設定ファイルの ID。
    pub rule_id: String,
    /// ルール名 (`AI_CONCLUSION` など)。
    pub rule_name: String,
    pub severity: Severity,
    pub lane: Lane,
    pub status: RuleStatus,
    /// 何が見つかったか (具体的に)。
    pub message: String,
    /// どう直すかの手掛かり。
    pub hint: Option<String>,
    /// 指摘箇所 (原文のバイト範囲)。文書全体の指摘では代表位置。
    pub span: Span,
    /// 指摘箇所を含む文や段落など、表示用の文脈 (原文のバイト範囲)。
    pub context: Option<Span>,
    /// 同じ集計に基づく他の該当箇所 (反復の検出など)。
    pub related: Vec<Span>,
    /// 判定に使った数値。
    pub metrics: BTreeMap<String, Metric>,
    /// 前回結果との突き合わせ用の識別子。エンジンが最後に埋める。
    pub fingerprint: String,
    /// 抑制コメントで残すと判断されたなら、その記録。
    pub suppressed: Option<Suppression>,
}

impl Diagnostic {
    pub fn new(
        rule_id: impl Into<String>,
        rule_name: impl Into<String>,
        severity: Severity,
        lane: Lane,
        status: RuleStatus,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            rule_name: rule_name.into(),
            severity,
            lane,
            status,
            message: message.into(),
            hint: None,
            span,
            context: None,
            related: Vec::new(),
            metrics: BTreeMap::new(),
            fingerprint: String::new(),
            suppressed: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_context(mut self, context: Span) -> Self {
        self.context = Some(context);
        self
    }

    pub fn with_related(mut self, related: Vec<Span>) -> Self {
        self.related = related;
        self
    }

    pub fn with_metric(mut self, key: impl Into<String>, value: impl Into<Metric>) -> Self {
        self.metrics.insert(key.into(), value.into());
        self
    }

    pub fn is_suppressed(&self) -> bool {
        self.suppressed.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_orders_and_parses() {
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
        assert_eq!(Severity::parse("warn"), Some(Severity::Warning));
        assert_eq!(Severity::parse("critical"), Some(Severity::Error));
        assert_eq!(Severity::parse("nope"), None);
    }

    #[test]
    fn float_metric_is_rounded() {
        assert_eq!(Metric::from(0.123456), Metric::Float(0.1235));
    }
}
