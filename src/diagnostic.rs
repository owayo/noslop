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
/// `Slop` は AI 臭さの検出、`Readability` は読みやすさの指摘、`Custom` は設定ファイルの
/// 独自ルールの指摘。AI 臭さは AI が書いたような癖を疑う箇所を、読みやすさは AI らしさとは
/// 無関係に読み手の負担になる箇所を指すので、目的が違う。混ぜると両方の判断が濁るため、
/// 出力では節も件数もレーンごとに分ける。
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

/// 手掛かりの強さ: AI 臭さのレーンの指摘が、生成文に特徴的な傾向の手掛かりとしてどれだけ強いか。
///
/// 指摘が正しい確率でも、文章が生成された確率でも、直す必要の大きさでもない (直す優先度は
/// [`Severity`] が表す)。人の文書と生成文書のコーパスでの反応率から決め (基準は
/// `noslop calibrate` の「測った強さ」と同じ)、測れないものは検出の仕組みから決める。
/// 段階どうしを足して点数にはしない。読みやすさのレーンと独自ルールの指摘は持たない。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Signal {
    /// 人の文書にほとんど出ず、生成文書に偏って出る。
    Strong,
    /// 生成文書に多いが人の文書にも出る、または件数が少なく測れていない。
    Moderate,
    /// 人の文書にも同じくらいかそれ以上に出る。
    Weak,
}

impl Signal {
    /// 弱い順に並べたときの順位 (大きいほど強い)。宣言の順に頼らずに比べるのに使う。
    pub fn rank(self) -> u8 {
        match self {
            Signal::Weak => 0,
            Signal::Moderate => 1,
            Signal::Strong => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Signal::Strong => "strong",
            Signal::Moderate => "moderate",
            Signal::Weak => "weak",
        }
    }

    /// 出力での呼び名 (「強い手掛かり」など)。
    pub fn label_ja(self) -> &'static str {
        match self {
            Signal::Strong => "強い手掛かり",
            Signal::Moderate => "中程度の手掛かり",
            Signal::Weak => "弱い手掛かり",
        }
    }

    /// 表の欄に書く段階だけの呼び名 (「強い」「中程度」「弱い」)。
    pub fn level_ja(self) -> &'static str {
        match self {
            Signal::Strong => "強い",
            Signal::Moderate => "中程度",
            Signal::Weak => "弱い",
        }
    }
}

impl PartialOrd for Signal {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Signal {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
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
    /// 手掛かりの強さ (AI 臭さのレーンの組み込みルールだけ)。検出したときの重大度で決め、
    /// 設定で重大度を上書きしても変えない。
    pub signal: Option<Signal>,
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
        signal: Option<Signal>,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule_id: rule_id.into(),
            rule_name: rule_name.into(),
            severity,
            lane,
            signal,
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
