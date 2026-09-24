//! ルールの共通の型と、組み込みルールの一覧。
//!
//! ルールは 3 系統に分かれる。
//!
//! - `P` 語句パターン: 文の中の表現を 1 件ずつ拾う ([`phrases`])
//! - `R` リズム・統計: 文や段落をまたいだ集計で拾う ([`rhythm`])
//! - `S` 構造: Markdown の体裁 (太字・箇条書き・見出し) を拾う ([`structure`])
//!
//! どの系統のルールも「AI 臭さ」(`Lane::Slop`) か「読解負荷」(`Lane::Readability`) の
//! どちらかのレーンに属し、校正状況 (`RuleStatus`) を持つ。

pub mod custom;
pub mod phrases;
pub mod rhythm;
pub mod structure;

#[cfg(test)]
pub mod testing;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::document::{Block, BlockKind, Document};
use crate::genre::Genre;
use crate::morph::DocMorphology;

/// ルールの静的な情報。`noslop rules` / `noslop explain` の表示にも使う。
#[derive(Debug)]
pub struct RuleMeta {
    /// ルール ID (`P01` など)。公開後は意味を変えない。
    pub id: &'static str,
    /// ルール名 (`AI_CONCLUSION` など)。
    pub name: &'static str,
    /// 日本語の短い名前 (「結論の押し付け」など)。
    pub title: &'static str,
    pub lane: Lane,
    /// ルール全体の校正状況。語句ルールは項目ごとにも持つ。
    pub status: RuleStatus,
    pub default_severity: Severity,
    /// 1 行の説明。
    pub summary: &'static str,
    /// `noslop explain` で出す本文 (何を見ているか・なぜ AI 臭いか・直し方・例・校正の根拠)。
    pub explanation: &'static str,
}

impl RuleMeta {
    /// このルールの既定の重大度・レーン・ステータスで診断を作る。
    pub fn diagnostic(&'static self, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(
            self.id,
            self.name,
            self.default_severity,
            self.lane,
            self.status,
            span,
            message,
        )
    }
}

/// 語句パターン系ルールがどのブロックを見るか。
///
/// 校正は地の文 (段落) で行ったため、既定は段落だけ。リスト・表・引用は設定で広げる。
/// 見出しは見出し用のルールだけが見る。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Scope {
    pub lists: bool,
    pub tables: bool,
    pub blockquotes: bool,
}

impl Scope {
    /// すべての本文ブロックを見る (リスト・表・引用を含む)。
    pub const ALL: Scope = Scope {
        lists: true,
        tables: true,
        blockquotes: true,
    };

    /// 語句パターン系ルールがこのブロックを見るか。
    pub fn accepts(&self, block: &Block) -> bool {
        if block.in_quote && !self.blockquotes {
            return false;
        }
        match block.kind {
            BlockKind::Paragraph => true,
            BlockKind::ListItem => self.lists,
            BlockKind::TableCell => self.tables,
            BlockKind::Heading(_) => false,
        }
    }
}

/// ルールの実行時の文脈。
pub struct RuleContext<'a> {
    pub doc: &'a Document,
    pub genre: Genre,
    pub scope: Scope,
    /// 実験的な項目 (未校正の語句など) も動かすか。
    pub experimental: bool,
    /// 形態素 (辞書があるときだけ)。辞書を使うルールは、なければ辞書なしの近似で判定する。
    pub morph: Option<&'a DocMorphology<'a>>,
}

impl<'a> RuleContext<'a> {
    pub fn new(doc: &'a Document) -> Self {
        Self {
            doc,
            genre: Genre::General,
            scope: Scope::default(),
            experimental: false,
            morph: None,
        }
    }

    /// 語句パターン系ルールが見るブロック (添字つき)。
    pub fn scoped_blocks(&self) -> impl Iterator<Item = (usize, &'a Block)> + '_ {
        self.doc
            .blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| self.scope.accepts(b))
    }
}

/// ルール。
pub trait Rule: Send + Sync {
    fn meta(&self) -> &'static RuleMeta;

    /// そのジャンルで既定で動かすか。
    ///
    /// ジャンルの正当な慣習と衝突するルール (ビジネス文書の太字・定型見出しなど) は `false` を返す。
    /// 利用者が設定で明示的に有効にした場合はこの判定より優先される。
    fn allowed_in(&self, _genre: Genre) -> bool {
        true
    }

    /// 設定ファイルの `[rules.<ID>]` のうち、エンジンが扱う `enabled` / `severity` 以外のキーを受け取る。
    fn configure(&mut self, key: &str, _value: &toml::Value) -> Result<(), String> {
        Err(format!(
            "{} に `{key}` という設定項目はありません",
            self.meta().id
        ))
    }

    /// 設定できる項目と現在値 (`noslop explain` の表示用)。
    fn options(&self) -> Vec<(&'static str, String)> {
        Vec::new()
    }

    /// 形態素解析の辞書があれば品詞で判定するか。
    ///
    /// エンジンは、これが真の有効なルールがあるときだけ辞書を探して読み込む。
    fn uses_morphology(&self) -> bool {
        false
    }

    /// 校正用に、閾値と比べる値を文書ごとに返す (`noslop calibrate` が閾値の掃引に使う)。
    ///
    /// 閾値を超えたかどうかに関係なく、判定の前提 (最小文数など) を満たす文書では値を返す。
    /// 前提を満たさない文書と、閾値をどう変えても指摘しない文書 (数える対象が 1 つもない文書) は
    /// 空を返す。閾値を持たないルールは実装しない (既定は空)。
    ///
    /// 指摘するかどうかを決める値は、どれか 1 つが閾値を越えることと `check` が指摘を 1 件以上
    /// 出すことを一致させる。重大度の切り替えを決める値は [`Measure::at`] で印を付け、その値が
    /// 閾値を越えることと、その重大度以上の指摘が出ることを一致させる。
    fn measure(&self, _ctx: &RuleContext<'_>) -> Vec<Measure> {
        Vec::new()
    }

    /// 校正の基準: `noslop calibrate` が誤検知率の目標を当てはめる重大度。
    ///
    /// この重大度以上の指摘だけを「指摘された」と数えて、校正が成り立っているかを見直す
    /// (確度の低い語句を意図して情報にしたルールが、情報の指摘だけで見直しに並ばないように)。
    /// 既定は、既定の重大度が警告以上なら警告、情報なら情報。特定の重大度になる割合で
    /// 校正したルールは、その重大度を返す。
    fn calibration_basis(&self) -> Severity {
        if self.meta().default_severity >= Severity::Warning {
            Severity::Warning
        } else {
            Severity::Info
        }
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>);
}

/// 閾値と比べる向き。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fires {
    /// 値が閾値より小さいと指摘する (例: burstiness < threshold)。
    Below,
    /// 値が閾値以下だと指摘する。
    AtOrBelow,
    /// 値が閾値より大きいと指摘する (例: 文長 > max_chars)。
    Above,
    /// 値が閾値以上だと指摘する (例: 比率 >= ratio_threshold)。
    AtOrAbove,
}

/// 校正用の測定値 1 つ。
#[derive(Debug, Clone, PartialEq)]
pub struct Measure {
    /// 測った量の名前 (例: `burstiness`)。
    pub name: &'static str,
    pub value: f64,
    /// この値と比べる閾値の設定キー (例: `threshold`)。`noslop calibrate` はこのキーの値を掃引する。
    pub threshold_key: &'static str,
    pub fires: Fires,
    /// この閾値が決めるのが重大度の切り替えなら、切り替わった先の重大度 (値が閾値を越えると、
    /// 指摘がこの重大度になる)。`None` なら、指摘するかどうかを決める閾値。
    pub switches_to: Option<Severity>,
}

impl Measure {
    /// 指摘するかどうかを決める閾値と比べる値。
    pub fn new(name: &'static str, value: f64, threshold_key: &'static str, fires: Fires) -> Self {
        Self {
            name,
            value,
            threshold_key,
            fires,
            switches_to: None,
        }
    }

    /// この閾値が決めるのは、重大度 `severity` への切り替えだとする
    /// (例: R05 の `error_above` は、指摘を重大にするかどうかを決める)。
    pub fn at(mut self, severity: Severity) -> Self {
        self.switches_to = Some(severity);
        self
    }

    /// 閾値 `threshold` で指摘するか (重大度の切り替えなら、その重大度になるか)。
    pub fn fires_at(&self, threshold: f64) -> bool {
        match self.fires {
            Fires::Below => self.value < threshold,
            Fires::AtOrBelow => self.value <= threshold,
            Fires::Above => self.value > threshold,
            Fires::AtOrAbove => self.value >= threshold,
        }
    }
}

/// 組み込みルールを、そのジャンルの既定の閾値で作る。
pub fn builtin_rules(genre: Genre) -> Vec<Box<dyn Rule>> {
    let mut rules: Vec<Box<dyn Rule>> = Vec::new();
    rules.extend(phrases::rules(genre));
    rules.extend(rhythm::rules(genre));
    rules.extend(structure::rules(genre));
    rules
}

/// 設定値を数値として読む。
pub fn option_f64(key: &str, value: &toml::Value) -> Result<f64, String> {
    match value {
        toml::Value::Float(f) => Ok(*f),
        toml::Value::Integer(i) => Ok(*i as f64),
        _ => Err(format!("`{key}` には数値を指定してください")),
    }
}

/// 設定値を 0 以上の整数として読む。
pub fn option_usize(key: &str, value: &toml::Value) -> Result<usize, String> {
    match value {
        toml::Value::Integer(i) if *i >= 0 => Ok(*i as usize),
        _ => Err(format!("`{key}` には 0 以上の整数を指定してください")),
    }
}

/// 診断のメッセージに引用する本文 (プレースホルダは「…」に置き換え、空白を詰める)。
pub(crate) fn quote(text: &str) -> String {
    let replaced: String = text
        .chars()
        .map(|c| {
            if c == crate::text::PLACEHOLDER {
                '…'
            } else {
                c
            }
        })
        .collect();
    replaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn builtin_rule_ids_and_names_are_unique() {
        let rules = builtin_rules(Genre::General);
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for r in &rules {
            let m = r.meta();
            assert!(ids.insert(m.id), "duplicate id {}", m.id);
            assert!(names.insert(m.name), "duplicate name {}", m.name);
            assert!(!m.title.is_empty() && !m.summary.is_empty());
        }
    }

    #[test]
    fn calibration_basis_follows_the_default_severity() {
        for rule in builtin_rules(Genre::General) {
            let m = rule.meta();
            let expected = match m.id {
                // 「人の文書で重大になる割合」で校正した
                "R05" => Severity::Error,
                _ if m.default_severity >= Severity::Warning => Severity::Warning,
                _ => Severity::Info,
            };
            assert_eq!(rule.calibration_basis(), expected, "{}", m.id);
        }
    }

    #[test]
    fn measures_mark_severity_switches() {
        let m = Measure::new("ratio", 0.04, "error_above", Fires::AtOrAbove);
        assert_eq!(m.switches_to, None);
        let m = m.at(Severity::Error);
        assert_eq!(m.switches_to, Some(Severity::Error));
        assert!(m.fires_at(0.03) && m.fires_at(0.04) && !m.fires_at(0.05));
    }

    #[test]
    fn scope_defaults_to_paragraphs_only() {
        let doc = Document::markdown("段落。\n\n- 項目\n\n> 引用。\n\n# 見出し\n");
        let scope = Scope::default();
        let accepted: Vec<_> = doc
            .blocks
            .iter()
            .filter(|b| scope.accepts(b))
            .map(|b| b.text.as_str())
            .collect();
        assert_eq!(accepted, vec!["段落。"]);
        let all: Vec<_> = doc
            .blocks
            .iter()
            .filter(|b| Scope::ALL.accepts(b))
            .map(|b| b.text.as_str())
            .collect();
        assert_eq!(all, vec!["段落。", "項目", "引用。"]);
    }
}
