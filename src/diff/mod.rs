//! 改稿の前後の比較 (`noslop diff`)。
//!
//! 同じ設定のエンジンで改稿前と改稿後の文書を検査し、次の 3 つを確認事項として出す。
//!
//! 1. 指摘の変化: 新規・書き換えても残った・継続・解消・抑制して残した (fingerprint を多重集合
//!    として突き合わせる)
//! 2. 事実の変化: 数字・日付・固有名詞らしい語・引用・URL の消失と追加 ([`facts`])
//! 3. 改稿の偏り: 同じ変換を文書全体に一律に当てた形跡 ([`shifts`])
//!
//! どれも改稿を失敗扱いにするものではない。指摘の件数やスコアの改善を目的にした
//! 書き直し (新しい均一さを生む) を、書き手が自分で見直すための材料にする。

pub mod facts;
mod render;
pub mod shifts;

use std::collections::{BTreeMap, HashSet, VecDeque};

use crate::diagnostic::Diagnostic;
use crate::document::Document;
use crate::engine::{Engine, FileError, FileReport, Input};

pub use facts::{Fact, FactChange, FactChanges, FactKind};
pub use render::{DIFF_SCHEMA_VERSION, render_json, render_text, render_toon};
pub use shifts::{Shift, ShiftKind};

/// 指摘の変化。値は [`FileReport::diagnostics`] の添字。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FindingChanges {
    /// 改稿後に新しく出た指摘 (改稿後の添字)。優先して見るもの。
    pub new: Vec<usize>,
    /// 文を書き換えても、同じルールの同じ語句で残った指摘 (改稿前の添字, 改稿後の添字)。
    /// 直したつもりの文に残っているもの。
    pub carried_over: Vec<(usize, usize)>,
    /// 前後の両方にある指摘 (改稿前の添字, 改稿後の添字)。同じ文の同じ指摘と、
    /// 文書・見出し単位の指摘 (文脈を持たないもの) で同じルールのもの。
    pub persisting: Vec<(usize, usize)>,
    /// 改稿で消えた指摘 (改稿前の添字)。
    pub resolved: Vec<usize>,
    /// 改稿で抑制コメントを付けて残した指摘 (改稿前の添字, 改稿後の添字)。
    pub suppressed: Vec<(usize, usize)>,
}

/// 比較の結果。
#[derive(Debug)]
pub struct DiffReport {
    pub before: FileReport,
    pub after: FileReport,
    pub findings: FindingChanges,
    pub facts: FactChanges,
    pub shifts: Vec<Shift>,
}

impl DiffReport {
    /// 確認事項 (新しい指摘・事実の変化・改稿の偏り) があるか。
    ///
    /// 改稿前からある指摘 (書き換えても残った・継続) は改稿で生じたものではないので含めない。
    pub fn has_concerns(&self) -> bool {
        !self.findings.new.is_empty()
            || !self.facts.removed.is_empty()
            || !self.facts.added.is_empty()
            || !self.shifts.is_empty()
    }
}

/// 改稿前と改稿後を同じエンジンで検査する。
///
/// どちらかが読めなければ、読めなかった入力をすべて返す (CLI はこれを終了コード 2 にする)。
pub fn lint_pair(
    engine: &Engine,
    before: Input,
    after: Input,
) -> Result<(FileReport, FileReport), Vec<FileError>> {
    let mut report = engine.run(vec![before, after]);
    if !report.errors.is_empty() {
        return Err(report.errors);
    }
    let after = report.files.pop().expect("after report");
    let before = report.files.pop().expect("before report");
    Ok((before, after))
}

/// 検査済みの改稿前・改稿後を比べる。
pub fn compare(before: FileReport, after: FileReport) -> DiffReport {
    let findings = match_findings(&before, &after);
    let facts = facts::compare(&facts::extract(&before.doc), &facts::extract(&after.doc));
    let shifts = shifts::detect(&before.doc, &after.doc);
    DiffReport {
        before,
        after,
        findings,
        facts,
        shifts,
    }
}

/// 前後の指摘を突き合わせる。キーを変えながら、残ったものどうしを順に対応づける。
///
/// 1. fingerprint (ルール ID・一致テキスト・文脈の先頭から作る、行番号に依存しない識別子) が
///    同じもの → 継続。多重集合として扱い、同じ文が繰り返されても件数分だけ 1 対 1 にする
/// 2. 文単位の指摘で、ルールと一致テキスト (空白を除く) が同じもの → 書き換えても残った
/// 3. 文書・見出し単位の指摘 (文脈を持たず、代表位置の文が改稿で変わる) で、同じルールのもの
///    → 継続
/// 4. 改稿後に抑制コメントで隠れた指摘と 1・2 のキーで対応するもの → 抑制して残した
///
/// どれにも対応しなかった改稿前の指摘が解消、改稿後の指摘が新規になる。
fn match_findings(before: &FileReport, after: &FileReport) -> FindingChanges {
    let (db, da) = (&before.diagnostics, &after.diagnostics);
    let indices = |ds: &[Diagnostic], suppressed: bool| -> Vec<usize> {
        (0..ds.len())
            .filter(|&i| ds[i].is_suppressed() == suppressed)
            .collect()
    };
    let mut rest_before = indices(db, false);
    let mut rest_after = indices(da, false);
    let mut suppressed_after = indices(da, true);

    let fingerprint_b = |i: usize| Some(db[i].fingerprint.as_str());
    let fingerprint_a = |j: usize| Some(da[j].fingerprint.as_str());
    let phrase_b = |i: usize| phrase_key(&before.doc, &db[i]);
    let phrase_a = |j: usize| phrase_key(&after.doc, &da[j]);
    let rule_b = |i: usize| db[i].context.is_none().then_some(db[i].rule_id.as_str());
    let rule_a = |j: usize| da[j].context.is_none().then_some(da[j].rule_id.as_str());

    let mut persisting = pair_by(
        &mut rest_before,
        &mut rest_after,
        fingerprint_b,
        fingerprint_a,
    );
    let mut carried_over = pair_by(&mut rest_before, &mut rest_after, phrase_b, phrase_a);
    persisting.extend(pair_by(&mut rest_before, &mut rest_after, rule_b, rule_a));
    let mut suppressed = pair_by(
        &mut rest_before,
        &mut suppressed_after,
        fingerprint_b,
        fingerprint_a,
    );
    suppressed.extend(pair_by(
        &mut rest_before,
        &mut suppressed_after,
        phrase_b,
        phrase_a,
    ));

    let pos_after = |j: usize| (da[j].span.start, da[j].rule_id.as_str());
    let pos_before = |i: usize| (db[i].span.start, db[i].rule_id.as_str());
    rest_after.sort_by_key(|&j| pos_after(j));
    rest_before.sort_by_key(|&i| pos_before(i));
    for pairs in [&mut carried_over, &mut persisting, &mut suppressed] {
        pairs.sort_by_key(|&(_, j)| pos_after(j));
    }
    FindingChanges {
        new: rest_after,
        carried_over,
        persisting,
        resolved: rest_before,
        suppressed,
    }
}

/// 文単位の指摘の、ルールと一致テキスト (空白を除く)。文脈を持たない指摘は `None`。
fn phrase_key<'a>(doc: &Document, d: &'a Diagnostic) -> Option<(&'a str, String)> {
    d.context?;
    let text: String = doc
        .slice(d.span)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    Some((d.rule_id.as_str(), text))
}

/// `before` と `after` の添字を、同じキーどうし出現順に 1 対 1 で対応づける。
///
/// 対応づいた組を返し、`before` と `after` からは取り除く。キーが `None` のものは対応づけない。
fn pair_by<K: Ord>(
    before: &mut Vec<usize>,
    after: &mut Vec<usize>,
    key_before: impl Fn(usize) -> Option<K>,
    key_after: impl Fn(usize) -> Option<K>,
) -> Vec<(usize, usize)> {
    let mut pool: BTreeMap<K, VecDeque<usize>> = BTreeMap::new();
    for &i in before.iter() {
        if let Some(k) = key_before(i) {
            pool.entry(k).or_default().push_back(i);
        }
    }
    let mut pairs = Vec::new();
    after.retain(|&j| {
        let hit = key_after(j)
            .and_then(|k| pool.get_mut(&k))
            .and_then(VecDeque::pop_front);
        if let Some(i) = hit {
            pairs.push((i, j));
        }
        hit.is_none()
    });
    let used: HashSet<usize> = pairs.iter().map(|&(i, _)| i).collect();
    before.retain(|i| !used.contains(i));
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CustomRuleConfig;
    use crate::document::SourceFormat;
    use crate::engine::EngineOptions;

    fn custom(id: &str, pattern: &str) -> CustomRuleConfig {
        CustomRuleConfig {
            id: id.to_string(),
            name: None,
            pattern: pattern.to_string(),
            regex: false,
            message: format!("「{pattern}」があります"),
            hint: None,
            severity: None,
            lane: None,
        }
    }

    /// 組み込みルールに左右されないよう、独自ルールだけのエンジンで比べる。
    fn engine() -> Engine {
        let options = EngineOptions {
            custom: vec![
                custom("X01", "と言えるでしょう"),
                custom("X02", "さて、"),
                custom("X03", "非常に重要"),
            ],
            ..EngineOptions::default()
        };
        Engine::with_rules(Vec::new(), options).expect("engine")
    }

    fn diff(before: &str, after: &str) -> DiffReport {
        let (b, a) = lint_pair(
            &engine(),
            Input::Text {
                name: "before.md".to_string(),
                source: before.to_string(),
                format: SourceFormat::Markdown,
            },
            Input::Text {
                name: "after.md".to_string(),
                source: after.to_string(),
                format: SourceFormat::Markdown,
            },
        )
        .expect("lint");
        compare(b, a)
    }

    #[test]
    fn classifies_new_persisting_and_resolved_even_when_lines_move() {
        let before =
            "さて、本題です。\n\n効果があると言えるでしょう。\n\n手順は簡単だと言えるでしょう。\n";
        // 先頭に段落を足して行をずらし、1 つを直し、1 つを新しく足す
        let after = "# 追記した見出し\n\n冒頭の段落を足した。\n\n効果があると言えるでしょう。\n\n手順は簡単です。\n\n検証は非常に重要だ。\n";
        let r = diff(before, after);
        let ids = |idx: &[usize], report: &FileReport| -> Vec<String> {
            idx.iter()
                .map(|&i| report.diagnostics[i].rule_id.clone())
                .collect()
        };
        assert_eq!(ids(&r.findings.new, &r.after), vec!["X03"]);
        assert_eq!(r.findings.persisting.len(), 1);
        let (i, j) = r.findings.persisting[0];
        assert_eq!(r.before.diagnostics[i].rule_id, "X01");
        assert_eq!(r.after.diagnostics[j].rule_id, "X01");
        assert_eq!(ids(&r.findings.resolved, &r.before), vec!["X02", "X01"]);
        assert!(r.findings.suppressed.is_empty());
    }

    #[test]
    fn repeated_sentences_are_matched_as_a_multiset() {
        let line = "効果があると言えるでしょう。\n\n";
        let before = line.repeat(3);
        let after = line.repeat(2);
        let r = diff(&before, &after);
        assert_eq!(r.findings.persisting.len(), 2);
        assert_eq!(r.findings.resolved.len(), 1);
        assert!(r.findings.new.is_empty());
    }

    #[test]
    fn phrases_left_in_rewritten_sentences_are_carried_over() {
        let before = "効果があると言えるでしょう。\n\nさて、次に進む。\n";
        // 文は書き換えたが「と言えるでしょう」は残し、「さて、」は消した
        let after = "導入の効果は大きいと言えるでしょう。\n\n次に進む。\n";
        let r = diff(before, after);
        assert_eq!(r.findings.carried_over.len(), 1);
        let (i, j) = r.findings.carried_over[0];
        assert_eq!(r.before.diagnostics[i].rule_id, "X01");
        assert_eq!(r.after.diagnostics[j].rule_id, "X01");
        assert_ne!(
            r.before.diagnostics[i].fingerprint,
            r.after.diagnostics[j].fingerprint
        );
        assert!(r.findings.new.is_empty());
        assert!(r.findings.persisting.is_empty());
        assert_eq!(r.findings.resolved.len(), 1);
        assert_eq!(r.before.diagnostics[r.findings.resolved[0]].rule_id, "X02");
        // 改稿前からある指摘は確認事項に数えない
        assert!(!r.has_concerns());
    }

    #[test]
    fn findings_kept_with_a_suppression_comment_are_reported_separately() {
        let before = "効果があると言えるでしょう。\n";
        let after =
            "<!-- noslop-disable-next-line X01 -- 引用のため -->\n効果があると言えるでしょう。\n";
        let r = diff(before, after);
        assert_eq!(r.findings.suppressed.len(), 1);
        assert!(r.findings.resolved.is_empty());
        assert!(r.findings.new.is_empty());
    }

    #[test]
    fn reports_fact_changes_and_shifts() {
        let before = "# 報告\n\n## 背景\n\n問い合わせは40件から11件に減った。\n\n## 課題\n\n担当は Slack で連絡した。\n\n## 提案\n\n本文。\n\n## 今後\n\n本文。\n";
        let after = "# 報告\n\n## 背景: 問い合わせが減った\n\n問い合わせは１１件に減り、満足度は35%上がった。\n\n## 課題: 連絡が遅い\n\n本文。\n\n## 提案: 窓口を一つにする\n\n本文。\n\n## 今後: 月末に見直す\n\n本文。\n";
        let r = diff(before, after);
        let removed: Vec<&str> = r.facts.removed.iter().map(|c| c.key.as_str()).collect();
        assert!(removed.contains(&"40件"), "{removed:?}");
        assert!(removed.contains(&"slack"), "{removed:?}");
        assert!(r.facts.added.iter().any(|c| c.key == "35%" && c.suspicious));
        assert!(!r.facts.added.iter().any(|c| c.key == "11件"));
        assert!(r.shifts.iter().any(|s| s.kind == ShiftKind::HeadingShape));
        assert!(r.has_concerns());
    }

    #[test]
    fn unreadable_inputs_are_returned_as_errors() {
        let missing = std::path::PathBuf::from("/nonexistent/noslop-diff-test.md");
        let err = lint_pair(
            &engine(),
            Input::Path(missing),
            Input::Text {
                name: "after.md".to_string(),
                source: "本文。".to_string(),
                format: SourceFormat::Markdown,
            },
        )
        .expect_err("missing file");
        assert_eq!(err.len(), 1);
    }
}
