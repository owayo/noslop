//! リズム・統計系ルール (`R` で始まる ID)。
//!
//! 文や段落をまたいだ集計で、機械的に均された文章のリズムを拾う。統計の母集団は
//! 地の文 ([`crate::document::Document::prose_sentences`]) で、校正に使った条件
//! (見出し・リスト・引用・表を除いた本文) に合わせている。

mod antithesis;
mod buried_list;
mod burstiness;
mod cleft;
mod commas;
mod duplicates;
mod endings;
mod future_closer;
mod leads;
mod length;
mod overcorrection;
mod paragraphs;
mod self_answer;
mod triads;

use crate::genre::Genre;
use crate::rules::{Rule, RuleMeta, option_f64};
// 各ルールは `super::` から使う (文末と体言止めの判定は `noslop diff` と共有する)
use crate::text::{is_nominal_ending, is_nounish, strip_sentence_end};

/// リズム・統計系の組み込みルール。
pub fn rules(genre: Genre) -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(burstiness::LowBurstiness::new(genre)),
        Box::new(endings::RepetitiveEnding::new(genre)),
        Box::new(length::LongSentence::new(genre)),
        Box::new(buried_list::BuriedList::new(genre)),
        Box::new(antithesis::AntithesisRepetition::new(genre)),
        Box::new(endings::NoNominalEnding::new(genre)),
        Box::new(paragraphs::UniformParagraphs::new(genre)),
        Box::new(leads::RepeatedSentenceLead::new(genre)),
        Box::new(paragraphs::ParagraphLeadConjunction::new(genre)),
        Box::new(cleft::CleftBecause::new(genre)),
        Box::new(self_answer::SelfAnswer::new(genre)),
        Box::new(overcorrection::Overcorrection::new(genre)),
        Box::new(commas::CommaProfile::new(genre)),
        Box::new(duplicates::DuplicatePassage::default()),
        Box::new(future_closer::FormulaicFutureCloser),
        Box::new(triads::RepeatedEvaluativeTriad::default()),
    ]
}

/// 平均と母標準偏差。空なら `None`。
fn mean_sd(values: &[f64]) -> Option<(f64, f64)> {
    if values.is_empty() {
        return None;
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    Some((mean, var.sqrt()))
}

/// 設定項目がないときのエラー文 (既定の `Rule::configure` と同じ書き方)。
fn unknown_option(meta: &RuleMeta, key: &str) -> String {
    format!("{} に `{key}` という設定項目はありません", meta.id)
}

/// 設定値を `lo..=hi` の数値として読む。
fn option_f64_in(key: &str, value: &toml::Value, lo: f64, hi: f64) -> Result<f64, String> {
    let v = option_f64(key, value)?;
    if !(lo..=hi).contains(&v) {
        return Err(format!(
            "`{key}` には {lo} 以上 {hi} 以下の数値を指定してください"
        ));
    }
    Ok(v)
}

/// 設定値を 1 以上の整数として読む。
fn option_count(key: &str, value: &toml::Value) -> Result<usize, String> {
    match crate::rules::option_usize(key, value)? {
        0 => Err(format!("`{key}` には 1 以上の整数を指定してください")),
        n => Ok(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::{Lane, RuleStatus, Severity};
    use crate::document::Document;
    use crate::rules::RuleContext;
    use crate::rules::testing::assert_measures_agree;

    /// measure を実装しているルール。R10・R11・R15 は閾値を持たない。
    const MEASURED: [&str; 13] = [
        "R01", "R02", "R03", "R04", "R05", "R06", "R07", "R08", "R09", "R12", "R13", "R14", "R16",
    ];

    fn rule_by_id(id: &str) -> Box<dyn Rule> {
        rules(Genre::General)
            .into_iter()
            .find(|r| r.meta().id == id)
            .unwrap_or_else(|| panic!("{id} がありません"))
    }

    /// 同じ長さの文を並べた段落。
    fn lengths(lengths: &[usize]) -> String {
        lengths
            .iter()
            .map(|&n| format!("{}。", "文".repeat(n)))
            .collect::<String>()
            + "\n"
    }

    /// 述語で終わる、指定した長さ (「あ」の数) の文を並べた段落。
    fn predicate_lengths(lengths: &[usize]) -> String {
        lengths
            .iter()
            .map(|&n| format!("{}た。", "あ".repeat(n)))
            .collect::<String>()
            + "\n"
    }

    /// 動詞で終わる文を `chars` 字ぶん以上並べた本文 (体言止めなし)。
    fn verb_only(chars: usize) -> String {
        let sentence = "担当者が毎朝の点検で見つけた不具合を記録して上長に報告した。";
        let per = crate::text::reading_length(sentence);
        sentence.repeat(chars / per + 1) + "\n"
    }

    fn repeated_lead(lead: &str, n: usize) -> String {
        (0..n)
            .map(|i| format!("{lead}{i}番目の手順を確かめた。"))
            .collect::<String>()
            + "\n"
    }

    /// 指摘する例・しない例をひととおり含む文書。
    fn samples() -> Vec<String> {
        let mut varied = Vec::new();
        for i in 0..24 {
            varied.push([4, 60, 25][i % 3]);
        }
        let contrast = |fillers: usize| {
            let mut s = String::from(
                "これは通知ではなく、仕組みだ。\n\n手順ではなく、考え方を示す。\n\n速さだけでなく、正確さも求められる。\n\n",
            );
            for i in 0..fillers {
                s.push_str(&format!("補足の文をここに{i}つ目として置いた。"));
            }
            s.push('\n');
            s
        };
        let alternating: Vec<usize> = (0..20).map(|i| [10, 40][i % 2]).collect();
        let clustered: Vec<usize> = (0..20).map(|i| 10 + (i / 5) * 10).collect();
        vec![
            String::new(),
            "# 見出しだけ\n".to_string(),
            "- 項目。\n- 項目。\n- 項目。\n- 項目。\n".to_string(),
            // R01
            (0..22)
                .map(|i| format!("担当者が手順書の{}番目の項目を読んで確かめた。", i % 10))
                .collect::<String>()
                + "\n",
            lengths(&varied),
            lengths(&[20; 15]),
            lengths(&[20; 9]),
            // R02
            "在庫を数えています。差異を集計しています。原因を調べています。\n".to_string(),
            "成果物の評価。担当者の確認。期限の設定。\n".to_string(),
            "これは重要な課題です。対応は来月です。担当はまだ未定です。\n".to_string(),
            // R03
            format!("{}。{}。\n", "長".repeat(95), "長".repeat(40)),
            format!("{}。\n", "長".repeat(90)),
            // R04
            "本機能は、監視対象のサーバーから集めたログの収集、しきい値を超えたかどうかの判定、当番の担当者への通知、対応履歴の記録を自動で行います。\n".to_string(),
            "来月の説明会に向けて、会場の手配を担当する総務部との予算の調整、講師の先生方の移動を含めた日程の調整、当日の受付と誘導を担う担当者の確認を、今週中に終えておく必要があります。\n".to_string(),
            "予算、日程、担当者の確認を行います。\n".to_string(),
            // R05 (重大・警告・情報になる比率と、回数が足りない例)
            contrast(7),
            contrast(117),
            contrast(200),
            "これは通知ではなく、仕組みだ。手順ではなく、考え方を示す。\n".to_string(),
            // R06
            verb_only(2100),
            verb_only(1200),
            verb_only(2100) + "\n残る課題は夜間の一次対応。\n",
            // R07
            "最初の文を書いた。次の文を書いた。最後の文を書いた。\n\n".repeat(4),
            "一文だけの段落。\n\n一つ目。二つ目。三つ目。四つ目。五つ目。\n\n二文の段落。もう一文。\n\n一つ目。二つ目。三つ目。四つ目。五つ目。六つ目。\n".to_string(),
            // R08
            repeated_lead("また、", 6),
            repeated_lead("また、", 5),
            // R09
            "結論から書く。\n\nしかし、例外もある。\n\nまた、費用の問題もある。\n\n最後に日程を決めた。\n".to_string(),
            "一段落目。\n\n二段落目。\n\n三段落目。\n\n四段落目。\n\nしかし、五段落目。\n".to_string(),
            "しかし、一段落目。\n\n二段落目。\n".to_string(),
            // R12 (長短の交互・体言止めの多さ)
            predicate_lengths(&alternating),
            predicate_lengths(&clustered),
            "申請の窓口を一本化した。理由は単純。締め日が部署ごとに違った。結果は良好。差し戻しは減った。次の課題は夜間の対応。担当者も増やした。問い合わせは半分。記入例も添えた。効果は明白。\n".to_string(),
            "申請の窓口を一本化した。理由は単純。締め日が部署ごとに違った。差し戻しは減った。次の課題は夜間の対応。担当者も増やした。問い合わせは半分に減った。記入例も添えた。効果ははっきり出た。来月も続ける。\n".to_string(),
            // R13 (読点の多さ)
            "今回は、申請の手順を、担当者ごとに、分けて説明します。".repeat(20) + "\n",
            "今回は申請の手順を、担当者ごとに分けて説明します。".repeat(20) + "\n",
            "申請は来週です。".repeat(20) + "\n",
            // R14 (長い重複と短い重複)
            "申請書は提出前に担当者が記入漏れと添付資料の不足を確認してください。\n\n".repeat(2),
            "受付は九時です。\n\n".repeat(2),
            // R16 (三項列挙の反復と単発)
            "操作は速く、柔軟で、直感的です。導入で効率、品質、成長を支えます。運用で信頼、安心、価値を届けます。\n".into(),
            "操作は速く、柔軟で、直感的です。\n".into(),
        ]
    }

    #[test]
    fn measures_agree_with_check() {
        let docs: Vec<Document> = samples().into_iter().map(Document::markdown).collect();
        for rule in rules(Genre::General) {
            let id = rule.meta().id;
            if !MEASURED.contains(&id) {
                assert!(
                    docs.iter()
                        .all(|d| rule.measure(&RuleContext::new(d)).is_empty()),
                    "{id}: measure を実装したら MEASURED に足してください"
                );
                continue;
            }
            let agreement = assert_measures_agree(|| rule_by_id(id), &docs);
            assert!(agreement.fired > 0, "{id}: 指摘する例がありません");
            assert!(
                agreement.quiet > 0,
                "{id}: 値を測れて指摘しない例がありません"
            );
            // 重大度の切り替えを決める閾値を持つのは R05 の error_above だけ
            let switches: &[Severity] = if id == "R05" { &[Severity::Error] } else { &[] };
            assert_eq!(agreement.switches, switches, "{id}");
        }
    }

    #[test]
    fn mean_sd_is_population() {
        let (m, sd) = mean_sd(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]).unwrap();
        assert!((m - 5.0).abs() < 1e-9);
        assert!((sd - 2.0).abs() < 1e-9);
        assert!(mean_sd(&[]).is_none());
    }

    #[test]
    fn rule_ids_are_in_order() {
        let ids: Vec<_> = rules(Genre::General).iter().map(|r| r.meta().id).collect();
        let expected: Vec<String> = (1..=16).map(|n| format!("R{n:02}")).collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn new_rules_are_experimental_slop_rules_with_the_explanation_template() {
        for rule in rules(Genre::General) {
            let m = rule.meta();
            if !matches!(m.id, "R11" | "R12" | "R13" | "R14" | "R15" | "R16") {
                continue;
            }
            let lane = if m.id == "R14" {
                Lane::Readability
            } else {
                Lane::Slop
            };
            assert_eq!((m.lane, m.status), (lane, RuleStatus::Experimental));
            for section in [
                "### 何を見るか",
                "### なぜ問題か",
                "### 直し方",
                "### 例",
                "### 根拠",
            ] {
                assert!(m.explanation.contains(section), "{} {section}", m.id);
            }
            assert!(m.explanation.contains("- 直す前: "), "{}", m.id);
            assert!(m.explanation.contains("- 直した後: "), "{}", m.id);
            assert!(!m.summary.contains('\n'), "{}", m.id);
        }
    }
}
