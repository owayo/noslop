//! 語句パターン系ルール (`P` で始まる ID)。
//!
//! 文の中の表現を 1 件ずつ拾う。多くは語句辞書による照合 ([`catalog`]) で、
//! 辞書だけでは誤爆する構文の型 ([`syntax`]) と、読みやすさのルール ([`reading`]) は
//! 個別に実装している。
//!
//! 多くのルールは `RuleContext::scoped_blocks` が返すブロック (既定は地の文の段落だけ) の
//! 文ごとに照合し、文をまたぐ一致はしない。引用マーカーと記法の残骸 ([`artifacts`]、P22・
//! [`markup`]、P23) は、置き場所を問わず見える記号を拾うので、スコープに従わず全ブロックを行ごとに見る。

mod artifacts;
mod attribution;
mod catalog;
mod closer;
mod effect;
mod engine;
mod jargon;
mod markup;
mod permission;
mod quotes;
mod reading;
mod stance;
mod syntax;

use crate::genre::Genre;
use crate::rules::Rule;

use engine::PhraseRule;

/// 語句パターン系の組み込みルール (ID 順)。
pub fn rules(genre: Genre) -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(PhraseRule::new(&catalog::P01).excluding_quoted_mentions()),
        Box::new(PhraseRule::new(&catalog::P02)),
        Box::new(PhraseRule::new(&catalog::P03).excluding_quoted_mentions()),
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

    #[test]
    fn conclusion_and_conjunction_skip_quoted_mentions() {
        use crate::rules::testing::{matched, run};
        let rules = rules(Genre::Tech);
        for (id, phrase) in [("P01", "と言えるでしょう"), ("P03", "このように")] {
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
        for (id, phrase) in [("P01", "と言えるでしょう"), ("P03", "このように")] {
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
            ("P03", "「つまり、確認が必要ということです」と書きます。"),
        ] {
            assert!(run_with(get(id), md, all).is_empty(), "{id}");
        }
        let md = "「それでは、確認していきます」という例です。";
        assert_eq!(matched(md, &run(get("P02"), md)), ["それでは、"]);
        let md = "「このように」は避けます。**このように**、確認します。";
        assert_eq!(matched(md, &run(get("P03"), md)), ["このように"]);
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
        assert_eq!(get("P01"), (Lane::Slop, RuleStatus::Stable));
        assert_eq!(get("P04"), (Lane::Readability, RuleStatus::Experimental));
        assert_eq!(get("P05"), (Lane::Slop, RuleStatus::Experimental));
        assert_eq!(get("P10"), (Lane::Slop, RuleStatus::Experimental));
        assert_eq!(get("P11"), (Lane::Slop, RuleStatus::Experimental));
        assert_eq!(get("P13"), (Lane::Slop, RuleStatus::Stable));
        assert_eq!(get("P14"), (Lane::Slop, RuleStatus::Experimental));
        for id in ["P15", "P16", "P17"] {
            assert_eq!(get(id), (Lane::Readability, RuleStatus::Stable), "{id}");
        }
        for id in ["P18", "P19", "P20", "P21", "P22", "P23", "P24"] {
            assert_eq!(get(id), (Lane::Slop, RuleStatus::Experimental), "{id}");
        }
    }
}
