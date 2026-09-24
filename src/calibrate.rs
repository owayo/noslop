//! コーパスでルールの閾値を校正する (`noslop calibrate`)。
//!
//! 人の書いた文書 (人の群) と生成された文書 (生成の群) を同じ設定で検査し、ルールごとに
//! 人の文書で指摘された割合 (誤検知率) と、生成文書で指摘された割合 (検出率) を求める。
//! 率は重大度を問わないものに加えて、重大度ごとの内訳と、警告以上・重大だけで数えたものも出し、
//! 誤検知率には片側 95% の上限 (Wilson のスコア区間) を添える。
//!
//! 校正済みのルールの見直しは、ルールごとの校正の基準 ([`Rule::calibration_basis`]) の重大度
//! 以上の指摘で判定する。確度の低い語句を意図して情報にしたルールが、情報の指摘だけで
//! 「校正が成り立っていない」と判定されないためである。情報も利用者の画面に出るので、
//! 重大度を問わないと目標を超えるルールは、別の理由 (軽い指摘が多い) で挙げる。
//! 実験的なルールと語句の昇格は、重大度を問わない率で判定する。
//!
//! 閾値を持つルール ([`Rule::measure`] を実装したもの) は、閾値を動かしたときの率も求め、
//! 誤検知率が目標以下のまま検出率が最大になる閾値を提案する。
//!
//! 閾値は校正用の文書だけで選び、取り分けておいた検証用の文書で確かめる (選んだ文書で
//! そのまま測ると、率を良く見積もりすぎるため)。どちらに回すかは、入力に指定したパスからの
//! 相対パスのハッシュで決める。何度実行しても同じ分け方になり、同じ相対パスの文書は人と
//! 生成で同じ側に入る (お題をそろえたコーパスは、お題ごと同じ側に入る)。
//!
//! 出した率は与えたコーパスでの値にすぎない。校正済みへの昇格や閾値の変更は、この結果を
//! 見て人が判断する (手順は `docs/calibration.md`)。
//!
//! [`Rule::measure`]: crate::rules::Rule::measure
//! [`Rule::calibration_basis`]: crate::rules::Rule::calibration_basis

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;
use unicode_width::UnicodeWidthStr;

use crate::config::ConfigError;
use crate::diagnostic::{Diagnostic, Lane, Metric, RuleStatus, Severity};
use crate::document::{Document, SourceFormat};
use crate::engine::{Engine, EngineOptions};
use crate::rules::{Fires, Measure, RuleContext, RuleMeta};
use crate::walk::{self, WalkOptions};

/// JSON 出力のスキーマの版。互換性のない変更をしたら上げる。
pub const SCHEMA_VERSION: u32 = 1;
/// 誤検知率の目標の既定値。
pub const DEFAULT_TARGET_FP: f64 = 0.05;
/// 検証用に取り分ける文書の割合の既定値。
pub const DEFAULT_HOLDOUT: f64 = 0.3;
/// 校正済みに上げる候補とする検出率の下限の既定値。
pub const DEFAULT_MIN_DETECTION: f64 = 0.2;
/// 1 群の文書がこれより少ないと警告する。
pub const MIN_DOCS_PER_GROUP: usize = 10;
/// 検証用に回った文書が 1 群でこれより少ないと警告する。
const MIN_HOLDOUT_DOCS: usize = 5;
/// 語句の項目を昇格の候補にするのに要る、その項目が出た生成文書の数。
const MIN_ITEM_AI_DOCS: usize = 3;
/// 語句の項目を昇格の候補にするのに要る、生成文書での割合の人の文書での割合に対する倍率。
const MIN_ITEM_LIFT: f64 = 2.0;
/// 見直しの候補に添える、人の文書で多く出た項目の数。
const REVIEW_TOP_ITEMS: usize = 3;
/// 語句の項目に添える、よく出た表記の数。
const ITEM_EXAMPLES: usize = 3;
/// 片側 95% の上限に使う、標準正規分布の上側 5% 点。
const Z_95: f64 = 1.6449;
/// 率を目標と比べるときの許容誤差 (割り算の丸めで目標ちょうどを外さないため)。
const EPSILON: f64 = 1e-9;

// ---------------------------------------------------------------------------
// 条件
// ---------------------------------------------------------------------------

/// `noslop calibrate` の条件。
#[derive(Clone)]
pub struct CalibrateOptions {
    /// 人の書いた文書 (ファイルかディレクトリ)。
    pub human: Vec<PathBuf>,
    /// 生成された文書 (ファイルかディレクトリ)。
    pub ai: Vec<PathBuf>,
    /// エンジンの設定 (ジャンル・設定ファイルのルール設定・範囲など)。
    ///
    /// `experimental` は見ない。既定の動作はいつも `experimental = false` で測り、
    /// `include_experimental` なら実験的な動作も別に測る。
    pub engine: EngineOptions,
    /// ディレクトリから集めるファイルの条件。
    pub walk: WalkOptions,
    /// 実験的なルールと語句も測るか。
    pub include_experimental: bool,
    /// 許容する誤検知率 (0 以上 1 以下)。
    pub target_fp: f64,
    /// 検証用に取り分ける文書の割合 (0 以上 1 未満)。
    pub holdout: f64,
    /// 校正済みに上げる候補とする検出率の下限 (0 以上 1 以下)。
    pub min_detection: f64,
}

impl CalibrateOptions {
    /// 既定の条件 (一般のジャンル、実験的なものも測る、目標 5%、検証 30%)。
    pub fn new(human: Vec<PathBuf>, ai: Vec<PathBuf>) -> Self {
        Self {
            human,
            ai,
            engine: EngineOptions::default(),
            walk: WalkOptions::default(),
            include_experimental: true,
            target_fp: DEFAULT_TARGET_FP,
            holdout: DEFAULT_HOLDOUT,
            min_detection: DEFAULT_MIN_DETECTION,
        }
    }
}

// ---------------------------------------------------------------------------
// 結果
// ---------------------------------------------------------------------------

/// 文書の群。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    /// 人の書いた文書。ここでの指摘を誤検知とみなす。
    Human,
    /// 生成された文書。ここでの指摘を検出とみなす。
    Ai,
}

impl Group {
    /// 短いラベル (「人」「生成」)。
    pub fn label_ja(self) -> &'static str {
        match self {
            Group::Human => "人",
            Group::Ai => "生成",
        }
    }

    /// 文中で使う呼び名 (「人の文書」「生成文書」)。
    fn docs_ja(self) -> &'static str {
        match self {
            Group::Human => "人の文書",
            Group::Ai => "生成文書",
        }
    }
}

/// ルールをどちらの動作で測ったか。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// 既定の動作 (`--experimental` なし) で動くルール。
    Default,
    /// 既定では動かず、実験的な動作 (`--experimental` あり) で測ったルール。
    Experimental,
}

/// 校正の結果。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationReport {
    pub schema_version: u32,
    pub settings: Settings,
    pub corpus: Corpus,
    /// ルール別の反応 (全文書)。エンジンのルールの順。
    pub rules: Vec<RuleStats>,
    /// 閾値の掃引 (ルールと設定キーごと)。
    pub thresholds: Vec<ThresholdSweep>,
    /// 語句ルールの項目ごとの反応。
    pub items: Vec<ItemStats>,
    /// 校正済みに上げる候補。
    pub promotions: Vec<Promotion>,
    /// 見直しが必要な校正済みルール。
    pub reviews: Vec<Review>,
    pub warnings: Vec<String>,
    /// 読めなかった入力。
    pub errors: Vec<CorpusError>,
}

/// 校正の条件 (出力に添える)。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub genre: &'static str,
    pub target_fp: f64,
    pub holdout: f64,
    pub min_detection: f64,
    pub include_experimental: bool,
}

/// 群ごとの文書の数。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Corpus {
    pub human: GroupSize,
    pub ai: GroupSize,
}

/// 1 群の文書の数と文字数。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupSize {
    pub docs: usize,
    /// 校正用に回った文書の数。
    pub calibration: usize,
    /// 検証用に回った文書の数。
    pub holdout: usize,
    /// 原文の文字数の合計。
    pub characters: usize,
}

/// ある群での、ルール (または語句の項目) の反応。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Rate {
    pub docs: usize,
    /// 1 件以上指摘された文書の数 (重大度を問わない)。
    pub fired: usize,
    /// 指摘の件数 (重大度を問わない)。
    pub hits: usize,
    pub characters: usize,
    /// 指摘された文書の割合 (重大度を問わない)。文書がなければ `None`。
    pub rate: Option<f64>,
    /// `rate` の片側 95% 上限 (Wilson のスコア区間)。文書がなければ `None`。
    pub upper95: Option<f64>,
    /// 1000 字あたりの指摘の件数。
    pub per_1000_chars: Option<f64>,
    /// 重大度ごとの内訳。件数 (`hits`) は重大度ごとに分かれ、文書の数 (`fired`) は同じ文書が
    /// 複数の重大度に入りうる。
    pub by_severity: BySeverity,
    /// 下限の重大度以上の指摘があった文書の割合 (文書の数は重複を除く)。
    pub at_least: AtLeast,
}

impl Rate {
    /// `severity` 以上の指摘があった文書の数 (`Info` なら重大度を問わない)。
    pub fn fired_at_least(&self, severity: Severity) -> usize {
        match severity {
            Severity::Info => self.fired,
            Severity::Warning => self.at_least.warning.fired,
            Severity::Error => self.at_least.error.fired,
        }
    }

    /// `severity` 以上の指摘があった文書の割合。
    pub fn rate_at_least(&self, severity: Severity) -> Option<f64> {
        ratio(self.fired_at_least(severity), self.docs)
    }
}

/// 重大度ごとの内訳。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct BySeverity {
    pub info: SeverityCount,
    pub warning: SeverityCount,
    pub error: SeverityCount,
}

impl BySeverity {
    pub fn get(&self, severity: Severity) -> SeverityCount {
        match severity {
            Severity::Info => self.info,
            Severity::Warning => self.warning,
            Severity::Error => self.error,
        }
    }
}

/// 1 つの重大度の指摘の数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct SeverityCount {
    /// その重大度の指摘が 1 件以上あった文書の数。
    pub fired: usize,
    /// その重大度の指摘の件数。
    pub hits: usize,
}

/// 下限の重大度ごとの文書率。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct AtLeast {
    /// 警告か重大の指摘が 1 件以上あった文書。
    pub warning: DocRate,
    /// 重大の指摘が 1 件以上あった文書。
    pub error: DocRate,
}

/// 文書の数と割合、その片側 95% 上限。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocRate {
    pub fired: usize,
    pub rate: Option<f64>,
    pub upper95: Option<f64>,
}

impl DocRate {
    fn new(fired: usize, docs: usize) -> Self {
        Self {
            fired,
            rate: ratio(fired, docs),
            upper95: wilson_upper(fired, docs),
        }
    }
}

/// ルール 1 つの反応。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleStats {
    pub id: &'static str,
    pub name: &'static str,
    pub title: &'static str,
    pub lane: Lane,
    pub status: RuleStatus,
    pub mode: Mode,
    /// 校正の基準: 誤検知率の目標を当てはめる重大度 (見直しはこの重大度以上の指摘で判定する)。
    pub calibration_basis: Severity,
    pub human: Rate,
    pub ai: Rate,
}

impl RuleStats {
    /// 重大度を問わない、全文書での誤検知率と検出率。
    pub fn outcome(&self) -> Outcome {
        self.outcome_at(Severity::Info)
    }

    /// `severity` 以上の指摘だけで数えた、全文書での誤検知率と検出率。
    pub fn outcome_at(&self, severity: Severity) -> Outcome {
        Outcome::new(
            severity,
            self.human.fired_at_least(severity),
            self.human.docs,
            self.ai.fired_at_least(severity),
            self.ai.docs,
        )
    }
}

/// ある閾値・ある文書の範囲での、誤検知率と検出率。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    /// この重大度以上の指摘があった文書を「指摘された」と数えた (`info` なら重大度を問わない)。
    pub at_least: Severity,
    pub human_fired: usize,
    pub human_docs: usize,
    pub ai_fired: usize,
    pub ai_docs: usize,
    /// 人の文書で指摘された割合。
    pub false_positive: Option<f64>,
    /// 誤検知率の片側 95% 上限 (Wilson のスコア区間)。人の文書がなければ `None`。
    pub false_positive_upper95: Option<f64>,
    /// 生成文書で指摘された割合。
    pub detection: Option<f64>,
}

impl Outcome {
    fn new(
        at_least: Severity,
        human_fired: usize,
        human_docs: usize,
        ai_fired: usize,
        ai_docs: usize,
    ) -> Self {
        Self {
            at_least,
            human_fired,
            human_docs,
            ai_fired,
            ai_docs,
            false_positive: ratio(human_fired, human_docs),
            false_positive_upper95: wilson_upper(human_fired, human_docs),
            detection: ratio(ai_fired, ai_docs),
        }
    }

    /// 誤検知率が目標以下で、生成文書を 1 件以上拾えているか (閾値を提案する条件)。
    fn usable(&self, target_fp: f64) -> bool {
        self.ai_fired > 0
            && self.detection.is_some()
            && self
                .false_positive
                .is_some_and(|fp| fp <= target_fp + EPSILON)
    }

    /// 校正済みに上げる条件 (誤検知率が目標以下、検出率が下限以上) を満たすか。
    fn promotable(&self, target_fp: f64, min_detection: f64) -> bool {
        self.usable(target_fp) && self.detection.is_some_and(|d| d + EPSILON >= min_detection)
    }

    /// 誤検知率の 95% 上限まで目標以下か (件数が少なくても目標以下と言い切れるか)。
    fn confirmed(&self, target_fp: f64) -> bool {
        self.false_positive_upper95
            .is_some_and(|u| u <= target_fp + EPSILON)
    }
}

/// ルール 1 つ・設定キー 1 つの閾値の掃引。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThresholdSweep {
    pub rule_id: &'static str,
    pub rule_name: &'static str,
    pub lane: Lane,
    pub status: RuleStatus,
    /// 測った量の名前。
    pub measure: &'static str,
    /// 閾値の設定キー (`[rules.<ID>]` に書くキー)。
    pub key: &'static str,
    /// 閾値と比べる向き (`below` / `atOrBelow` / `above` / `atOrAbove`)。
    pub fires: &'static str,
    #[serde(skip)]
    pub direction: Fires,
    /// この閾値が決める重大度の切り替え。`None` なら指摘するかどうかを決める閾値。
    /// `Some` なら、率はその重大度以上の指摘があった文書の割合。
    pub switches_to: Option<Severity>,
    /// 値を測れた人の文書の数 (全文書)。測れない文書はこのキーでは指摘されないものとして数える。
    pub measured_human: usize,
    /// 値を測れた生成文書の数 (全文書)。
    pub measured_ai: usize,
    /// 現在の閾値での率。設定値を数値として読めなければ `None`。
    pub current: Option<ThresholdPoint>,
    /// 校正用の文書で選んだ閾値。条件を満たす閾値がなければ `None`。
    pub suggested: Option<ThresholdPoint>,
}

/// 閾値 1 つでの率 (校正用・検証用・全文書)。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThresholdPoint {
    pub threshold: f64,
    pub calibration: Outcome,
    pub holdout: Outcome,
    pub all: Outcome,
}

/// 語句ルールの項目 1 つの反応 (全文書)。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemStats {
    pub rule_id: &'static str,
    pub rule_name: &'static str,
    /// 項目。辞書のリテラルはその文字列、正規表現は `/パターン/`。辞書の項目を持たないルール
    /// (構文の型で拾うもの) は一致した文字列。
    pub item: String,
    /// よく出た表記 (件数の多い順に最大 3 つ)。
    pub examples: Vec<String>,
    pub status: RuleStatus,
    /// 項目の重大度 (辞書で宣言した重大度。設定で上書きしていればその値)。
    pub severity: Severity,
    /// 人の文書での反応 (文書の数は人の文書全体)。
    pub human: Rate,
    /// 生成文書での反応。
    pub ai: Rate,
    /// 実験的な項目を校正済みに上げたときの、ルール全体の率 (全文書・重大度を問わない)。
    /// 校正済みのルールにある実験的な項目だけに付ける。
    pub promoted_rule: Option<Outcome>,
}

/// 昇格・見直しに添える項目。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemSummary {
    pub item: String,
    pub examples: Vec<String>,
    pub severity: Severity,
    /// 項目が出た人の文書の数。
    pub human_docs: usize,
    /// 項目が出た生成文書の数。
    pub ai_docs: usize,
}

impl ItemSummary {
    fn of(item: &ItemStats) -> Self {
        Self {
            item: item.item.clone(),
            examples: item.examples.clone(),
            severity: item.severity,
            human_docs: item.human.fired,
            ai_docs: item.ai.fired,
        }
    }
}

/// 校正済みに上げる候補。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Promotion {
    pub rule_id: &'static str,
    pub rule_name: &'static str,
    /// 語句の項目を上げる候補なら、その項目。
    pub item: Option<ItemSummary>,
    /// 閾値を変えれば条件を満たす候補なら、その変更。
    pub threshold: Option<ThresholdChange>,
    /// 判断に使った率 (重大度を問わない。項目なら、上げた後のルール全体の率)。
    pub outcome: Outcome,
    /// 率を測った文書の範囲。
    pub basis: Basis,
    /// 件数が少なく、誤検知率の 95% 上限が目標を超える (目標以下と言い切れない)。
    pub small_sample: bool,
}

/// 閾値の変更の提案。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThresholdChange {
    pub key: &'static str,
    pub from: Option<f64>,
    pub to: f64,
}

/// 率を測った文書の範囲。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Basis {
    /// 全文書 (閾値を選んでいないので、分けずに全部で測る)。
    All,
    /// 検証用の文書 (閾値を校正用の文書で選んだので、選ぶのに使っていない文書で測る)。
    Holdout,
}

/// 見直しが必要な校正済みルール。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    pub rule_id: &'static str,
    pub rule_name: &'static str,
    pub reason: ReviewReason,
    /// 校正の基準 (見直しの判定に使った重大度)。
    pub calibration_basis: Severity,
    /// 基準の重大度以上の指摘で数えた、全文書での率。
    pub outcome: Outcome,
    /// 重大度を問わない、全文書での率。
    pub any_severity: Outcome,
    /// 人の文書で多く出た項目 (語句ルールのみ)。見直しの理由に関わる重大度の項目に限る
    /// (軽い指摘が多いなら基準より軽い項目、それ以外は基準以上の項目)。
    pub human_items: Vec<ItemSummary>,
}

/// 見直しの理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ReviewReason {
    /// 基準の重大度以上の指摘で数えても、誤検知率が目標を超えている (校正が成り立っていない)。
    FalsePositives,
    /// 基準の重大度以上の指摘で、生成文書での割合が人の文書を上回っていない
    /// (指摘はあるのに見分けられていない)。
    NoSeparation,
    /// 基準の重大度では目標以下だが、重大度を問わないと誤検知率が目標を超える
    /// (基準が情報より重いルールだけ)。直ちに実験的に戻す話ではなく、原因の項目の重大度・除外・
    /// 説明を見直す材料。
    InfoBurden,
}

impl ReviewReason {
    const ALL: [ReviewReason; 3] = [
        ReviewReason::FalsePositives,
        ReviewReason::NoSeparation,
        ReviewReason::InfoBurden,
    ];

    /// 見出し (理由の要約と、判定の条件)。
    fn heading(self) -> &'static str {
        match self {
            ReviewReason::FalsePositives => {
                "誤検知が目標を超える (基準の重大度以上で数えても目標を超え、校正が成り立っていない)"
            }
            ReviewReason::NoSeparation => {
                "見分けられていない (基準の重大度以上で数えると、生成文書で指摘される割合が人の文書を上回らない)"
            }
            ReviewReason::InfoBurden => {
                "軽い指摘が多い (基準の重大度では目標以下だが、重大度を問わないと目標を超える。直ちに実験的に戻す話ではなく、原因の項目の重大度・除外・説明を見直す材料)"
            }
        }
    }
}

/// 読めなかった入力 (ファイル、または 1 件もファイルを集められなかったディレクトリ)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CorpusError {
    pub group: Group,
    pub path: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// 集計
// ---------------------------------------------------------------------------

/// コーパスを検査して、ルールごとの率と閾値の提案をまとめる。
///
/// 読めないファイルは結果の `errors` に入れ、他のファイルは処理を続ける。
/// 条件の誤りと、エンジンの設定の誤り (未知のルールなど) だけをエラーとして返す。
pub fn calibrate(options: &CalibrateOptions) -> Result<CalibrationReport, ConfigError> {
    validate(options)?;
    let base = Engine::new(EngineOptions {
        experimental: false,
        ..options.engine.clone()
    })?;
    let full = if options.include_experimental {
        Some(Engine::new(EngineOptions {
            experimental: true,
            ..options.engine.clone()
        })?)
    } else {
        None
    };
    let evaluated = evaluated_rules(&base, full.as_ref());
    let index: HashMap<&'static str, usize> = evaluated
        .iter()
        .enumerate()
        .map(|(i, ev)| (ev.meta.id, i))
        .collect();

    let mut errors = Vec::new();
    let mut sources = Vec::new();
    collect_sources(
        Group::Human,
        &options.human,
        &options.walk,
        &mut sources,
        &mut errors,
    );
    collect_sources(
        Group::Ai,
        &options.ai,
        &options.walk,
        &mut sources,
        &mut errors,
    );

    let scan = Scan {
        base: &base,
        full: full.as_ref(),
        evaluated: &evaluated,
        index: &index,
        holdout: options.holdout,
    };
    let results: Vec<Result<DocResult, CorpusError>> =
        sources.into_par_iter().map(|s| scan.doc(s)).collect();
    let mut docs = Vec::with_capacity(results.len());
    for result in results {
        match result {
            Ok(doc) => docs.push(doc),
            Err(e) => errors.push(e),
        }
    }
    errors.sort_by(|a, b| (a.group == Group::Ai, &a.path).cmp(&(b.group == Group::Ai, &b.path)));
    Ok(build_report(options, &evaluated, &docs, errors))
}

fn validate(options: &CalibrateOptions) -> Result<(), ConfigError> {
    let invalid = |message: &str| Err(ConfigError::Invalid(message.to_string()));
    if options.human.is_empty() {
        return invalid("人の文書 (--human) を 1 つ以上指定してください");
    }
    if options.ai.is_empty() {
        return invalid("生成文書 (--ai) を 1 つ以上指定してください");
    }
    if !(0.0..=1.0).contains(&options.target_fp) {
        return invalid("--target-fp には 0 以上 1 以下の値を指定してください");
    }
    if !(0.0..1.0).contains(&options.holdout) {
        return invalid("--holdout には 0 以上 1 未満の値を指定してください");
    }
    if !(0.0..=1.0).contains(&options.min_detection) {
        return invalid("--min-detection には 0 以上 1 以下の値を指定してください");
    }
    Ok(())
}

/// 測るルール 1 つ。
struct Evaluated {
    /// エンジンのルール一覧での添字 (2 つのエンジンで共通)。
    entry: usize,
    meta: &'static RuleMeta,
    mode: Mode,
    builtin: bool,
    /// 数値として読める設定項目の現在値 (`Rule::options` の順)。
    options: Vec<(&'static str, f64)>,
    /// 校正の基準 (`Rule::calibration_basis`)。
    basis: Severity,
}

impl Evaluated {
    /// 昇格・見直しの対象か (組み込みの、AI 臭さのレーンのルール)。
    fn judged(&self) -> bool {
        self.builtin && self.meta.lane == Lane::Slop
    }

    fn current(&self, key: &str) -> Option<f64> {
        self.options
            .iter()
            .find(|(k, _)| *k == key)
            .map(|&(_, v)| v)
    }
}

/// 既定の動作で動くルールは既定の動作で、そうでないルールは実験的な動作で測る。
fn evaluated_rules(base: &Engine, full: Option<&Engine>) -> Vec<Evaluated> {
    let mut out = Vec::new();
    for (i, entry) in base.entries().iter().enumerate() {
        let full_entry = full.map(|f| &f.entries()[i]);
        if let Some(f) = full_entry {
            debug_assert_eq!(entry.rule.meta().id, f.rule.meta().id);
        }
        let (mode, rule) = if entry.enabled {
            (Mode::Default, &entry.rule)
        } else if let Some(f) = full_entry.filter(|f| f.enabled) {
            (Mode::Experimental, &f.rule)
        } else {
            continue;
        };
        out.push(Evaluated {
            entry: i,
            meta: rule.meta(),
            mode,
            builtin: entry.builtin,
            options: rule
                .options()
                .into_iter()
                .filter_map(|(k, v)| v.parse::<f64>().ok().map(|v| (k, v)))
                .collect(),
            basis: rule.calibration_basis(),
        });
    }
    out
}

/// 検査する文書 1 つ。
struct Source {
    group: Group,
    path: PathBuf,
    /// 校正用と検証用の分け方を決めるキー。
    key: String,
}

fn collect_sources(
    group: Group,
    roots: &[PathBuf],
    walk_options: &WalkOptions,
    sources: &mut Vec<Source>,
    errors: &mut Vec<CorpusError>,
) {
    let mut seen = HashSet::new();
    for root in roots {
        let collected = walk::collect(std::slice::from_ref(root), walk_options);
        // ディレクトリから 1 件も集まらないのは、除外の書き方の誤りであることが多い
        // (`/corpus/**` のようにファイルに当たる .gitignore の行は、コーパスの中身も除外する)。
        // `check` では警告に留めるが、校正では群の文書が欠けるのでエラーにする
        errors.extend(collected.empty_dirs.into_iter().map(|path| CorpusError {
            group,
            path,
            message: "検査できるファイルがありません (拡張子と、.gitignore などの除外の指定を確かめてください)"
                .to_string(),
        }));
        errors.extend(collected.errors.into_iter().map(|e| CorpusError {
            group,
            path: e.path,
            message: e.message,
        }));
        for path in collected.files {
            if seen.insert(path.clone()) {
                let key = relative_key(root, &path);
                sources.push(Source { group, path, key });
            }
        }
    }
}

/// 分け方を決めるキー: 入力に指定したパスからの相対パス (ファイルを直接指定したならファイル名)。
///
/// 絶対パスを使わないので、コーパスを置く場所を変えても同じ分け方になる。
fn relative_key(root: &Path, file: &Path) -> String {
    let cleaned = root.strip_prefix(".").unwrap_or(root);
    let relative = file
        .strip_prefix(root)
        .or_else(|_| file.strip_prefix(cleaned))
        .ok()
        .filter(|p| !p.as_os_str().is_empty());
    match relative {
        Some(p) => walk::display(p),
        None => file
            .file_name()
            .map_or_else(|| walk::display(file), |n| n.to_string_lossy().into_owned()),
    }
}

/// キーを [0, 1) の値に写す (FNV-1a に攪拌をかけたもの)。
fn split_point(key: &str) -> f64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    // 短いキーでも上位のビットがよく混ざるよう、MurmurHash3 の最後の攪拌をかける
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xff51_afd7_ed55_8ccd);
    hash ^= hash >> 33;
    hash = hash.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    hash ^= hash >> 33;
    (hash >> 11) as f64 / (1u64 << 53) as f64
}

/// 文書を校正用と検証用のどちらに回したか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Split {
    Calibration,
    Holdout,
}

/// 率を求める文書の範囲。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subset {
    All,
    Calibration,
    Holdout,
}

impl Subset {
    fn contains(self, split: Split) -> bool {
        match self {
            Subset::All => true,
            Subset::Calibration => split == Split::Calibration,
            Subset::Holdout => split == Split::Holdout,
        }
    }
}

/// 重大度の並び (軽い順)。
const SEVERITIES: [Severity; 3] = [Severity::Info, Severity::Warning, Severity::Error];

/// 重大度ごとの配列の添字。
fn slot(severity: Severity) -> usize {
    match severity {
        Severity::Info => 0,
        Severity::Warning => 1,
        Severity::Error => 2,
    }
}

/// 1 文書での、重大度ごとの指摘の件数 (情報・警告・重大の順)。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Hits([usize; 3]);

impl Hits {
    fn add(&mut self, severity: Severity) {
        self.0[slot(severity)] += 1;
    }

    fn total(&self) -> usize {
        self.0.iter().sum()
    }

    /// `severity` 以上の件数。
    fn at_least(&self, severity: Severity) -> usize {
        self.0[slot(severity)..].iter().sum()
    }

    /// 件数のある重大度のうち、最も重いもの。
    fn highest(&self) -> Option<Severity> {
        SEVERITIES.into_iter().rev().find(|&s| self.0[slot(s)] > 0)
    }
}

/// 文書ごとの件数を足し合わせて、率を組み立てる。
#[derive(Debug, Clone, Copy, Default)]
struct Tally {
    hits: Hits,
    /// その重大度の指摘があった文書の数。
    fired_with: [usize; 3],
    /// その重大度以上の指摘があった文書の数。
    fired_at_least: [usize; 3],
}

impl Tally {
    fn add(&mut self, doc: &Hits) {
        for s in SEVERITIES {
            let i = slot(s);
            self.hits.0[i] += doc.0[i];
            self.fired_with[i] += usize::from(doc.0[i] > 0);
            self.fired_at_least[i] += usize::from(doc.at_least(s) > 0);
        }
    }

    /// 文書の数が `docs`、文字数が `characters` の群での率。
    fn rate(&self, docs: usize, characters: usize) -> Rate {
        let fired = self.fired_at_least[slot(Severity::Info)];
        let hits = self.hits.total();
        let count = |s: Severity| SeverityCount {
            fired: self.fired_with[slot(s)],
            hits: self.hits.0[slot(s)],
        };
        let floor = |s: Severity| DocRate::new(self.fired_at_least[slot(s)], docs);
        Rate {
            docs,
            fired,
            hits,
            characters,
            rate: ratio(fired, docs),
            upper95: wilson_upper(fired, docs),
            per_1000_chars: (characters > 0).then(|| hits as f64 * 1000.0 / characters as f64),
            by_severity: BySeverity {
                info: count(Severity::Info),
                warning: count(Severity::Warning),
                error: count(Severity::Error),
            },
            at_least: AtLeast {
                warning: floor(Severity::Warning),
                error: floor(Severity::Error),
            },
        }
    }
}

/// 語句の項目: (測るルールの添字, 項目, 項目の校正状況)。
type ItemKey = (usize, String, RuleStatus);

/// 1 文書での項目の出方。
#[derive(Debug, Clone, Default)]
struct ItemHit {
    hits: Hits,
    /// 一致した表記ごとの件数。
    notations: HashMap<String, usize>,
}

/// 1 文書の結果。
struct DocResult {
    group: Group,
    split: Split,
    characters: usize,
    /// 段落のあいだに空行がないように見える Markdown の文書か ([`looks_line_per_paragraph`])。
    line_per_paragraph: bool,
    /// 測るルールごとの、重大度ごとの指摘の件数 (抑制コメントで残したものも数える)。
    hits: Vec<Hits>,
    /// 測るルールごとの測定値。
    measures: Vec<Vec<Measure>>,
    items: HashMap<ItemKey, ItemHit>,
}

/// 文書を 1 つずつ測る道具一式。
struct Scan<'a> {
    base: &'a Engine,
    full: Option<&'a Engine>,
    evaluated: &'a [Evaluated],
    /// ルール ID → 測るルールの添字。
    index: &'a HashMap<&'static str, usize>,
    holdout: f64,
}

impl Scan<'_> {
    fn engine(&self, mode: Mode) -> &Engine {
        match (mode, self.full) {
            (Mode::Experimental, Some(full)) => full,
            _ => self.base,
        }
    }

    fn doc(&self, source: Source) -> Result<DocResult, CorpusError> {
        let name = walk::display(&source.path);
        let fail = |message: String| CorpusError {
            group: source.group,
            path: name.clone(),
            message,
        };
        let bytes =
            std::fs::read(&source.path).map_err(|e| fail(format!("読み込めません: {e}")))?;
        let text = String::from_utf8(bytes).map_err(|_| {
            fail("UTF-8 として読めません (文字コードを UTF-8 にしてください)".to_string())
        })?;
        let doc = Document::parse(
            name.clone(),
            text,
            SourceFormat::from_path(&source.path),
            &self.base.options().parse,
        );
        let characters = doc.char_count();
        let line_per_paragraph = looks_line_per_paragraph(&doc);
        let measures: Vec<Vec<Measure>> = self
            .evaluated
            .iter()
            .map(|ev| {
                let engine = self.engine(ev.mode);
                let o = engine.options();
                let morph = engine.doc_morphology(&doc);
                let ctx = RuleContext {
                    doc: &doc,
                    genre: o.genre,
                    scope: o.scope,
                    experimental: o.experimental,
                    morph: morph.as_ref(),
                };
                engine.entries()[ev.entry].rule.measure(&ctx)
            })
            .collect();

        // 抑制コメントで残した指摘も数える (検出器そのものの反応を測るため)
        let (base_diags, full_diags) = match self.full {
            Some(full) => (
                self.base.lint(doc.clone()).diagnostics,
                full.lint(doc).diagnostics,
            ),
            None => (self.base.lint(doc).diagnostics, Vec::new()),
        };
        let mut hits = vec![Hits::default(); self.evaluated.len()];
        let mut items = HashMap::new();
        for d in &base_diags {
            let Some(&r) = self.index.get(d.rule_id.as_str()) else {
                continue;
            };
            if self.evaluated[r].mode == Mode::Default {
                hits[r].add(d.severity);
                record_item(&mut items, r, d);
            }
        }
        for d in &full_diags {
            let Some(&r) = self.index.get(d.rule_id.as_str()) else {
                continue;
            };
            let ev = &self.evaluated[r];
            match ev.mode {
                Mode::Experimental => {
                    hits[r].add(d.severity);
                    record_item(&mut items, r, d);
                }
                // 校正済みのルールにある実験的な項目は、実験的な動作でだけ出る
                Mode::Default => {
                    if ev.meta.status == RuleStatus::Stable && d.status == RuleStatus::Experimental
                    {
                        record_item(&mut items, r, d);
                    }
                }
            }
        }
        let split = if split_point(&source.key) < self.holdout {
            Split::Holdout
        } else {
            Split::Calibration
        };
        Ok(DocResult {
            group: source.group,
            split,
            characters,
            line_per_paragraph,
            hits,
            measures,
            items,
        })
    }
}

/// 段落のあいだに空行がないように見える Markdown の文書か。
///
/// 1 行 1 段落で書いた文書を Markdown として読むと、空行のない行は 1 つの段落にまとまり、
/// 段落を単位に見るルール (R07・R09 など) が測れなくなる。文末 (句点・閉じ括弧) の直後の改行が
/// 10 か所以上あり、それが段落の中の改行の 8 割以上を占める地の文の段落があれば、そう見なす
/// (1 文ずつ改行して段落のあいだに空行を置く書き方では、1 段落にこれほど文末の改行が並ばない)。
fn looks_line_per_paragraph(doc: &Document) -> bool {
    if doc.format != SourceFormat::Markdown {
        return false;
    }
    doc.blocks.iter().filter(|b| b.is_prose()).any(|b| {
        let after_ender = b
            .line_breaks
            .iter()
            .filter(|&&pos| {
                b.text
                    .get(..pos)
                    .and_then(|before| before.trim_end().chars().next_back())
                    .is_some_and(|c| {
                        crate::text::is_sentence_ender(c) || crate::text::is_closing_bracket(c)
                    })
            })
            .count();
        after_ender >= 10 && after_ender * 5 >= b.line_breaks.len() * 4
    })
}

/// 診断の metrics の文字列の値。
fn text_metric<'d>(d: &'d Diagnostic, key: &str) -> Option<&'d String> {
    match d.metrics.get(key) {
        Some(Metric::Text(s)) => Some(s),
        _ => None,
    }
}

/// 語句ルールの指摘 (一致した表記を持つもの) を、項目ごとに数える。
///
/// 項目は metrics の `item` (辞書の項目の名前)。`item` を持たない指摘 (独自ルールなど) は、
/// 一致した表記を項目とする。
fn record_item(items: &mut HashMap<ItemKey, ItemHit>, rule: usize, d: &Diagnostic) {
    let Some(matched) = text_metric(d, "matched") else {
        return;
    };
    let item = text_metric(d, "item").unwrap_or(matched);
    let hit = items.entry((rule, item.clone(), d.status)).or_default();
    hit.hits.add(d.severity);
    *hit.notations.entry(matched.clone()).or_default() += 1;
}

fn build_report(
    options: &CalibrateOptions,
    evaluated: &[Evaluated],
    docs: &[DocResult],
    errors: Vec<CorpusError>,
) -> CalibrationReport {
    let corpus = Corpus {
        human: group_size(docs, Group::Human),
        ai: group_size(docs, Group::Ai),
    };
    let rules: Vec<RuleStats> = evaluated
        .iter()
        .enumerate()
        .map(|(r, ev)| RuleStats {
            id: ev.meta.id,
            name: ev.meta.name,
            title: ev.meta.title,
            lane: ev.meta.lane,
            status: ev.meta.status,
            mode: ev.mode,
            calibration_basis: ev.basis,
            human: rule_rate(docs, Group::Human, &corpus.human, r),
            ai: rule_rate(docs, Group::Ai, &corpus.ai, r),
        })
        .collect();
    let thresholds: Vec<ThresholdSweep> = evaluated
        .iter()
        .enumerate()
        .flat_map(|(r, ev)| {
            sweep_keys(docs, r, ev)
                .into_iter()
                .map(move |key| sweep(docs, r, ev, key, options.target_fp))
        })
        .collect();
    let items = item_stats(evaluated, docs, &rules, &corpus);
    let promotions = promotions(options, evaluated, &rules, &thresholds, &items);
    let reviews = reviews(options, evaluated, &rules, &items);
    let warnings = warnings(options, &corpus, docs);
    CalibrationReport {
        schema_version: SCHEMA_VERSION,
        settings: Settings {
            genre: options.engine.genre.as_str(),
            target_fp: options.target_fp,
            holdout: options.holdout,
            min_detection: options.min_detection,
            include_experimental: options.include_experimental,
        },
        corpus,
        rules,
        thresholds,
        items,
        promotions,
        reviews,
        warnings,
        errors,
    }
}

fn group_size(docs: &[DocResult], group: Group) -> GroupSize {
    let mut size = GroupSize::default();
    for d in docs.iter().filter(|d| d.group == group) {
        size.docs += 1;
        size.characters += d.characters;
        match d.split {
            Split::Calibration => size.calibration += 1,
            Split::Holdout => size.holdout += 1,
        }
    }
    size
}

fn rule_rate(docs: &[DocResult], group: Group, size: &GroupSize, rule: usize) -> Rate {
    let mut tally = Tally::default();
    for d in docs.iter().filter(|d| d.group == group) {
        tally.add(&d.hits[rule]);
    }
    tally.rate(size.docs, size.characters)
}

fn ratio(n: usize, d: usize) -> Option<f64> {
    (d > 0).then(|| n as f64 / d as f64)
}

/// 割合 `fired / docs` の片側 95% 上限 (Wilson のスコア区間)。文書がなければ `None`。
///
/// 件数が少ないときも 0 や 1 に張り付かず、「この件数ではここまでの率がありうる」を示す。
fn wilson_upper(fired: usize, docs: usize) -> Option<f64> {
    if docs == 0 {
        return None;
    }
    let n = docs as f64;
    let p = fired as f64 / n;
    let z2 = Z_95 * Z_95;
    let center = p + z2 / (2.0 * n);
    let margin = Z_95 * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt();
    Some(((center + margin) / (1.0 + z2 / n)).min(1.0))
}

/// 人の文書が 1 件も指摘されなくても、誤検知率の 95% 上限が `target` 以下になるのに要る
/// 文書の数。`target` が 0 以下なら、何件そろえても届かないので `None`。
///
/// 目標以下かの判定は、昇格の候補の注記 ([`Outcome::confirmed`]) と同じ許容誤差で行う
/// (警告の件数だけそろえた候補に注記が付かないように)。
fn required_docs(target: f64) -> Option<usize> {
    if target <= 0.0 {
        return None;
    }
    let within = |n: usize| wilson_upper(0, n).is_some_and(|u| u <= target + EPSILON);
    // 0 件のときの上限は z² / (n + z²) なので、n ≥ z² (1 − target) / target で必ず目標以下になる。
    // 許容誤差のぶん少ない件数で足りることがあるので、その範囲を二分探索する
    // (目標がごく小さいと件数が桁違いに大きくなるため、1 件ずつは探さない)
    let z2 = Z_95 * Z_95;
    let mut hi = ((z2 * (1.0 - target) / target).ceil().max(1.0)) as usize;
    while !within(hi) {
        if hi == usize::MAX {
            return None;
        }
        hi = hi.saturating_mul(2);
    }
    let mut lo = 1;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if within(mid) {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Some(lo)
}

/// `subset` の文書で、`fires` が真になる文書の割合 (`at_least` 以上の指摘として数えたもの)。
fn outcome(
    docs: &[DocResult],
    subset: Subset,
    at_least: Severity,
    fires: impl Fn(&DocResult) -> bool,
) -> Outcome {
    let (mut human_fired, mut human_docs, mut ai_fired, mut ai_docs) = (0, 0, 0, 0);
    for d in docs.iter().filter(|d| subset.contains(d.split)) {
        let fired = usize::from(fires(d));
        match d.group {
            Group::Human => {
                human_docs += 1;
                human_fired += fired;
            }
            Group::Ai => {
                ai_docs += 1;
                ai_fired += fired;
            }
        }
    }
    Outcome::new(at_least, human_fired, human_docs, ai_fired, ai_docs)
}

/// 掃引する設定キー 1 つ (設定キー, 測った量の名前, 比べる向き, 切り替える重大度)。
type SweepKey = (&'static str, &'static str, Fires, Option<Severity>);

/// ルールの測定値に出てくる設定キー。`Rule::options` の順に並べる。
fn sweep_keys(docs: &[DocResult], rule: usize, ev: &Evaluated) -> Vec<SweepKey> {
    let mut keys: Vec<SweepKey> = Vec::new();
    for m in docs.iter().flat_map(|d| &d.measures[rule]) {
        if !keys.iter().any(|(k, ..)| *k == m.threshold_key) {
            keys.push((m.threshold_key, m.name, m.fires, m.switches_to));
        }
    }
    let position = |key: &str| {
        ev.options
            .iter()
            .position(|(k, _)| *k == key)
            .unwrap_or(usize::MAX)
    };
    keys.sort_by_key(|(k, ..)| position(k));
    keys
}

/// 文書の測定値のうち、設定キー `key` と比べるもの。
fn of_key<'d>(
    d: &'d DocResult,
    rule: usize,
    key: &'static str,
) -> impl Iterator<Item = &'d Measure> + 'd {
    d.measures[rule]
        .iter()
        .filter(move |m| m.threshold_key == key)
}

fn sweep(
    docs: &[DocResult],
    rule: usize,
    ev: &Evaluated,
    (key, measure, direction, switches_to): SweepKey,
    target_fp: f64,
) -> ThresholdSweep {
    // 重大度の切り替えを決める閾値なら、率はその重大度以上の指摘があった文書の割合
    let at_least = switches_to.unwrap_or(Severity::Info);
    let fires_at = |d: &DocResult, t: f64| of_key(d, rule, key).any(|m| m.fires_at(t));
    let point = |t: f64| ThresholdPoint {
        threshold: t,
        calibration: outcome(docs, Subset::Calibration, at_least, |d| fires_at(d, t)),
        holdout: outcome(docs, Subset::Holdout, at_least, |d| fires_at(d, t)),
        all: outcome(docs, Subset::All, at_least, |d| fires_at(d, t)),
    };
    let measured = |group: Group| {
        docs.iter()
            .filter(|d| d.group == group && of_key(d, rule, key).next().is_some())
            .count()
    };

    let current = ev.current(key);
    let mut values: Vec<f64> = docs
        .iter()
        .filter(|d| d.split == Split::Calibration)
        .flat_map(|d| of_key(d, rule, key).map(|m| m.value))
        .filter(|v| v.is_finite())
        .collect();
    values.sort_by(f64::total_cmp);
    values.dedup();
    let integer =
        values.iter().all(|v| v.fract() == 0.0) && current.is_none_or(|c| c.fract() == 0.0);
    let suggested = candidates(&values, direction, integer, current)
        .into_iter()
        .map(|t| {
            let o = outcome(docs, Subset::Calibration, at_least, |d| fires_at(d, t));
            (t, o)
        })
        .filter(|(_, o)| o.usable(target_fp))
        .min_by(|a, b| rank(a, b, current))
        .map(|(t, _)| point(t));

    ThresholdSweep {
        rule_id: ev.meta.id,
        rule_name: ev.meta.name,
        lane: ev.meta.lane,
        status: ev.meta.status,
        measure,
        key,
        fires: fires_key(direction),
        direction,
        switches_to,
        measured_human: measured(Group::Human),
        measured_ai: measured(Group::Ai),
        current: current.map(point),
        suggested,
    }
}

/// 閾値の候補の順位: 検出した文書が多い → 誤検知した文書が少ない → 現在の値に近い → 小さい。
fn rank(a: &(f64, Outcome), b: &(f64, Outcome), current: Option<f64>) -> Ordering {
    let distance = |t: f64| current.map_or(t.abs(), |c| (t - c).abs());
    b.1.ai_fired
        .cmp(&a.1.ai_fired)
        .then(a.1.human_fired.cmp(&b.1.human_fired))
        .then(distance(a.0).total_cmp(&distance(b.0)))
        .then(a.0.total_cmp(&b.0))
}

/// 閾値の候補: 現在の値、観測値の隣り合う 2 つのあいだ、測れた文書をすべて指摘する値。
///
/// `values` は校正用の文書の観測値 (昇順・重複なし)。整数の設定キーは、あいだの値を
/// 向きに合わせて丸める (以上で指摘するなら切り上げ、を超えると指摘するなら切り捨て)。
fn candidates(values: &[f64], direction: Fires, integer: bool, current: Option<f64>) -> Vec<f64> {
    let mut out: Vec<f64> = current.into_iter().collect();
    for w in values.windows(2) {
        out.push(between(w[0], w[1], direction, integer));
    }
    if let (Some(&lo), Some(&hi)) = (values.first(), values.last()) {
        out.push(everything(lo, hi, direction, integer));
    }
    out
}

/// 観測値 `a < b` を分ける閾値 (`a` と `b` の一方だけを指摘する)。
fn between(a: f64, b: f64, direction: Fires, integer: bool) -> f64 {
    if !integer {
        return round_between(a, b);
    }
    let mid = (a + b) / 2.0;
    match direction {
        Fires::AtOrAbove | Fires::Below => mid.ceil(),
        Fires::Above | Fires::AtOrBelow => mid.floor(),
    }
}

/// 観測値の最小 `lo`・最大 `hi` に対して、測れた文書をすべて指摘する閾値。
fn everything(lo: f64, hi: f64, direction: Fires, integer: bool) -> f64 {
    match direction {
        Fires::AtOrAbove => lo,
        Fires::AtOrBelow => hi,
        Fires::Above if integer => lo - 1.0,
        Fires::Below if integer => hi + 1.0,
        Fires::Above => round_between(lo - lo.abs().max(1.0), lo),
        Fires::Below => round_between(hi, hi + hi.abs().max(1.0)),
    }
}

/// `a < t < b` を満たす、桁数のなるべく少ない値 (設定ファイルに書きやすくするため)。
fn round_between(a: f64, b: f64) -> f64 {
    let mid = (a + b) / 2.0;
    for digits in 0..=6 {
        let scale = 10f64.powi(digits);
        let t = (mid * scale).round() / scale;
        if a < t && t < b {
            return t;
        }
    }
    mid
}

fn fires_key(direction: Fires) -> &'static str {
    match direction {
        Fires::Below => "below",
        Fires::AtOrBelow => "atOrBelow",
        Fires::Above => "above",
        Fires::AtOrAbove => "atOrAbove",
    }
}

/// 比べる向きの表示 (「閾値以上で指摘」。重大度の切り替えなら「閾値以上で重大」)。
fn fires_label(direction: Fires, switches_to: Option<Severity>) -> String {
    let what = switches_to.map_or("指摘", Severity::label_ja);
    match direction {
        Fires::Below => format!("閾値未満で{what}"),
        Fires::AtOrBelow => format!("閾値以下で{what}"),
        Fires::Above => format!("閾値を超えると{what}"),
        Fires::AtOrAbove => format!("閾値以上で{what}"),
    }
}

fn item_stats(
    evaluated: &[Evaluated],
    docs: &[DocResult],
    rules: &[RuleStats],
    corpus: &Corpus,
) -> Vec<ItemStats> {
    #[derive(Default)]
    struct Agg {
        human: Tally,
        ai: Tally,
        severity: Option<Severity>,
        notations: HashMap<String, usize>,
        /// 項目が出た文書のうち、ルール (校正済みの項目) が何も指摘しなかった文書の数。
        human_new: usize,
        ai_new: usize,
    }
    let mut aggs: HashMap<&ItemKey, Agg> = HashMap::new();
    for d in docs {
        for (key, hit) in &d.items {
            let agg = aggs.entry(key).or_default();
            agg.severity = agg.severity.max(hit.hits.highest());
            for (notation, n) in &hit.notations {
                *agg.notations.entry(notation.clone()).or_default() += n;
            }
            let new = usize::from(d.hits[key.0].total() == 0);
            match d.group {
                Group::Human => {
                    agg.human.add(&hit.hits);
                    agg.human_new += new;
                }
                Group::Ai => {
                    agg.ai.add(&hit.hits);
                    agg.ai_new += new;
                }
            }
        }
    }
    let mut out: Vec<(usize, ItemStats)> = aggs
        .into_iter()
        .map(|((rule, item, status), agg)| {
            let ev = &evaluated[*rule];
            let stats = &rules[*rule];
            let promoted_rule = (*status == RuleStatus::Experimental
                && ev.mode == Mode::Default
                && ev.meta.status == RuleStatus::Stable)
                .then(|| {
                    Outcome::new(
                        Severity::Info,
                        stats.human.fired + agg.human_new,
                        stats.human.docs,
                        stats.ai.fired + agg.ai_new,
                        stats.ai.docs,
                    )
                });
            let mut notations: Vec<(String, usize)> = agg.notations.into_iter().collect();
            notations.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let stats = ItemStats {
                rule_id: ev.meta.id,
                rule_name: ev.meta.name,
                item: item.clone(),
                examples: notations
                    .into_iter()
                    .take(ITEM_EXAMPLES)
                    .map(|(n, _)| n)
                    .collect(),
                status: *status,
                severity: agg.severity.unwrap_or(Severity::Info),
                human: agg.human.rate(corpus.human.docs, corpus.human.characters),
                ai: agg.ai.rate(corpus.ai.docs, corpus.ai.characters),
                promoted_rule,
            };
            (*rule, stats)
        })
        .collect();
    out.sort_by(|(ra, a), (rb, b)| {
        ra.cmp(rb)
            .then(b.ai.fired.cmp(&a.ai.fired))
            .then(a.human.fired.cmp(&b.human.fired))
            .then_with(|| a.item.cmp(&b.item))
            .then(
                (a.status == RuleStatus::Experimental).cmp(&(b.status == RuleStatus::Experimental)),
            )
    });
    out.into_iter().map(|(_, item)| item).collect()
}

fn promotions(
    options: &CalibrateOptions,
    evaluated: &[Evaluated],
    rules: &[RuleStats],
    thresholds: &[ThresholdSweep],
    items: &[ItemStats],
) -> Vec<Promotion> {
    let (target, min_detection) = (options.target_fp, options.min_detection);
    let candidate = |ev: &Evaluated,
                     item: Option<ItemSummary>,
                     threshold: Option<ThresholdChange>,
                     outcome: Outcome,
                     basis: Basis| Promotion {
        rule_id: ev.meta.id,
        rule_name: ev.meta.name,
        item,
        threshold,
        small_sample: !outcome.confirmed(target),
        outcome,
        basis,
    };
    let mut out = Vec::new();
    for (r, ev) in evaluated.iter().enumerate() {
        if ev.meta.status != RuleStatus::Experimental || !ev.judged() {
            continue;
        }
        // 重大度を問わない率で判定する (情報だけのルールは、警告以上で数えると 0% に見えるため)
        let current = rules[r].outcome();
        if current.promotable(target, min_detection) {
            out.push(candidate(ev, None, None, current, Basis::All));
            continue;
        }
        // 指摘するかどうかを決める閾値が 1 つだけのルールは、閾値を変えれば条件を満たすかも見る
        // (複数あるルールは、キーごとの率がルール全体の率と一致しないので見ない。重大度の
        // 切り替えを決める閾値は、指摘するかどうかを変えないので数えない)
        let sweeps: Vec<&ThresholdSweep> = thresholds
            .iter()
            .filter(|t| t.rule_id == ev.meta.id && t.switches_to.is_none())
            .collect();
        if let [sweep] = sweeps.as_slice()
            && let Some(suggested) = &sweep.suggested
            && sweep
                .current
                .as_ref()
                .is_none_or(|c| !same(c.threshold, suggested.threshold))
            && suggested.holdout.promotable(target, min_detection)
        {
            let change = ThresholdChange {
                key: sweep.key,
                from: sweep.current.as_ref().map(|c| c.threshold),
                to: suggested.threshold,
            };
            out.push(candidate(
                ev,
                None,
                Some(change),
                suggested.holdout.clone(),
                Basis::Holdout,
            ));
        }
    }
    for item in items {
        let Some(after) = &item.promoted_rule else {
            continue;
        };
        let Some(ev) = evaluated.iter().find(|ev| ev.meta.id == item.rule_id) else {
            continue;
        };
        let human_rate = item.human.rate.unwrap_or(0.0);
        let ai_rate = item.ai.rate.unwrap_or(0.0);
        if ev.judged()
            && item.ai.fired >= MIN_ITEM_AI_DOCS
            && ai_rate >= MIN_ITEM_LIFT * human_rate
            && after.usable(target)
        {
            out.push(candidate(
                ev,
                Some(ItemSummary::of(item)),
                None,
                after.clone(),
                Basis::All,
            ));
        }
    }
    out
}

fn reviews(
    options: &CalibrateOptions,
    evaluated: &[Evaluated],
    rules: &[RuleStats],
    items: &[ItemStats],
) -> Vec<Review> {
    let target = options.target_fp;
    let mut out = Vec::new();
    for (r, ev) in evaluated.iter().enumerate() {
        if ev.meta.status != RuleStatus::Stable || !ev.judged() || ev.mode != Mode::Default {
            continue;
        }
        let at_basis = rules[r].outcome_at(ev.basis);
        let any = rules[r].outcome();
        let (Some(fp), Some(detection)) = (at_basis.false_positive, at_basis.detection) else {
            continue;
        };
        let reason = if fp > target + EPSILON {
            ReviewReason::FalsePositives
        } else if at_basis.human_fired + at_basis.ai_fired > 0 && detection <= fp {
            ReviewReason::NoSeparation
        } else if ev.basis > Severity::Info
            && any.false_positive.is_some_and(|fp| fp > target + EPSILON)
        {
            ReviewReason::InfoBurden
        } else {
            continue;
        };
        // 理由に関わる重大度の項目: 軽い指摘が多いなら基準より軽い項目、ほかは基準以上の項目
        let relevant = |severity: Severity| match reason {
            ReviewReason::InfoBurden => severity < ev.basis,
            ReviewReason::FalsePositives | ReviewReason::NoSeparation => severity >= ev.basis,
        };
        let mut human_items: Vec<&ItemStats> = items
            .iter()
            .filter(|i| {
                i.rule_id == ev.meta.id
                    && i.status == RuleStatus::Stable
                    && i.human.fired > 0
                    && relevant(i.severity)
            })
            .collect();
        human_items.sort_by(|a, b| {
            b.human
                .fired
                .cmp(&a.human.fired)
                .then_with(|| a.item.cmp(&b.item))
        });
        out.push(Review {
            rule_id: ev.meta.id,
            rule_name: ev.meta.name,
            reason,
            calibration_basis: ev.basis,
            outcome: at_basis,
            any_severity: any,
            human_items: human_items
                .into_iter()
                .take(REVIEW_TOP_ITEMS)
                .map(ItemSummary::of)
                .collect(),
        });
    }
    out
}

fn warnings(options: &CalibrateOptions, corpus: &Corpus, docs: &[DocResult]) -> Vec<String> {
    let mut out = Vec::new();
    for group in [Group::Human, Group::Ai] {
        let n = docs
            .iter()
            .filter(|d| d.group == group && d.line_per_paragraph)
            .count();
        if n > 0 {
            out.push(format!(
                "{}の {n} 件は、段落のあいだに空行がないようです (文末で改行した行が、空行を挟まずに続いています)。Markdown では空行のない行は 1 つの段落にまとまるため、段落を単位に見るルール (R07・R09 など) を正しく測れません。段落のあいだに空行を入れるか、拡張子を .txt にしてください (テキストは、空行のほとんどない文書を 1 行 1 段落として読みます)",
                group.docs_ja()
            ));
        }
    }
    for (group, size) in [(Group::Human, &corpus.human), (Group::Ai, &corpus.ai)] {
        if size.docs < MIN_DOCS_PER_GROUP {
            out.push(format!(
                "{}が {} 件しかありません。1 群 {MIN_DOCS_PER_GROUP} 件以上そろえないと、率が大きくぶれます",
                group.docs_ja(),
                size.docs
            ));
        } else if options.holdout > 0.0 && size.holdout < MIN_HOLDOUT_DOCS {
            out.push(format!(
                "検証用に回った{}が {} 件しかありません。検証の率は参考程度に見てください",
                group.docs_ja(),
                size.holdout
            ));
        }
    }
    let docs = corpus.human.docs;
    match required_docs(options.target_fp) {
        Some(needed) if docs > 0 && docs < needed => out.push(format!(
            "人の文書が {docs} 件では、1 件も指摘されなくても誤検知率の 95% 上限が {} になり、目標の {} 以下と言い切れません。言い切るには人の文書が最低 {needed} 件要ります",
            pct(wilson_upper(0, docs)),
            pct(Some(options.target_fp))
        )),
        None if docs > 0 => out.push(
            "目標の誤検知率が 0% では、人の文書を何件そろえても、95% 上限で目標以下と言い切れません"
                .to_string(),
        ),
        _ => {}
    }
    out
}

/// 閾値として同じ値か。
fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= EPSILON * a.abs().max(b.abs()).max(1.0)
}

// ---------------------------------------------------------------------------
// 出力
// ---------------------------------------------------------------------------

/// 出力形式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReportFormat {
    #[default]
    Text,
    Json,
    Markdown,
}

/// 指定の形式で書き出す。
pub fn render(
    report: &CalibrationReport,
    format: ReportFormat,
    out: &mut dyn Write,
) -> io::Result<()> {
    match format {
        ReportFormat::Text => render_text(report, out),
        ReportFormat::Json => render_json(report, out),
        ReportFormat::Markdown => render_markdown(report, out),
    }
}

/// JSON で書き出す (整形済み)。
pub fn render_json(report: &CalibrationReport, out: &mut dyn Write) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut *out, report).map_err(io::Error::other)?;
    writeln!(out)
}

/// ルール別の反応の表の見出しの説明 (text と markdown で共通)。
const RULE_TABLE_LEGEND: [&str; 2] = [
    "誤検知 = 人の文書で指摘された割合、検出 = 生成文書で指摘された割合",
    "全 = 重大度を問わない、基準 = 校正の基準の重大度以上の指摘だけで数えた割合",
];

/// 端末向けのテキストで書き出す。
pub fn render_text(report: &CalibrationReport, out: &mut dyn Write) -> io::Result<()> {
    let s = &report.settings;
    let target = pct(Some(s.target_fp));
    writeln!(out, "noslop calibrate")?;
    writeln!(out, "条件: {}", settings_line(s))?;
    writeln!(out, "文書: {}", corpus_line(&report.corpus))?;
    if !report.warnings.is_empty() {
        writeln!(out)?;
        writeln!(out, "注意:")?;
        for w in &report.warnings {
            writeln!(out, "  - {w}")?;
        }
    }

    writeln!(out)?;
    writeln!(out, "ルール別の反応 (全文書)")?;
    for line in RULE_TABLE_LEGEND {
        writeln!(out, "  {line}")?;
    }
    if report.rules.is_empty() {
        writeln!(out, "  (測ったルールがありません)")?;
    } else {
        let mut rows = vec![
            [
                "ID",
                "状態",
                "基準",
                "誤検知 全",
                "誤検知 基準",
                "検出 全",
                "検出 基準",
                "ルール",
            ]
            .map(String::from)
            .to_vec(),
        ];
        for r in &report.rules {
            let basis = r.calibration_basis;
            rows.push(vec![
                r.id.to_string(),
                status_label(r.status).to_string(),
                basis.label_ja().to_string(),
                pct(r.human.rate),
                pct(r.human.rate_at_least(basis)),
                pct(r.ai.rate),
                pct(r.ai.rate_at_least(basis)),
                r.name.to_string(),
            ]);
        }
        write_columns(out, "  ", &rows, &[3, 4, 5, 6])?;
    }

    writeln!(out)?;
    writeln!(
        out,
        "閾値 (校正用の文書で誤検知 {target} 以下のまま検出が最大になる値を探し、検証用の文書で確かめる)"
    )?;
    if report.thresholds.is_empty() {
        writeln!(out, "  (閾値を測れるルールがありません)")?;
    }
    for t in &report.thresholds {
        writeln!(
            out,
            "  {} {}: {} を {} と比べる ({})。値を測れた文書: 人 {}/{}・生成 {}/{}",
            t.rule_id,
            t.rule_name,
            t.measure,
            t.key,
            fires_label(t.direction, t.switches_to),
            t.measured_human,
            report.corpus.human.docs,
            t.measured_ai,
            report.corpus.ai.docs
        )?;
        if let Some(severity) = t.switches_to {
            writeln!(out, "    {}", switch_note(severity))?;
        }
        match &t.current {
            Some(c) => writeln!(
                out,
                "    現在 {}  全体: {}  検証: {}",
                num(c.threshold),
                outcome_text(&c.all),
                outcome_text(&c.holdout)
            )?,
            None => writeln!(out, "    現在 (設定値を数値として読めません)")?,
        }
        match (&t.suggested, &t.current) {
            (Some(p), Some(c)) if same(p.threshold, c.threshold) => {
                writeln!(out, "    提案 現在の値のまま")?;
            }
            (Some(p), _) => writeln!(
                out,
                "    提案 {}  校正: {}  検証: {}",
                num(p.threshold),
                outcome_text(&p.calibration),
                outcome_text(&p.holdout)
            )?,
            (None, _) => writeln!(
                out,
                "    提案 なし (誤検知 {target} 以下で生成文書を拾える閾値がありません)"
            )?,
        }
    }

    writeln!(out)?;
    writeln!(out, "校正済みに上げる候補 ({})", promotion_condition(s))?;
    if report.promotions.is_empty() {
        writeln!(out, "  (なし)")?;
    }
    for p in &report.promotions {
        writeln!(out, "  - {}", promotion_text(p, s.target_fp))?;
    }

    writeln!(out)?;
    writeln!(out, "見直しが必要な校正済みルール")?;
    if report.reviews.is_empty() {
        writeln!(out, "  (なし)")?;
    }
    for reason in ReviewReason::ALL {
        let group: Vec<&Review> = report
            .reviews
            .iter()
            .filter(|r| r.reason == reason)
            .collect();
        if group.is_empty() {
            continue;
        }
        writeln!(out, "  {}:", reason.heading())?;
        for r in group {
            writeln!(out, "    - {}", review_text(r, s.target_fp))?;
            if let Some(stats) = report.rules.iter().find(|x| x.id == r.rule_id) {
                writeln!(
                    out,
                    "        人の文書の内訳: {}",
                    breakdown_text(&stats.human)
                )?;
            }
            if !r.human_items.is_empty() {
                writeln!(
                    out,
                    "        人の文書で多い項目: {}",
                    human_items_text(&r.human_items)
                )?;
            }
        }
    }

    if !report.errors.is_empty() {
        writeln!(out)?;
        writeln!(out, "読めなかった入力:")?;
        for e in &report.errors {
            writeln!(out, "  - {} {}: {}", e.group.label_ja(), e.path, e.message)?;
        }
    }
    Ok(())
}

/// Markdown (記録やレビュー用) で書き出す。
pub fn render_markdown(report: &CalibrationReport, out: &mut dyn Write) -> io::Result<()> {
    let s = &report.settings;
    writeln!(out, "# noslop calibrate の結果")?;
    writeln!(out)?;
    writeln!(out, "- 条件: {}", settings_line(s))?;
    writeln!(out, "- 文書: {}", corpus_line(&report.corpus))?;
    writeln!(
        out,
        "- 誤検知は人の文書で、検出は生成文書で、1 件以上指摘された文書の割合"
    )?;
    if !report.warnings.is_empty() {
        writeln!(out)?;
        writeln!(out, "## 注意")?;
        writeln!(out)?;
        for w in &report.warnings {
            writeln!(out, "- {w}")?;
        }
    }

    writeln!(out)?;
    writeln!(out, "## ルール別の反応 (全文書)")?;
    writeln!(out)?;
    writeln!(out, "{}。", RULE_TABLE_LEGEND.join("。"))?;
    writeln!(out)?;
    writeln!(
        out,
        "| ID | ルール | 状態 | 基準 | 誤検知 全 | 誤検知 基準 | 検出 全 | 検出 基準 |"
    )?;
    writeln!(out, "|---|---|---|---|---:|---:|---:|---:|")?;
    for r in &report.rules {
        let basis = r.calibration_basis;
        writeln!(
            out,
            "| {} | {} ({}) | {} | {} | {} | {} | {} | {} |",
            r.id,
            cell(r.name),
            cell(r.title),
            status_label(r.status),
            basis.label_ja(),
            pct(r.human.rate),
            pct(r.human.rate_at_least(basis)),
            pct(r.ai.rate),
            pct(r.ai.rate_at_least(basis))
        )?;
    }

    writeln!(out)?;
    writeln!(out, "## 閾値")?;
    writeln!(out)?;
    writeln!(
        out,
        "提案は校正用の文書で「誤検知 {} 以下のまま検出が最大」になる値で、検証用の文書の率を添える。向きが「〜で重大」の行は重大にするかどうかを決める閾値で、率は重大の指摘があった文書の割合。",
        pct(Some(s.target_fp))
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "| ID | 測る量 → 設定キー | 向き | 現在 | 現在の誤検知 / 検出 (全体) | 提案 | 提案の誤検知 / 検出 (校正) | 提案の誤検知 / 検出 (検証) |"
    )?;
    writeln!(out, "|---|---|---|---:|---:|---:|---:|---:|")?;
    for t in &report.thresholds {
        let (current, current_rates) = match &t.current {
            Some(c) => (num(c.threshold), pair_text(&c.all)),
            None => ("—".to_string(), "—".to_string()),
        };
        let (suggested, calibration, holdout) = match (&t.suggested, &t.current) {
            (Some(p), Some(c)) if same(p.threshold, c.threshold) => (
                "(現在のまま)".to_string(),
                pair_text(&p.calibration),
                pair_text(&p.holdout),
            ),
            (Some(p), _) => (
                num(p.threshold),
                pair_text(&p.calibration),
                pair_text(&p.holdout),
            ),
            (None, _) => ("なし".to_string(), "—".to_string(), "—".to_string()),
        };
        writeln!(
            out,
            "| {} | {} → `{}` | {} | {current} | {current_rates} | {suggested} | {calibration} | {holdout} |",
            t.rule_id,
            cell(t.measure),
            t.key,
            fires_label(t.direction, t.switches_to)
        )?;
    }

    writeln!(out)?;
    writeln!(out, "## 校正済みに上げる候補")?;
    writeln!(out)?;
    writeln!(out, "条件: {}。", promotion_condition(s))?;
    writeln!(out)?;
    if report.promotions.is_empty() {
        writeln!(out, "なし")?;
    }
    for p in &report.promotions {
        writeln!(out, "- {}", cell(&promotion_text(p, s.target_fp)))?;
    }

    writeln!(out)?;
    writeln!(out, "## 見直しが必要な校正済みルール")?;
    writeln!(out)?;
    if report.reviews.is_empty() {
        writeln!(out, "なし")?;
    }
    let mut first = true;
    for reason in ReviewReason::ALL {
        let group: Vec<&Review> = report
            .reviews
            .iter()
            .filter(|r| r.reason == reason)
            .collect();
        if group.is_empty() {
            continue;
        }
        if !first {
            writeln!(out)?;
        }
        first = false;
        writeln!(out, "### {}", reason.heading())?;
        writeln!(out)?;
        for r in group {
            writeln!(out, "- {}", review_text(r, s.target_fp))?;
            if let Some(stats) = report.rules.iter().find(|x| x.id == r.rule_id) {
                writeln!(out, "  - 人の文書の内訳: {}", breakdown_text(&stats.human))?;
            }
            if !r.human_items.is_empty() {
                writeln!(
                    out,
                    "  - 人の文書で多い項目: {}",
                    human_items_text(&r.human_items)
                )?;
            }
        }
    }

    if !report.errors.is_empty() {
        writeln!(out)?;
        writeln!(out, "## 読めなかった入力")?;
        writeln!(out)?;
        for e in &report.errors {
            writeln!(out, "- {} `{}`: {}", e.group.label_ja(), e.path, e.message)?;
        }
    }
    Ok(())
}

fn settings_line(s: &Settings) -> String {
    format!(
        "ジャンル {} / 目標の誤検知率 {} / 検証に回す割合 {} / 昇格に要る検出率 {} / 実験的なルールと語句を{}",
        s.genre,
        pct(Some(s.target_fp)),
        pct(Some(s.holdout)),
        pct(Some(s.min_detection)),
        if s.include_experimental {
            "含む"
        } else {
            "含まない"
        }
    )
}

fn corpus_line(corpus: &Corpus) -> String {
    let group = |g: Group, size: &GroupSize| {
        format!(
            "{} {} 件 (校正 {}・検証 {}、{} 字)",
            g.label_ja(),
            size.docs,
            size.calibration,
            size.holdout,
            thousands(size.characters)
        )
    };
    format!(
        "{}、{}",
        group(Group::Human, &corpus.human),
        group(Group::Ai, &corpus.ai)
    )
}

fn promotion_condition(s: &Settings) -> String {
    format!(
        "重大度を問わない誤検知 {} 以下、検出 {} 以上。誤検知の 95% 上限が目標を超える候補には注記を付ける",
        pct(Some(s.target_fp)),
        pct(Some(s.min_detection))
    )
}

/// 下限の重大度の呼び方 (「重大度を問わない」「警告以上」「重大」)。
fn at_least_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Info => "重大度を問わない",
        Severity::Warning => "警告以上",
        Severity::Error => "重大",
    }
}

/// 重大度の切り替えを決める閾値の注記。
fn switch_note(severity: Severity) -> String {
    format!(
        "この閾値が決めるのは{}にするかどうかで、率は{}の指摘があった文書の割合",
        severity.label_ja(),
        at_least_label(severity)
    )
}

fn promotion_text(p: &Promotion, target_fp: f64) -> String {
    let mut text = if let Some(item) = &p.item {
        format!(
            "{} {} の項目「{}」({}{}、生成 {} 件・人 {} 件): 上げた後の {} は {}",
            p.rule_id,
            p.rule_name,
            item.item,
            examples_text(&item.item, &item.examples),
            item.severity.label_ja(),
            item.ai_docs,
            item.human_docs,
            p.rule_id,
            outcome_text(&p.outcome)
        )
    } else {
        match &p.threshold {
            Some(change) => format!(
                "{} {}: {} を {} から {} にすると {} (検証用の文書)",
                p.rule_id,
                p.rule_name,
                change.key,
                change.from.map_or_else(|| "—".to_string(), num),
                num(change.to),
                outcome_text(&p.outcome)
            ),
            None => format!(
                "{} {}: {} (全文書・現在の閾値)",
                p.rule_id,
                p.rule_name,
                outcome_text(&p.outcome)
            ),
        }
    };
    if p.small_sample {
        text.push_str(&format!(
            "。件数が少なく、目標の {} 以下と言い切れません (95% 上限 {})",
            pct(Some(target_fp)),
            pct(p.outcome.false_positive_upper95)
        ));
    }
    text
}

fn review_text(r: &Review, target_fp: f64) -> String {
    let (o, any) = (&r.outcome, &r.any_severity);
    let basis = at_least_label(r.calibration_basis);
    match r.reason {
        ReviewReason::FalsePositives => format!(
            "{} {}: 誤検知 ({basis}) {} が目標の {} を超えています (検出 {})",
            r.rule_id,
            r.rule_name,
            fraction(o.false_positive, o.human_fired, o.human_docs),
            pct(Some(target_fp)),
            fraction(o.detection, o.ai_fired, o.ai_docs)
        ),
        ReviewReason::NoSeparation => format!(
            "{} {}: {basis}の指摘で数えると、生成文書で指摘される割合が人の文書を上回っていません ({})",
            r.rule_id,
            r.rule_name,
            outcome_text(o)
        ),
        ReviewReason::InfoBurden => format!(
            "{} {}: 誤検知 ({basis}) {} は目標の {} 以下ですが、重大度を問わないと {} です (検出 {})",
            r.rule_id,
            r.rule_name,
            fraction(o.false_positive, o.human_fired, o.human_docs),
            pct(Some(target_fp)),
            fraction(any.false_positive, any.human_fired, any.human_docs),
            fraction(any.detection, any.ai_fired, any.ai_docs)
        ),
    }
}

/// 重大度ごとの、指摘が出た文書の数。
fn breakdown_text(rate: &Rate) -> String {
    let parts: Vec<String> = SEVERITIES
        .iter()
        .map(|&s| format!("{} {}", s.label_ja(), rate.by_severity.get(s).fired))
        .collect();
    format!("{} (指摘が出た文書の数)", parts.join("・"))
}

/// 表記の例 (項目の名前と同じ表記しかなければ空)。「例「a」「b」、」の形。
fn examples_text(item: &str, examples: &[String]) -> String {
    if examples.iter().all(|e| e == item) {
        return String::new();
    }
    let quoted: String = examples.iter().map(|e| format!("「{e}」")).collect();
    format!("例{quoted}、")
}

fn human_items_text(items: &[ItemSummary]) -> String {
    items
        .iter()
        .map(|i| {
            format!(
                "「{}」({}{}、人 {} 件・生成 {} 件)",
                i.item,
                examples_text(&i.item, &i.examples),
                i.severity.label_ja(),
                i.human_docs,
                i.ai_docs
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn outcome_text(o: &Outcome) -> String {
    format!(
        "誤検知 {} 検出 {}",
        fraction(o.false_positive, o.human_fired, o.human_docs),
        fraction(o.detection, o.ai_fired, o.ai_docs)
    )
}

fn pair_text(o: &Outcome) -> String {
    format!("{} / {}", pct(o.false_positive), pct(o.detection))
}

fn fraction(value: Option<f64>, n: usize, d: usize) -> String {
    format!("{} ({n}/{d})", pct(value))
}

fn pct(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_string(), |v| format!("{:.1}%", v * 100.0))
}

/// 閾値の表示。短く表せる値はそのまま、長くなる値は小数第 4 位までにする。
fn num(value: f64) -> String {
    let plain = value.to_string();
    if plain.len() <= 8 {
        return plain;
    }
    let rounded = format!("{value:.4}");
    rounded
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn status_label(status: RuleStatus) -> &'static str {
    match status {
        RuleStatus::Stable => "校正済み",
        RuleStatus::Experimental => "実験的",
    }
}

/// Markdown の表のセルに入れられるようにする。
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// 列をそろえて書く (全角文字は 2 桁として数える)。`right` に挙げた列 (数値の列) は右に寄せる。
fn write_columns(
    out: &mut dyn Write,
    indent: &str,
    rows: &[Vec<String>],
    right: &[usize],
) -> io::Result<()> {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|c| {
            rows.iter()
                .filter_map(|r| r.get(c))
                .map(|s| s.width())
                .max()
                .unwrap_or(0)
        })
        .collect();
    for row in rows {
        let mut line = String::from(indent);
        for (c, text) in row.iter().enumerate() {
            let pad = " ".repeat(widths[c] - text.width());
            let last = c + 1 == row.len();
            if right.contains(&c) {
                line.push_str(&pad);
                line.push_str(text);
            } else {
                line.push_str(text);
                if !last {
                    line.push_str(&pad);
                }
            }
            if !last {
                line.push_str("  ");
            }
        }
        writeln!(out, "{}", line.trim_end())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::engine::Selection;

    // -----------------------------------------------------------------------
    // 率と、その重大度の内訳
    // -----------------------------------------------------------------------

    #[test]
    fn severity_breakdown_counts_hits_per_severity_and_documents_once_per_floor() {
        let mut tally = Tally::default();
        for hits in [[1, 1, 1], [0, 2, 0], [3, 0, 0], [0, 0, 0]] {
            tally.add(&Hits(hits));
        }
        let r = tally.rate(4, 4000);
        assert_eq!((r.fired, r.hits), (3, 8));
        assert_eq!(r.by_severity.info, SeverityCount { fired: 2, hits: 4 });
        assert_eq!(r.by_severity.warning, SeverityCount { fired: 2, hits: 3 });
        assert_eq!(r.by_severity.error, SeverityCount { fired: 1, hits: 1 });
        // 警告と重大の両方が出た文書も、警告以上では 1 件と数える
        assert_eq!((r.at_least.warning.fired, r.at_least.error.fired), (2, 1));
        assert_eq!(r.at_least.warning.rate, Some(0.5));
        assert_eq!(
            SEVERITIES.map(|s| r.fired_at_least(s)),
            [3, 2, 1],
            "重大度を問わない・警告以上・重大"
        );
        assert_eq!(r.per_1000_chars, Some(2.0));
        assert!(r.upper95.unwrap() > r.rate.unwrap());
        assert!(r.at_least.error.upper95.unwrap() > r.at_least.error.rate.unwrap());

        let empty = Tally::default().rate(0, 0);
        assert_eq!(
            (empty.rate, empty.upper95, empty.per_1000_chars),
            (None, None, None)
        );
    }

    #[test]
    fn wilson_upper_bounds_and_required_documents() {
        let close = |value: Option<f64>, expected: f64| {
            let v = value.unwrap();
            assert!((v - expected).abs() < 1e-6, "{v} ≠ {expected}");
        };
        close(wilson_upper(0, 10), 0.212951);
        close(wilson_upper(5, 100), 0.099163);
        close(wilson_upper(1, 71), 0.060696);
        close(wilson_upper(10, 10), 1.0);
        assert_eq!(wilson_upper(0, 0), None);
        // 0 件でも上限が 5% 以下になるのは 52 件から
        assert!(wilson_upper(0, 51).unwrap() > 0.05);
        assert!(wilson_upper(0, 52).unwrap() <= 0.05);
        assert_eq!(required_docs(0.05), Some(52));
        assert_eq!(required_docs(0.1), Some(25));
        assert_eq!(required_docs(0.01), Some(268));
        assert_eq!(required_docs(1.0), Some(1));
        assert_eq!(required_docs(0.0), None);
        // ごく小さい目標でも、1 件ずつ探さずにすぐ返る (境界は注記と同じ許容誤差で決まる)
        let tiny = 1e-10;
        let n = required_docs(tiny).unwrap();
        let within = |n: usize| wilson_upper(0, n).unwrap() <= tiny + EPSILON;
        assert!(within(n) && !within(n - 1), "{n}");
    }

    // -----------------------------------------------------------------------
    // 見直しと昇格 (合成した件数で)
    // -----------------------------------------------------------------------

    macro_rules! test_meta {
        ($id:literal, $status:expr, $severity:expr) => {{
            static META: RuleMeta = RuleMeta {
                id: $id,
                name: $id,
                title: "テスト用",
                lane: Lane::Slop,
                status: $status,
                default_severity: $severity,
                summary: "テスト用",
                explanation: "",
            };
            &META
        }};
    }

    /// 昇格・見直しの対象になる、組み込みの AI 臭さのルール。
    fn judged(meta: &'static RuleMeta, basis: Severity) -> Evaluated {
        Evaluated {
            entry: 0,
            meta,
            mode: match meta.status {
                RuleStatus::Stable => Mode::Default,
                RuleStatus::Experimental => Mode::Experimental,
            },
            builtin: true,
            options: Vec::new(),
            basis,
        }
    }

    /// 人と生成で `docs` 件ずつの文書。`per_rule[r]` は (人, 生成) それぞれで、情報・警告・重大の
    /// 指摘を 1 件持つ文書の数 (先頭の文書から順に、重なりなく割り当てる)。
    fn severity_docs(docs: usize, per_rule: &[([usize; 3], [usize; 3])]) -> Vec<DocResult> {
        let mut out = Vec::new();
        for group in [Group::Human, Group::Ai] {
            for i in 0..docs {
                let hits = per_rule
                    .iter()
                    .map(|(human, ai)| {
                        let counts = if group == Group::Human { human } else { ai };
                        let mut hits = Hits::default();
                        let mut start = 0;
                        for s in SEVERITIES {
                            let n = counts[slot(s)];
                            if (start..start + n).contains(&i) {
                                hits.add(s);
                            }
                            start += n;
                        }
                        hits
                    })
                    .collect();
                out.push(DocResult {
                    group,
                    split: Split::Calibration,
                    characters: 1000,
                    line_per_paragraph: false,
                    hits,
                    measures: vec![Vec::new(); per_rule.len()],
                    items: HashMap::new(),
                });
            }
        }
        out
    }

    fn synthetic_report(evaluated: &[Evaluated], docs: &[DocResult]) -> CalibrationReport {
        let options = CalibrateOptions::new(Vec::new(), Vec::new());
        build_report(&options, evaluated, docs, Vec::new())
    }

    #[test]
    fn reviews_are_judged_on_the_calibration_basis() {
        use ReviewReason::*;
        use Severity::*;
        let evaluated = [
            judged(test_meta!("A01", RuleStatus::Stable, Warning), Warning),
            judged(test_meta!("A02", RuleStatus::Stable, Warning), Warning),
            judged(test_meta!("A03", RuleStatus::Stable, Info), Info),
            judged(test_meta!("A04", RuleStatus::Stable, Warning), Error),
            judged(test_meta!("A05", RuleStatus::Stable, Warning), Warning),
            judged(test_meta!("A06", RuleStatus::Stable, Warning), Warning),
        ];
        let docs = severity_docs(
            40,
            &[
                // 警告以上で 20%: 基準で数えても目標を超える
                ([0, 8, 0], [0, 30, 0]),
                // 情報だけで 50%: 基準 (警告以上) では 0% だが、重大度を問わないと超える
                ([20, 0, 0], [0, 30, 0]),
                // 基準が情報なら、情報の 50% はそのまま誤検知
                ([20, 0, 0], [30, 0, 0]),
                // 基準が重大なら、警告の 50% は軽い指摘
                ([0, 20, 0], [0, 0, 30]),
                // 警告以上で 2.5% と 0%: 目標以下だが見分けられていない
                ([0, 1, 0], [0, 0, 0]),
                // 重大度を問わなくても 5% ちょうど: 見直さない
                ([1, 1, 0], [0, 30, 0]),
            ],
        );
        let report = synthetic_report(&evaluated, &docs);
        let reasons: Vec<(&str, ReviewReason, Severity)> = report
            .reviews
            .iter()
            .map(|r| (r.rule_id, r.reason, r.calibration_basis))
            .collect();
        assert_eq!(
            reasons,
            vec![
                ("A01", FalsePositives, Warning),
                ("A02", InfoBurden, Warning),
                ("A03", FalsePositives, Info),
                ("A04", InfoBurden, Error),
                ("A05", NoSeparation, Warning),
            ]
        );
        let a02 = &report.reviews[1];
        assert_eq!(
            (a02.outcome.at_least, a02.outcome.human_fired),
            (Warning, 0)
        );
        assert_eq!(
            (a02.any_severity.at_least, a02.any_severity.human_fired),
            (Info, 20)
        );
        let a04 = &report.reviews[3];
        assert_eq!((a04.outcome.at_least, a04.outcome.ai_fired), (Error, 30));
        assert_eq!(report.rules[3].calibration_basis, Error);

        let mut text = Vec::new();
        render_text(&report, &mut text).unwrap();
        let text = String::from_utf8(text).unwrap();
        let burden = text.find("軽い指摘が多い").unwrap();
        let separation = text.find("見分けられていない").unwrap();
        assert!(text.find("誤検知が目標を超える").unwrap() < separation);
        assert!(separation < burden, "{text}");
        assert!(
            text.contains(
                "A02 A02: 誤検知 (警告以上) 0.0% (0/40) は目標の 5.0% 以下ですが、重大度を問わないと 50.0% (20/40) です"
            ),
            "{text}"
        );
        assert!(
            text.contains("A01 A01: 誤検知 (警告以上) 20.0% (8/40) が目標の 5.0% を超えています"),
            "{text}"
        );
        assert!(
            text.contains("人の文書の内訳: 情報 0・警告 20・重大 0"),
            "{text}"
        );
        assert!(text.contains("直ちに実験的に戻す話ではなく"), "{text}");
    }

    #[test]
    fn promotions_use_the_rate_of_every_severity() {
        use Severity::*;
        let evaluated = [
            // 情報だけのルール: 警告以上で数えると人も生成も 0% に見える
            judged(test_meta!("B01", RuleStatus::Experimental, Info), Info),
            // 警告のルール: 人の文書で 1 件も出ない
            judged(
                test_meta!("B02", RuleStatus::Experimental, Warning),
                Warning,
            ),
            // 警告のルールでも、情報の指摘で人の文書に多く出るなら上げない
            judged(
                test_meta!("B03", RuleStatus::Experimental, Warning),
                Warning,
            ),
        ];
        let per_rule = [
            ([30, 0, 0], [50, 0, 0]),
            ([0, 0, 0], [0, 40, 0]),
            ([30, 0, 0], [0, 40, 0]),
        ];
        let report = synthetic_report(&evaluated, &severity_docs(60, &per_rule));
        let promoted: Vec<(&str, bool)> = report
            .promotions
            .iter()
            .map(|p| (p.rule_id, p.small_sample))
            .collect();
        // 60 件で 0 件なら、95% 上限 (4.3%) も目標以下
        assert_eq!(promoted, vec![("B02", false)]);
        let p = &report.promotions[0];
        assert_eq!(p.outcome.at_least, Info);
        assert!(p.outcome.false_positive_upper95.unwrap() <= 0.05);

        // 20 件では、0 件でも 95% 上限が 11.9% で目標を超える
        let report = synthetic_report(&evaluated, &severity_docs(20, &per_rule));
        let promoted: Vec<(&str, bool)> = report
            .promotions
            .iter()
            .map(|p| (p.rule_id, p.small_sample))
            .collect();
        assert_eq!(promoted, vec![("B02", true)]);
        let mut text = Vec::new();
        render_text(&report, &mut text).unwrap();
        let text = String::from_utf8(text).unwrap();
        assert!(
            text.contains("件数が少なく、目標の 5.0% 以下と言い切れません (95% 上限 11.9%)"),
            "{text}"
        );
        assert!(text.contains("最低 52 件要ります"), "{:?}", report.warnings);
        let mut json = Vec::new();
        render_json(&report, &mut json).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value["promotions"][0]["smallSample"], true);
        assert_eq!(value["promotions"][0]["outcome"]["atLeast"], "info");
    }

    // -----------------------------------------------------------------------
    // 閾値の選び方
    // -----------------------------------------------------------------------

    static STUB: RuleMeta = RuleMeta {
        id: "T01",
        name: "STUB",
        title: "テスト用",
        lane: Lane::Slop,
        status: RuleStatus::Experimental,
        default_severity: Severity::Info,
        summary: "テスト用",
        explanation: "",
    };

    fn stub(current: f64) -> Evaluated {
        Evaluated {
            entry: 0,
            meta: &STUB,
            mode: Mode::Experimental,
            builtin: true,
            options: vec![("k", current)],
            basis: Severity::Info,
        }
    }

    fn synthetic(docs: &[(Group, Split, f64)], direction: Fires) -> Vec<DocResult> {
        docs.iter()
            .map(|&(group, split, value)| DocResult {
                group,
                split,
                characters: 1000,
                line_per_paragraph: false,
                hits: vec![Hits::default()],
                measures: vec![vec![Measure::new("m", value, "k", direction)]],
                items: HashMap::new(),
            })
            .collect()
    }

    fn calibration(group: Group, values: &[f64]) -> Vec<(Group, Split, f64)> {
        values
            .iter()
            .map(|&v| (group, Split::Calibration, v))
            .collect()
    }

    #[test]
    fn integer_thresholds_are_rounded_toward_the_firing_side() {
        let mut spec = calibration(
            Group::Human,
            &[1.0, 1.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0, 4.0, 5.0],
        );
        spec.extend(calibration(
            Group::Ai,
            &[3.0, 4.0, 4.0, 5.0, 5.0, 5.0, 6.0, 6.0, 7.0, 8.0],
        ));
        spec.push((Group::Human, Split::Holdout, 6.0));
        spec.push((Group::Ai, Split::Holdout, 6.0));
        let docs = synthetic(&spec, Fires::AtOrAbove);
        let key = ("k", "m", Fires::AtOrAbove, None);
        let s = sweep(&docs, 0, &stub(3.0), key, 0.1);
        let current = s.current.unwrap();
        assert_eq!(
            (
                current.calibration.human_fired,
                current.calibration.ai_fired
            ),
            (5, 10)
        );
        let suggested = s.suggested.unwrap();
        // 5 以上で指摘: 人 1/10 (目標 10% ちょうど)、生成 7/10
        assert_eq!(suggested.threshold, 5.0);
        assert_eq!(
            (
                suggested.calibration.human_fired,
                suggested.calibration.ai_fired
            ),
            (1, 7)
        );
        assert_eq!(
            (suggested.holdout.human_fired, suggested.holdout.human_docs),
            (1, 1)
        );
        assert_eq!((s.measured_human, s.measured_ai), (11, 11));
        assert_eq!(s.switches_to, None);
        assert_eq!(suggested.all.at_least, Severity::Info);
    }

    #[test]
    fn float_thresholds_use_few_digits() {
        let mut spec = calibration(
            Group::Human,
            &[0.30, 0.35, 0.40, 0.45, 0.50, 0.55, 0.60, 0.65, 0.70, 0.75],
        );
        spec.extend(calibration(
            Group::Ai,
            &[0.05, 0.10, 0.15, 0.20, 0.22, 0.24, 0.26, 0.28, 0.32, 0.36],
        ));
        let docs = synthetic(&spec, Fires::Below);
        let s = sweep(&docs, 0, &stub(0.2), ("k", "m", Fires::Below, None), 0.1);
        let suggested = s.suggested.unwrap();
        // 0.34 未満で指摘: 人 1/10、生成 9/10
        assert_eq!(num(suggested.threshold), "0.34");
        assert_eq!(
            (
                suggested.calibration.human_fired,
                suggested.calibration.ai_fired
            ),
            (1, 9)
        );
    }

    #[test]
    fn ties_keep_the_value_closest_to_the_current_one() {
        let key = ("k", "m", Fires::AtOrAbove, None);
        let mut spec = calibration(Group::Human, &[1.0; 10]);
        spec.extend(calibration(Group::Ai, &[9.0; 10]));
        let docs = synthetic(&spec, Fires::AtOrAbove);
        let s = sweep(&docs, 0, &stub(8.0), key, 0.05);
        assert_eq!(s.suggested.unwrap().threshold, 8.0);
        // 目標を満たす値がなければ提案しない
        let mut spec = calibration(Group::Human, &[5.0; 10]);
        spec.extend(calibration(Group::Ai, &[5.0; 10]));
        let docs = synthetic(&spec, Fires::AtOrAbove);
        let s = sweep(&docs, 0, &stub(8.0), key, 0.05);
        assert!(s.suggested.is_none());
    }

    #[test]
    fn severity_switch_sweeps_count_documents_at_that_severity() {
        let mut spec = calibration(Group::Human, &[0.01, 0.02, 0.025, 0.05]);
        spec.extend(calibration(Group::Ai, &[0.04, 0.06, 0.08, 0.1]));
        let mut docs = synthetic(&spec, Fires::AtOrAbove);
        for d in &mut docs {
            let m = d.measures[0].pop().unwrap();
            d.measures[0].push(m.at(Severity::Error));
        }
        let keys = sweep_keys(&docs, 0, &stub(0.03));
        assert_eq!(
            keys,
            vec![("k", "m", Fires::AtOrAbove, Some(Severity::Error))]
        );
        let s = sweep(&docs, 0, &stub(0.03), keys[0], 0.25);
        assert_eq!(s.switches_to, Some(Severity::Error));
        let current = s.current.unwrap();
        assert_eq!(current.all.at_least, Severity::Error);
        assert_eq!((current.all.human_fired, current.all.ai_fired), (1, 4));
        assert_eq!(fires_label(s.direction, s.switches_to), "閾値以上で重大");
        assert_eq!(fires_label(Fires::Below, None), "閾値未満で指摘");
    }

    #[test]
    fn candidate_helpers() {
        assert_eq!(between(3.0, 4.0, Fires::AtOrAbove, true), 4.0);
        assert_eq!(between(3.0, 4.0, Fires::Above, true), 3.0);
        assert_eq!(between(3.0, 5.0, Fires::Below, true), 4.0);
        assert_eq!(between(3.0, 4.0, Fires::AtOrBelow, true), 3.0);
        assert_eq!(round_between(-0.42, -0.36), -0.4);
        assert_eq!(round_between(0.341, 0.359), 0.35);
        assert_eq!(everything(90.0, 130.0, Fires::Above, true), 89.0);
        assert_eq!(everything(-0.9, -0.2, Fires::Below, false), 0.0);
        assert_eq!(num(0.1 + 0.2), "0.3");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    // -----------------------------------------------------------------------
    // 校正用と検証用の分け方
    // -----------------------------------------------------------------------

    #[test]
    fn split_is_deterministic_and_close_to_the_requested_share() {
        let keys: Vec<String> = (0..2000).map(|i| format!("topic-{i:04}.md")).collect();
        let first: Vec<bool> = keys.iter().map(|k| split_point(k) < 0.3).collect();
        let second: Vec<bool> = keys.iter().map(|k| split_point(k) < 0.3).collect();
        assert_eq!(first, second);
        let share = first.iter().filter(|&&h| h).count() as f64 / keys.len() as f64;
        assert!((0.26..=0.34).contains(&share), "{share}");
    }

    #[test]
    fn split_keys_are_relative_to_the_input() {
        assert_eq!(
            relative_key(Path::new("corpus/human"), Path::new("corpus/human/a/b.md")),
            "a/b.md"
        );
        assert_eq!(
            relative_key(Path::new("./corpus/human"), Path::new("corpus/human/b.md")),
            "b.md"
        );
        assert_eq!(relative_key(Path::new("."), Path::new("a/b.md")), "a/b.md");
        assert_eq!(
            relative_key(Path::new("corpus/one.md"), Path::new("corpus/one.md")),
            "one.md"
        );
    }

    // -----------------------------------------------------------------------
    // コーパス全体
    // -----------------------------------------------------------------------

    fn write_docs(dir: &Path, docs: impl IntoIterator<Item = String>) {
        fs::create_dir_all(dir).unwrap();
        for (i, text) in docs.into_iter().enumerate() {
            fs::write(dir.join(format!("doc-{i:02}.md")), text).unwrap();
        }
    }

    fn only(ids: &[&str]) -> EngineOptions {
        EngineOptions {
            selection: Selection {
                only: Some(ids.iter().map(|s| s.to_string()).collect()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn corpus_options(root: &Path, engine: EngineOptions) -> CalibrateOptions {
        CalibrateOptions {
            engine,
            ..CalibrateOptions::new(vec![root.join("human")], vec![root.join("ai")])
        }
    }

    /// 同じ長さの文を並べた段落。
    fn lengths(lengths: &[usize]) -> String {
        lengths
            .iter()
            .map(|&n| format!("{}。", "文".repeat(n)))
            .collect::<String>()
            + "\n"
    }

    /// 文長がばらつく本文 (R01 は出ない)。
    fn varied_lengths(i: usize) -> String {
        let pattern = [4, 60, 25, 12, 40];
        lengths(
            &(0..24)
                .map(|j| pattern[(i + j) % pattern.len()])
                .collect::<Vec<_>>(),
        )
    }

    /// 文長がそろった本文 (R01 が警告する)。
    fn uniform_lengths(i: usize) -> String {
        (0..22)
            .map(|j| {
                format!(
                    "担当者が手順書の{}番目の項目を読んで確かめた。",
                    (i + j) % 10
                )
            })
            .collect::<String>()
            + "\n"
    }

    #[test]
    fn measures_rates_and_sweeps_thresholds_on_a_corpus() {
        let dir = tempfile::tempdir().unwrap();
        write_docs(&dir.path().join("human"), (0..12).map(varied_lengths));
        write_docs(&dir.path().join("ai"), (0..12).map(uniform_lengths));
        let report = calibrate(&corpus_options(dir.path(), only(&["R01"]))).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.corpus.human.docs, 12);
        assert_eq!(report.corpus.ai.calibration + report.corpus.ai.holdout, 12);

        assert_eq!(report.rules.len(), 1);
        let r01 = &report.rules[0];
        assert_eq!((r01.id, r01.mode), ("R01", Mode::Default));
        assert_eq!(r01.calibration_basis, Severity::Warning);
        assert_eq!((r01.human.fired, r01.ai.fired), (0, 12));
        assert_eq!((r01.human.rate, r01.ai.rate), (Some(0.0), Some(1.0)));
        assert_eq!(r01.human.per_1000_chars, Some(0.0));
        assert!(r01.ai.per_1000_chars.unwrap() > 0.0);
        // 22 文の文書なので、どれも警告として出る
        assert_eq!(
            r01.ai.by_severity.warning,
            SeverityCount {
                fired: 12,
                hits: 12
            }
        );
        assert_eq!(r01.ai.by_severity.info.fired, 0);
        assert_eq!(
            (r01.ai.at_least.warning.fired, r01.ai.at_least.error.fired),
            (12, 0)
        );
        assert!((r01.human.upper95.unwrap() - wilson_upper(0, 12).unwrap()).abs() < 1e-12);

        let sweep = report
            .thresholds
            .iter()
            .find(|t| t.key == "threshold")
            .unwrap();
        assert_eq!((sweep.rule_id, sweep.measure), ("R01", "burstiness"));
        assert_eq!((sweep.measured_human, sweep.measured_ai), (12, 12));
        let current = sweep.current.as_ref().unwrap();
        assert_eq!(current.threshold, -0.38);
        assert_eq!((current.all.human_fired, current.all.ai_fired), (0, 12));
        // 現在の閾値で誤検知 0・検出 100% なので、動かす理由がない
        assert_eq!(sweep.suggested.as_ref().unwrap().threshold, -0.38);
        assert!(report.promotions.is_empty());
        assert!(report.reviews.is_empty());
        assert!(!report.warnings.iter().any(|w| w.contains("1 群")));

        let mut text = Vec::new();
        render_text(&report, &mut text).unwrap();
        let text = String::from_utf8(text).unwrap();
        assert!(text.contains("LOW_BURSTINESS"), "{text}");
        assert!(text.contains("提案 現在の値のまま"), "{text}");
        // 表は割合だけにして、端末の幅に収める
        let row = text
            .lines()
            .find(|l| l.trim_start().starts_with("R01"))
            .unwrap();
        assert!(row.width() <= 100, "{row}");
        assert!(row.contains("警告"), "{row}");
    }

    /// 「ではなく」の対比を 3 回含む人の文書 (R05 が重大として出る)。
    fn contrast_doc(i: usize) -> String {
        format!(
            "これは通知ではなく、仕組みだ。\n\n手順ではなく、考え方を示す。\n\n速さだけでなく、正確さも求められる。\n\n補足の文をここに{i}つ置いた。\n"
        )
    }

    /// 接続詞で始まる段落と「過言ではない」を含む生成文書 (R09 と P01 の実験的な項目が出る)。
    fn conjunction_doc(i: usize) -> String {
        format!(
            "最初の段落に{i}番目の結論を書く。\n\nしかし、例外もある。\n\nまた、費用の問題もある。\n\nこれは革命と言っても過言ではない。\n"
        )
    }

    fn promotion_report() -> CalibrationReport {
        let dir = tempfile::tempdir().unwrap();
        write_docs(&dir.path().join("human"), (0..10).map(contrast_doc));
        write_docs(&dir.path().join("ai"), (0..10).map(conjunction_doc));
        calibrate(&corpus_options(dir.path(), only(&["R05", "R09", "P01"]))).unwrap()
    }

    /// P01 の実験的な正規表現の項目。
    const OVERSTATEMENT: &str = "/(?:と言っても)?過言では(?:ない|ありません)/";

    #[test]
    fn lists_promotion_candidates_and_rules_to_review() {
        let report = promotion_report();
        let promoted: Vec<(&str, Option<&str>, bool)> = report
            .promotions
            .iter()
            .map(|p| {
                (
                    p.rule_id,
                    p.item.as_ref().map(|i| i.item.as_str()),
                    p.small_sample,
                )
            })
            .collect();
        // 人の文書 10 件で 0 件では、95% 上限が 21.3% なので注記が付く
        assert!(promoted.contains(&("R09", None, true)), "{promoted:?}");
        assert!(
            promoted.contains(&("P01", Some(OVERSTATEMENT), true)),
            "{promoted:?}"
        );
        assert_eq!(promoted.len(), 2, "{promoted:?}");

        // R05 は重大になる割合で校正したので、重大の指摘で数えて見直す
        let reviews: Vec<(&str, ReviewReason, Severity)> = report
            .reviews
            .iter()
            .map(|r| (r.rule_id, r.reason, r.calibration_basis))
            .collect();
        assert_eq!(
            reviews,
            vec![("R05", ReviewReason::FalsePositives, Severity::Error)]
        );
        let r05 = report.rules.iter().find(|r| r.id == "R05").unwrap();
        assert_eq!(r05.human.at_least.error.fired, 10);
        let sweep = report
            .thresholds
            .iter()
            .find(|t| t.key == "error_above")
            .unwrap();
        assert_eq!(sweep.switches_to, Some(Severity::Error));
        assert_eq!(sweep.current.as_ref().unwrap().all.human_fired, 10);

        let item = report
            .items
            .iter()
            .find(|i| i.item == OVERSTATEMENT)
            .unwrap();
        assert_eq!(item.status, RuleStatus::Experimental);
        assert_eq!(item.examples, vec!["と言っても過言ではない"]);
        assert_eq!(
            (item.severity, item.human.fired, item.ai.fired),
            (Severity::Info, 0, 10)
        );
        assert_eq!(item.ai.by_severity.info.hits, 10);
        let after = item.promoted_rule.as_ref().unwrap();
        assert_eq!(
            (after.at_least, after.human_fired, after.ai_fired),
            (Severity::Info, 0, 10)
        );
        // P01 の校正済みの項目は、どちらの文書にも出ていない
        let p01 = report.rules.iter().find(|r| r.id == "P01").unwrap();
        assert_eq!((p01.human.fired, p01.ai.fired), (0, 0));
    }

    #[test]
    fn regex_items_are_counted_as_one_item_with_example_notations() {
        let dir = tempfile::tempdir().unwrap();
        let human = (0..10).map(|i| match i % 3 {
            0 => format!("ここでは{i}件目の記録を読むことができる。\n"),
            1 => format!("{i}件目の記録は来週に書くことが出来ます。\n"),
            _ => format!("{i}件目の記録は見送った。\n"),
        });
        write_docs(&dir.path().join("human"), human);
        write_docs(
            &dir.path().join("ai"),
            (0..10).map(|i| format!("{i}件目の本文を書いた。\n")),
        );
        let report = calibrate(&corpus_options(dir.path(), only(&["P12"]))).unwrap();
        let rows: Vec<&ItemStats> = report.items.iter().filter(|i| i.rule_id == "P12").collect();
        assert_eq!(rows.len(), 1, "{rows:?}");
        let row = rows[0];
        // 正規表現の項目は、表記が違っても `/パターン/` の名前で 1 行にまとまる
        assert!(
            row.item.starts_with("/こと(?:が|は)(?:でき|出来)") && row.item.ends_with('/'),
            "{}",
            row.item
        );
        assert_eq!(row.examples, vec!["ことができる", "ことが出来ます"]);
        assert_eq!(row.severity, Severity::Info);
        assert_eq!((row.human.fired, row.human.by_severity.info.fired), (7, 7));
        assert_eq!(row.human.docs, 10);

        // P12 は既定の重大度が情報なので、情報の指摘で数えて見直す
        let review = &report.reviews[0];
        assert_eq!(
            (review.rule_id, review.reason, review.calibration_basis),
            ("P12", ReviewReason::FalsePositives, Severity::Info)
        );
        assert_eq!(review.human_items[0].item, row.item);
        let mut text = Vec::new();
        render_text(&report, &mut text).unwrap();
        let text = String::from_utf8(text).unwrap();
        assert!(
            text.contains(&format!(
                "「{}」(例「ことができる」「ことが出来ます」、情報、人 7 件・生成 0 件)",
                row.item
            )),
            "{text}"
        );
    }

    #[test]
    fn weak_items_alone_make_a_light_burden_review() {
        // P02 の「さて、」は人の文書にも出る弱い手掛かり (情報)。基準は警告以上
        let dir = tempfile::tempdir().unwrap();
        write_docs(
            &dir.path().join("human"),
            (0..10).map(|i| format!("さて、{i}件目の話を始める。\n")),
        );
        write_docs(
            &dir.path().join("ai"),
            (0..10).map(|i| format!("それでは、{i}件目の手順を見ていきましょう。\n")),
        );
        let report = calibrate(&corpus_options(dir.path(), only(&["P02"]))).unwrap();
        let review = &report.reviews[0];
        assert_eq!(
            (review.rule_id, review.reason, review.calibration_basis),
            ("P02", ReviewReason::InfoBurden, Severity::Warning)
        );
        assert_eq!(
            (review.outcome.human_fired, review.any_severity.human_fired),
            (0, 10)
        );
        let items: Vec<(&str, Severity)> = review
            .human_items
            .iter()
            .map(|i| (i.item.as_str(), i.severity))
            .collect();
        assert_eq!(items, vec![("さて、", Severity::Info)]);
        let mut md = Vec::new();
        render_markdown(&report, &mut md).unwrap();
        let md = String::from_utf8(md).unwrap();
        assert!(md.contains("### 軽い指摘が多い"), "{md}");
        assert!(md.contains("「さて、」(情報、人 10 件・生成 0 件)"), "{md}");
    }

    #[test]
    fn experimental_items_are_skipped_when_not_requested() {
        let dir = tempfile::tempdir().unwrap();
        write_docs(&dir.path().join("human"), (0..10).map(contrast_doc));
        write_docs(&dir.path().join("ai"), (0..10).map(conjunction_doc));
        let mut options = corpus_options(dir.path(), EngineOptions::default());
        options.include_experimental = false;
        let report = calibrate(&options).unwrap();
        assert!(report.rules.iter().all(|r| r.mode == Mode::Default));
        assert!(report.rules.iter().all(|r| r.status == RuleStatus::Stable));
        assert!(report.items.iter().all(|i| i.status == RuleStatus::Stable));
        assert!(!report.rules.iter().any(|r| r.id == "R09"));
    }

    #[test]
    fn renders_text_markdown_and_json() {
        let report = promotion_report();
        let mut text = Vec::new();
        render(&report, ReportFormat::Text, &mut text).unwrap();
        let text = String::from_utf8(text).unwrap();
        assert!(text.contains("校正済みに上げる候補"), "{text}");
        assert!(
            text.contains("項目「/(?:と言っても)?過言では(?:ない|ありません)/」(例「と言っても過言ではない」、情報、生成 10 件・人 0 件)"),
            "{text}"
        );
        assert!(
            text.contains(
                "R05 ANTITHESIS_REPETITION: 誤検知 (重大) 100.0% (10/10) が目標の 5.0% を超えています"
            ),
            "{text}"
        );
        assert!(
            text.contains("人の文書の内訳: 情報 0・警告 0・重大 10"),
            "{text}"
        );
        assert!(
            text.contains("error_above と比べる (閾値以上で重大)"),
            "{text}"
        );
        assert!(
            text.contains("この閾値が決めるのは重大にするかどうかで"),
            "{text}"
        );
        assert!(text.contains("95% 上限 21.3%"), "{text}");

        let mut md = Vec::new();
        render(&report, ReportFormat::Markdown, &mut md).unwrap();
        let md = String::from_utf8(md).unwrap();
        assert!(md.starts_with("# noslop calibrate の結果"), "{md}");
        assert!(md.contains("| R09 | PARAGRAPH_LEAD_CONJUNCTION"), "{md}");
        assert!(md.contains("| R05 | ANTITHESIS_REPETITION (否定→肯定の対比の反復) | 校正済み | 重大 | 100.0% | 100.0% | 0.0% | 0.0% |"), "{md}");
        assert!(md.contains("## 見直しが必要な校正済みルール"), "{md}");
        assert!(md.contains("### 誤検知が目標を超える"), "{md}");

        let mut json = Vec::new();
        render(&report, ReportFormat::Json, &mut json).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["settings"]["targetFp"], 0.05);
        let review = &value["reviews"][0];
        assert_eq!(review["reason"], "falsePositives");
        assert_eq!(review["calibrationBasis"], "error");
        assert_eq!(review["outcome"]["atLeast"], "error");
        assert_eq!(review["anySeverity"]["atLeast"], "info");
        assert!(review["outcome"]["falsePositiveUpper95"].is_number());
        assert!(
            value["thresholds"]
                .as_array()
                .is_some_and(|t| t.iter().any(|t| t["switchesTo"] == "error"))
        );
        let human = &value["rules"][0]["human"];
        assert!(human["per1000Chars"].is_number());
        assert!(human["upper95"].is_number());
        assert!(human["bySeverity"]["warning"]["fired"].is_number());
        assert!(human["atLeast"]["error"]["upper95"].is_number());
        assert!(value["items"][0]["examples"].is_array());
        assert!(value["promotions"][0]["smallSample"].is_boolean());
    }

    #[test]
    fn warns_on_small_corpora_and_reports_unreadable_files() {
        let dir = tempfile::tempdir().unwrap();
        write_docs(&dir.path().join("human"), ["人の文書。\n".to_string()]);
        fs::write(dir.path().join("human/broken.md"), [0xff, 0xfe, 0x00]).unwrap();
        write_docs(
            &dir.path().join("ai"),
            (0..3).map(|i| format!("生成文書{i}。\n")),
        );
        let mut options = corpus_options(dir.path(), EngineOptions::default());
        options.ai.push(dir.path().join("missing"));
        let report = calibrate(&options).unwrap();
        assert_eq!(report.corpus.human.docs, 1);
        assert_eq!(report.corpus.ai.docs, 3);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("人の文書が 1 件")),
            "{:?}",
            report.warnings
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("生成文書が 3 件")),
            "{:?}",
            report.warnings
        );
        // 1 件では 0 件指摘でも 95% 上限は 73.0% で、目標 5% には 52 件要る
        assert!(
            report.warnings.iter().any(|w| w.contains(
                "人の文書が 1 件では、1 件も指摘されなくても誤検知率の 95% 上限が 73.0% になり"
            ) && w.contains("最低 52 件要ります")),
            "{:?}",
            report.warnings
        );
        let errors: Vec<(Group, bool)> = report
            .errors
            .iter()
            .map(|e| (e.group, e.message.contains("UTF-8")))
            .collect();
        assert_eq!(errors, vec![(Group::Human, true), (Group::Ai, false)]);
    }

    #[test]
    fn rejects_invalid_options() {
        let base = CalibrateOptions::new(vec!["h".into()], vec!["a".into()]);
        for options in [
            CalibrateOptions {
                holdout: 1.0,
                ..base.clone()
            },
            CalibrateOptions {
                target_fp: 1.5,
                ..base.clone()
            },
            CalibrateOptions {
                min_detection: f64::NAN,
                ..base.clone()
            },
            CalibrateOptions {
                human: Vec::new(),
                ..base.clone()
            },
        ] {
            assert!(matches!(calibrate(&options), Err(ConfigError::Invalid(_))));
        }
        let unknown = CalibrateOptions {
            engine: only(&["NOPE"]),
            ..base
        };
        assert!(calibrate(&unknown).is_err());
    }

    #[test]
    fn documents_without_blank_lines_between_paragraphs_are_warned_about() {
        // 1 行 1 段落で空行のない文書を Markdown として置くと、全体が 1 段落にまとまる
        let line_per_paragraph: String = (0..12)
            .map(|i| format!("{i}番目の段落の文だ。\n"))
            .collect();
        // 段落のあいだに空行を置いた文書と、1 文ずつ改行して段落を空行で区切った文書は正しい
        let with_blank_lines: String = (0..12)
            .map(|i| format!("{i}番目の段落の文だ。\n\n"))
            .collect();
        let semantic_breaks: String = (0..6)
            .map(|i| format!("{i}番目の段落の最初の文だ。\n次の文も同じ段落に書く。\n\n"))
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_docs(
            &root.join("human"),
            [
                line_per_paragraph.clone(),
                with_blank_lines,
                semantic_breaks,
            ],
        );
        write_docs(&root.join("ai"), [contrast_doc(0)]);
        // 同じ内容でもテキスト (.txt) は 1 行 1 段落として読むので警告しない
        fs::write(root.join("ai/plain.txt"), &line_per_paragraph).unwrap();

        let report = calibrate(&corpus_options(root, EngineOptions::default())).unwrap();
        let warned: Vec<&String> = report
            .warnings
            .iter()
            .filter(|w| w.contains("段落のあいだに空行がない"))
            .collect();
        assert_eq!(warned.len(), 1, "{:?}", report.warnings);
        assert!(warned[0].starts_with("人の文書の 1 件は"), "{}", warned[0]);
    }

    #[test]
    fn corpora_under_a_gitignored_directory_are_still_read() {
        // コーパスはコミットしないので .gitignore で除外する。入力に直接指定したディレクトリは読む
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "/corpus/\n*.part\n").unwrap();
        let root = dir.path().join("corpus");
        write_docs(&root.join("human"), (0..2).map(contrast_doc));
        write_docs(&root.join("ai"), (0..2).map(conjunction_doc));
        fs::write(root.join("ai/.doc-09.md.part"), "書きかけ。\n").unwrap();
        let report = calibrate(&corpus_options(&root, EngineOptions::default())).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!((report.corpus.human.docs, report.corpus.ai.docs), (2, 2));

        // ファイルに当たる書き方だと中身も除外されるので、集まらなかったことを知らせる
        fs::write(dir.path().join(".gitignore"), "/corpus/**\n").unwrap();
        let report = calibrate(&corpus_options(&root, EngineOptions::default())).unwrap();
        assert_eq!((report.corpus.human.docs, report.corpus.ai.docs), (0, 0));
        let groups: Vec<Group> = report
            .errors
            .iter()
            .filter(|e| e.message.contains("検査できるファイルがありません"))
            .map(|e| e.group)
            .collect();
        assert_eq!(groups, vec![Group::Human, Group::Ai]);
    }
}
