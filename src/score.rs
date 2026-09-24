//! 自然度スコア。0〜100 の値で、大きいほど AI 臭さの指摘が少ない。
//!
//! 算式 v1:
//!
//! ```text
//! 減点 = (error×8 + warning×4 + info×0.5) × (1000 / max(原文の文字数, 1000))
//! スコア = max(100 − 減点, 20)
//! ```
//!
//! 対象にするのは、未抑制・AI 臭さのレーン・校正済み (stable)・組み込みルールの
//! 指摘だけ。読みやすさの指摘、実験的なルール・語句、独自ルールは入れない
//! (別目的の指標や未校正の指標が同じ数字に乗ると、どちらの判断も濁るため)。
//! 文字数で割り戻すので、短い文書では 1 件あたりの減点が大きく、長い文書では指摘の密度が効く。
//!
//! あくまで目安で、CI の合否判定には使わない。ルールの追加や閾値の変更で値が
//! 変わるため、比べるときは算式の版 ([`FORMULA`]) と noslop の版をそろえる。

use serde::Serialize;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};

/// 算式の版。
pub const FORMULA: &str = "v1";

/// この文字数未満の文書にはスコアを付けない (診断の対象として短すぎる)。
pub const MIN_CHARACTERS: usize = 100;

/// スコアの帯。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Band {
    /// 90〜100: AI 臭さはほぼ見当たらない。
    Natural,
    /// 70〜89: 数カ所に癖が残る。
    Minor,
    /// 50〜69: 表層か構造に明確な AI 臭さがある。
    NeedsWork,
    /// 0〜49: 複数のカテゴリで強く検出。
    Heavy,
}

impl Band {
    pub fn from_value(value: u32) -> Self {
        match value {
            90.. => Band::Natural,
            70..=89 => Band::Minor,
            50..=69 => Band::NeedsWork,
            _ => Band::Heavy,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Band::Natural => "natural",
            Band::Minor => "minor",
            Band::NeedsWork => "needs-work",
            Band::Heavy => "heavy",
        }
    }

    /// 日本語のラベル。
    pub fn label_ja(self) -> &'static str {
        match self {
            Band::Natural => "自然",
            Band::Minor => "軽微",
            Band::NeedsWork => "要修正",
            Band::Heavy => "濃厚",
        }
    }
}

/// 自然度スコア。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Score {
    pub value: u32,
    pub band: Band,
    /// 減点の合計 (丸める前)。
    pub deduction: f64,
}

/// スコアに入れる指摘か。
pub fn counts(d: &Diagnostic, is_builtin: impl Fn(&str) -> bool) -> bool {
    !d.is_suppressed()
        && d.lane == Lane::Slop
        && d.status == RuleStatus::Stable
        && is_builtin(&d.rule_id)
}

/// スコアを計算する。`diagnostics` はスコアに入れる指摘だけを渡す。
pub fn compute<'a>(
    characters: usize,
    diagnostics: impl IntoIterator<Item = &'a Diagnostic>,
) -> Option<Score> {
    if characters < MIN_CHARACTERS {
        return None;
    }
    let weight: f64 = diagnostics
        .into_iter()
        .map(|d| match d.severity {
            Severity::Error => 8.0,
            Severity::Warning => 4.0,
            Severity::Info => 0.5,
        })
        .sum();
    let deduction = weight * (1000.0 / characters.max(1000) as f64);
    let value = (100.0 - deduction).max(20.0).round() as u32;
    Some(Score {
        value,
        band: Band::from_value(value),
        deduction,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Span;

    fn diag(severity: Severity) -> Diagnostic {
        Diagnostic::new(
            "P01",
            "AI_CONCLUSION",
            severity,
            Lane::Slop,
            RuleStatus::Stable,
            Span::new(0, 1),
            "m",
        )
    }

    #[test]
    fn deduction_is_normalized_by_length() {
        let d = vec![diag(Severity::Warning), diag(Severity::Info)];
        // 1000 字未満は 1000 字として扱う: 100 - 4.5
        let s = compute(500, &d).unwrap();
        assert_eq!(s.value, 96);
        assert_eq!(s.band, Band::Natural);
        // 2000 字なら半分の重み: 100 - 2.25
        assert_eq!(compute(2000, &d).unwrap().value, 98);
    }

    #[test]
    fn score_has_a_floor_and_short_documents_have_no_score() {
        let many: Vec<_> = (0..30).map(|_| diag(Severity::Error)).collect();
        let s = compute(1000, &many).unwrap();
        assert_eq!(s.value, 20);
        assert_eq!(s.band, Band::Heavy);
        assert!(compute(99, &many).is_none());
        assert_eq!(compute(100, &[] as &[Diagnostic]).unwrap().value, 100);
    }

    #[test]
    fn bands_follow_the_documented_ranges() {
        assert_eq!(Band::from_value(90), Band::Natural);
        assert_eq!(Band::from_value(89), Band::Minor);
        assert_eq!(Band::from_value(70), Band::Minor);
        assert_eq!(Band::from_value(69), Band::NeedsWork);
        assert_eq!(Band::from_value(50), Band::NeedsWork);
        assert_eq!(Band::from_value(49), Band::Heavy);
    }

    #[test]
    fn only_unsuppressed_stable_slop_builtin_diagnostics_count() {
        let builtin = |id: &str| id != "X01";
        let base = diag(Severity::Warning);
        assert!(counts(&base, builtin));
        let mut readability = base.clone();
        readability.lane = Lane::Readability;
        assert!(!counts(&readability, builtin));
        let mut experimental = base.clone();
        experimental.status = RuleStatus::Experimental;
        assert!(!counts(&experimental, builtin));
        let mut suppressed = base.clone();
        suppressed.suppressed = Some(crate::diagnostic::Suppression {
            reason: None,
            line: 1,
        });
        assert!(!counts(&suppressed, builtin));
        let mut custom = base;
        custom.rule_id = "X01".into();
        assert!(!counts(&custom, builtin));
    }
}
