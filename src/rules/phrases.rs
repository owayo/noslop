//! 語句パターン系ルール (`P` で始まる ID)。
//!
//! 文の中の表現を 1 件ずつ拾う。多くは語句辞書による照合 ([`catalog`]) で、
//! 辞書だけでは誤爆する構文の型 ([`syntax`]) と、読みやすさのルール ([`reading`]) は
//! 個別に実装している。
//!
//! 多くのルールは `RuleContext::scoped_blocks` が返すブロック (既定は地の文の段落だけ) の
//! 文ごとに照合し、文をまたぐ一致はしない。引用マーカーと記法の残骸 ([`artifacts`]、P22・
//! [`markup`]、P23) は、置き場所を問わず見える記号を拾うので、スコープに従わず全ブロックを行ごとに見る。
//!
//! 語句辞書で動くルール ([`engine::PhraseRule`]) と、P14 (ダッシュの挿入句)・P17 (二重否定) は、
//! 同じブロック内で対応する引用符の内側に収まる一致と、開き引用符の直後の一致を、語句への言及として
//! 除く ([`quotes`])。ほかのルールは、引用の内側の一致がほとんどないか、引用の中でも読み手の負荷や
//! 構文が変わらないので除かない。

mod artifacts;
mod attribution;
mod catalog;
mod closer;
mod effect;
mod engine;
mod jargon;
mod location;
mod markup;
mod permission;
mod quotes;
mod reading;
mod stance;
mod syntax;

use crate::genre::Genre;
use crate::rules::Rule;

use engine::PhraseRule;

/// P01 (結論の定型句) が文書の結びとして見る、地の文の最後の段落の数。
///
/// 人の文書では、同じ語句が本文の途中 (問いかけの書き出し・節の途中のまとめ) によく出る。
/// 生成文書の結論の定型句は文書の最後の段落に集まるので、結びに限って拾う。
const P01_CLOSING_PARAGRAPHS: usize = 2;

/// 語句パターン系の組み込みルール (ID 順)。
pub fn rules(genre: Genre) -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(PhraseRule::new(&catalog::P01).only_in_closing(P01_CLOSING_PARAGRAPHS)),
        Box::new(PhraseRule::new(&catalog::P02)),
        Box::new(PhraseRule::new(&catalog::P03)),
        Box::new(PhraseRule::new(&catalog::P04)),
        Box::new(PhraseRule::new(&catalog::P05)),
        Box::new(PhraseRule::new(&catalog::P06)),
        Box::new(PhraseRule::new(&catalog::P07)),
        Box::new(PhraseRule::new(&catalog::P08)),
        Box::new(PhraseRule::new(&catalog::P09)),
        Box::new(PhraseRule::new(&catalog::P10)),
        Box::new(PhraseRule::new(&catalog::P11)),
        Box::new(PhraseRule::new(&catalog::P12)),
        Box::new(syntax::InanimateSubject),
        Box::new(syntax::EmDash),
        Box::new(reading::KanjiRun::default()),
        Box::new(reading::NoChain::default()),
        Box::new(reading::DoubleNegative),
        Box::new(PhraseRule::new(&catalog::P18)),
        Box::new(PhraseRule::new(&catalog::P19)),
        Box::new(syntax::ColonContinuation),
        Box::new(attribution::VagueAttribution),
        Box::new(artifacts::CitationArtifact::new()),
        Box::new(markup::MarkupResidue),
        Box::new(PhraseRule::new(&effect::P24)),
        Box::new(stance::SourceStance),
        Box::new(jargon::ActionJargon::new()),
        Box::new(closer::StockCloser::new(genre)),
        Box::new(permission::PermissiveAction),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::{Lane, RuleStatus};

    #[test]
    fn all_phrase_rules_are_registered_in_id_order() {
        let ids: Vec<_> = rules(Genre::General).iter().map(|r| r.meta().id).collect();
        let expected: Vec<String> = (1..=28).map(|n| format!("P{n:02}")).collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn metas_follow_the_explanation_template() {
        for rule in rules(Genre::General) {
            let m = rule.meta();
            for section in [
                "### 何を見るか",
                "### なぜ問題か",
                "### 直し方",
                "### 例",
                "### 根拠",
            ] {
                assert!(
                    m.explanation.contains(section),
                    "{} の explanation に {section} がない",
                    m.id
                );
            }
            assert!(m.explanation.contains("- 直す前: "), "{}", m.id);
            assert!(m.explanation.contains("- 直した後: "), "{}", m.id);
            assert!(!m.summary.contains('\n'), "{}", m.id);
            assert!(m.name.chars().all(|c| c.is_ascii_uppercase() || c == '_'));
        }
    }

    #[test]
    fn diagnostics_carry_the_item_and_the_matched_text() {
        use crate::diagnostic::Metric;
        use crate::rules::testing::run;
        let rules = rules(Genre::General);
        let get = |id: &str| rules.iter().find(|r| r.meta().id == id).unwrap();
        let metric = |d: &crate::diagnostic::Diagnostic, key: &str| match d.metrics.get(key) {
            Some(Metric::Text(s)) => s.clone(),
            other => panic!("{key}: {other:?}"),
        };

        // 辞書の正規表現の項目は、表記が違っても同じ項目の名前 (`/パターン/`) になる
        let d = run(
            get("P12").as_ref(),
            "記録を読むことができる。来週に書くことが出来ます。\n",
        );
        let pairs: Vec<(String, String)> = d
            .iter()
            .map(|d| (metric(d, "item"), metric(d, "matched")))
            .collect();
        let pattern = catalog::P12
            .entries
            .last()
            .expect("P12 の項目")
            .pattern
            .item();
        assert!(
            pattern.starts_with("/こと(?:が|は)(?:でき|出来)"),
            "{pattern}"
        );
        assert!(pattern.ends_with('/'), "{pattern}");
        assert_eq!(
            pairs,
            vec![
                (pattern.clone(), "ことができる".to_string()),
                (pattern, "ことが出来ます".to_string()),
            ]
        );
        // リテラルの項目はその文字列
        let d = run(get("P02").as_ref(), "さて、本題に入る。\n");
        assert_eq!(metric(&d[0], "item"), "さて、");

        // 構文の型で拾うルールは、校正で比べたい単位 (P13 は述語の終止形) を項目にする
        let d = run(
            get("P13").as_ref(),
            "この事実は、利用者の関心が移ったことを示している。\n",
        );
        assert_eq!(metric(&d[0], "item"), "示す");
        assert_eq!(
            metric(&d[0], "matched"),
            "この事実は、利用者の関心が移ったことを示し"
        );
    }

    /// 語句辞書で動くルール (P27 は自前の条件も持つので、closer のテストで確かめる) と、
    /// 引用の外で拾われる語句。
    const DICTIONARY_PHRASES: [(&str, &str); 15] = [
        ("P01", "と言えるでしょう"),
        ("P02", "見ていきましょう"),
        ("P03", "このように"),
        ("P04", "を行う"),
        ("P05", "様々な"),
        ("P06", "非常に重要"),
        ("P07", "一概には言えません"),
        ("P08", "多角的"),
        ("P09", "掘り下げる"),
        ("P10", "に他なりません"),
        ("P11", "理由は3つあります"),
        ("P12", "することができる"),
        ("P18", "ご質問ありがとうございます"),
        ("P19", "革命的"),
        ("P24", "地味に効く"),
    ];

    #[test]
    fn dictionary_rules_skip_quoted_mentions() {
        use crate::rules::testing::{matched, run};
        let rules = rules(Genre::Tech);
        for (id, phrase) in DICTIONARY_PHRASES {
            let rule = rules.iter().find(|r| r.meta().id == id).unwrap();
            for (open, close) in [("「", "」"), ("『", "』"), ("“", "”"), ("\"", "\"")] {
                for md in [
                    format!("{open}{phrase}{close}という語を見直します。"),
                    format!("{open}例として、**{phrase}**と書きます。{close}という説明です。"),
                    format!("{open}準備は済みました。\n{phrase}と書きます。{close}という例です。"),
                ] {
                    assert!(run(rule.as_ref(), &md).is_empty(), "{id}: {md}");
                }
            }
            let md = format!("「{phrase}」という語です。続いて{phrase}、確認します。");
            assert_eq!(matched(&md, &run(rule.as_ref(), &md)), [phrase]);
            let md = format!("「{phrase}という語を見直します。");
            assert!(run(rule.as_ref(), &md).is_empty(), "{id}: {md}");
            let md = format!("「前の例です。別の説明に{phrase}、確認します。");
            assert_eq!(matched(&md, &run(rule.as_ref(), &md)), [phrase]);
        }
    }

    #[test]
    fn quoted_mentions_respect_scope_experiments_and_fragment_boundaries() {
        use crate::document::{Document, ParseOptions, SourceFormat};
        use crate::rules::Scope;
        use crate::rules::testing::{Options, matched, run, run_doc, run_with};
        use crate::segment::LineBreakMode;
        let rules = rules(Genre::Tech);
        let get = |id: &str| rules.iter().find(|r| r.meta().id == id).unwrap().as_ref();
        let all = Options {
            scope: Scope::ALL,
            experimental: true,
            ..Options::default()
        };
        for (id, phrase) in DICTIONARY_PHRASES {
            // P01 は文書の結びの段落だけを見るので、スコープを広げても箇条書きと引用は見ない
            // (p01_looks_only_at_the_closing_paragraphs で確かめる)
            if id == "P01" {
                continue;
            }
            let md = format!("- 「{phrase}」という語です。\n\n> 実際に{phrase}、確認します。\n");
            assert!(run(get(id), &md).is_empty());
            assert_eq!(matched(&md, &run_with(get(id), &md, all)), [phrase]);
            let md = format!("「先の例です。\n{phrase}と書きます。」\n\n{phrase}、確認します。");
            let doc = Document::parse(
                "<input>",
                &md,
                SourceFormat::Markdown,
                &ParseOptions {
                    line_breaks: LineBreakMode::Sentence,
                    ..ParseOptions::default()
                },
            );
            assert_eq!(matched(&md, &run_doc(get(id), &doc, all)), [phrase]);
            let md = format!("「閉じない例。\n\n{phrase}、確認します。");
            assert_eq!(matched(&md, &run(get(id), &md)), [phrase]);
            for prefix in ["\"説明\"", "5\"", "」"] {
                let md = format!("{prefix}{phrase}、確認します。");
                assert_eq!(matched(&md, &run(get(id), &md)), [phrase]);
            }
        }
        for (id, md) in [
            ("P01", "「結論から述べると、準備が必要です。」という例。"),
            ("P02", "「それでは、確認していきます」という例です。"),
            ("P03", "「つまり、確認が必要ということです」と書きます。"),
            ("P08", "「包括的な見直し」という題の資料を読む。"),
            (
                "P12",
                "「複数の情報源を比較することができる」と答えた人が多い。",
            ),
        ] {
            assert!(run_with(get(id), md, all).is_empty(), "{id}");
        }
        let md = "「このように」は避けます。**このように**、確認します。";
        assert_eq!(matched(md, &run(get("P03"), md)), ["このように"]);
        // 辞書を使わないルールのうち、引用の内側を除くのは P14 と P17 だけ。
        // P13 は、括弧の中で主語と述語がそろった文 (引用した発言) をこれまでどおり数える
        let md = "彼は「これは誤りを示す」と言った。";
        assert_eq!(matched(md, &run(get("P13"), md)), ["これは誤りを示す"]);
        for (id, md) in [
            ("P14", "著書『贈与の論理——資本主義のすきま』を読む。"),
            ("P17", "「〜ないわけではない」のような言い回しを避ける。"),
        ] {
            assert!(run_with(get(id), md, all).is_empty(), "{id}");
        }
    }

    #[test]
    fn p01_looks_only_at_the_closing_paragraphs() {
        use crate::rules::testing::{Options, matched, run, run_with};
        use crate::rules::{RuleUnit, Scope};
        let rules = rules(Genre::General);
        let p01 = rules
            .iter()
            .find(|r| r.meta().id == "P01")
            .unwrap()
            .as_ref();
        // 本文の途中の問いかけは拾わず、最後の 2 段落の結びの定型句だけを拾う
        let md = "迷う人も多いのではないでしょうか。\n\n設定を見直した。\n\n手順は短くなった。\n\nまずは試してみてはいかがでしょうか。\n";
        assert_eq!(matched(md, &run(p01, md)), ["いかがでしょうか"]);
        // 結びの後ろの見出し・箇条書きは数えず、その手前の段落を結びとみなす
        let md =
            "本文です。\n\n総じて、移行は順調と言えるでしょう。\n\n## まとめ\n\n- 移行は終わった\n";
        assert_eq!(matched(md, &run(p01, md)), ["総じて", "と言えるでしょう"]);
        // スコープを広げても、箇条書きと引用の中は結びとみなさない
        let all = Options {
            scope: Scope::ALL,
            ..Options::default()
        };
        let md = "本文です。\n\n- と言えるでしょう、確認します。\n\n> 結論として、確かめた。\n";
        assert!(run_with(p01, md, all).is_empty());
        // 文書の構造が要るので、断片の集まり (コードのコメントなど) には当てない
        // (エンジンは文書単位のルールを断片の集まりに当てない)
        assert_eq!(p01.unit(), RuleUnit::Document);
    }

    #[test]
    fn lanes_and_statuses_match_the_design() {
        let rules = rules(Genre::General);
        let get = |id: &str| {
            rules
                .iter()
                .find(|r| r.meta().id == id)
                .map(|r| (r.meta().lane, r.meta().status))
                .unwrap()
        };
        for id in ["P01", "P06", "P07", "P08", "P09"] {
            assert_eq!(get(id), (Lane::Slop, RuleStatus::Stable), "{id}");
        }
        // 人の文章にも同じくらいかそれ以上に出たので、ルールごと実験的にしたもの
        for id in ["P02", "P03", "P13"] {
            assert_eq!(get(id), (Lane::Slop, RuleStatus::Experimental), "{id}");
        }
        assert_eq!(get("P04"), (Lane::Readability, RuleStatus::Experimental));
        assert_eq!(get("P05"), (Lane::Slop, RuleStatus::Experimental));
        assert_eq!(get("P10"), (Lane::Slop, RuleStatus::Experimental));
        assert_eq!(get("P11"), (Lane::Slop, RuleStatus::Experimental));
        assert_eq!(get("P14"), (Lane::Slop, RuleStatus::Experimental));
        for id in ["P15", "P16", "P17"] {
            assert_eq!(get(id), (Lane::Readability, RuleStatus::Stable), "{id}");
        }
        for id in ["P18", "P19", "P20", "P21", "P22", "P23", "P24"] {
            assert_eq!(get(id), (Lane::Slop, RuleStatus::Experimental), "{id}");
        }
    }
}
