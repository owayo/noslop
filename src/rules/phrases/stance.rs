//! P25: 出典の内容よりも、出典の態度を解説する言い回し。

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::rules::{Rule, RuleContext, RuleMeta, RuleUnit, quote};
use crate::text;

use super::engine::{WEAK_SIGNAL_NOTE, diagnostic};

static META: RuleMeta = RuleMeta {
    id: "P25",
    name: "SOURCE_STANCE",
    title: "出典の態度の解説",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "本・記事・著者などを主語にして、内容よりも出典の態度を語る言い回しを見直す (実験的)",
    explanation: r"### 何を見るか

「本」「本書」「この本」「著者」「筆者」「この章」「本章」「記事」「論文」「資料」「ドキュメント」に「は」「が」「では」が続き、同じ文の主語のあと 40 字以内に始まる「認める」「強調する」「指摘する」「釘を刺す」「戒める」「説く」「念を押す」「主張する」「警告する」の活用形がある箇所を探します。「この記事」「この資料」なども含みます。主語のあとで開いた括弧の中にある述語と、主語自体が括弧の中にあるものは数えません。

「〜と呼んでいます」のような用語の紹介だけの文や、「効果が認められた」のような受け身の形、主語のない「先に認めています」だけでは指摘しません。既定は段落だけで、リスト・表・引用は語句ルールのスコープ設定に従います。

### なぜ問題か

出典が強調した・戒めたという態度の説明が重なると、その出典に何が書かれているのかを読むまでに遠回りします。ただし、書評で著者の態度自体を論じる場合や、主張と事実を区別するための帰属は必要です。書き手や執筆方法の判定には使えません。

### 直し方

内容の要約なら、出典の態度ではなく内容をそのまま述べられるか確かめます。出典を示す必要がある場合や、態度そのものを論じる場合は残せます。原文にない事実・引用・ページ番号を足さないでください。

### 例

- 直す前: 本書は、確認を省くと入力の誤りを見逃すと強調しています。
- 直した後: 確認を省くと、入力の誤りを見逃します。

### 根拠

人の書評や読書メモにも出る型です。人の文書 171 本中 2 本・生成文書 390 本中 0 本に反応しましたが、書評や読書メモに絞った校正はないため、実験的な情報の指摘にとどめます。辞書を使わず文字列と括弧で近似し、近い主語との距離で照合します。入れ子の節の主語への帰属や、出典への帰属が必要かまでは判定しません。",
};

pub(super) struct SourceStance;

static SUBJECT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<noun>本書|この本|本|著者|筆者|この章|本章|(?:この|本)?(?:記事|論文|資料|ドキュメント))(?:では|は|が)")
        .expect("出典の主語")
});

static PREDICATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"認め(?:る|た|て|ます|ました)|(?:強調|指摘|主張|警告)(?:する|し)|釘を刺[すし]|戒め(?:る|た|て|ます|ました)|説[くいき]|念を押[すし]")
        .expect("出典の態度の述語")
});

const ITEMS: [(&str, &str); 9] = [
    ("認め", "認める"),
    ("強調", "強調する"),
    ("指摘", "指摘する"),
    ("主張", "主張する"),
    ("警告", "警告する"),
    ("釘を刺", "釘を刺す"),
    ("戒め", "戒める"),
    ("説", "説く"),
    ("念を押", "念を押す"),
];

/// 主語のあとで開き、まだ閉じていない括弧の内側か。
fn inside_brackets(value: &str) -> bool {
    let mut stack = Vec::new();
    for c in value.chars() {
        if stack.last() == Some(&c) {
            stack.pop();
        } else if let Some(close) = text::closing_bracket(c) {
            stack.push(close);
        }
    }
    !stack.is_empty()
}

impl SourceStance {
    fn find(value: &str) -> Vec<(Range<usize>, &'static str, String)> {
        let subjects: Vec<_> = SUBJECT
            .captures_iter(value)
            .filter(|s| {
                let subject = s.get(0).expect("主語の全体");
                !inside_brackets(&value[..subject.start()])
                    && (subject.as_str().starts_with("この")
                        || !value[..subject.start()]
                            .chars()
                            .next_back()
                            .is_some_and(text::is_kanji))
            })
            .collect();
        let mut out = Vec::new();
        for predicate in PREDICATE.find_iter(value) {
            // 「解説」「力説」の末尾を独立した「説く」とみなさない。
            if predicate.as_str().starts_with('説')
                && value[..predicate.start()]
                    .chars()
                    .next_back()
                    .is_some_and(text::is_kanji)
            {
                continue;
            }
            let Some(subject) = subjects
                .iter()
                .rev()
                .find(|s| s.get(0).expect("主語の全体").end() <= predicate.start())
            else {
                continue;
            };
            let between = &value[subject.get(0).expect("主語の全体").end()..predicate.start()];
            if between.chars().count() >= 40 || inside_brackets(between) {
                continue;
            }
            let item = ITEMS
                .iter()
                .find(|(stem, _)| predicate.as_str().starts_with(stem))
                .expect("述語の項目")
                .1;
            out.push((predicate.range(), item, subject["noun"].to_string()));
        }
        out
    }
}

impl Rule for SourceStance {
    fn unit(&self) -> RuleUnit {
        RuleUnit::Sentence
    }
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for (idx, block) in ctx.scoped_blocks() {
            for sentence in ctx.doc.block_sentences(idx) {
                let value = &block.text[sentence.range.clone()];
                for (range, item, subject) in Self::find(value) {
                    let matched = &value[range.clone()];
                    let span = block.to_source(
                        sentence.range.start + range.start..sentence.range.start + range.end,
                    );
                    out.push(diagnostic(&META, span, sentence.span, matched,
                        format!("出典への言及「{}」のあとにある「{}」が、内容の説明に必要か確かめてください。{WEAK_SIGNAL_NOTE}", quote(&subject), quote(matched)),
                        "内容を直接述べられるか確かめてください。必要な帰属は残し、原文にない事実を足さないでください",
                        Severity::Info, RuleStatus::Experimental).with_metric("item", item));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, DocumentKind};
    use crate::engine::{Engine, EngineOptions, Selection};
    use crate::rules::{
        Scope,
        testing::{Options, matched, run, run_with},
    };

    #[test]
    fn flags_source_predicates_with_original_positions() {
        for (md, expected) in [
            (
                "確認の難しさを、本は「見落とし」と呼び、起こり得ると先に認めています。",
                "認めて",
            ),
            (
                "間違いが起こると、本が先に認めている点を覚えています。",
                "認めて",
            ),
            ("**著者**は、確認が必要だと**強調しています**。", "強調し"),
            (
                "本書は、手順を省く危険にも先回りして釘を刺しています。",
                "釘を刺し",
            ),
            (
                "この章は、確認を省く行動をはっきりと戒めています。",
                "戒めて",
            ),
        ] {
            let findings = run(&SourceStance, md);
            assert_eq!(matched(md, &findings), [expected], "{md}");
            assert_eq!(findings[0].severity, Severity::Info);
            assert_eq!(findings[0].status, RuleStatus::Experimental);
            assert_eq!(findings[0].lane, Lane::Slop);
            assert!(findings[0].context.is_some());
        }
    }

    #[test]
    fn excludes_terms_passive_predicates_and_quoted_predicates() {
        for md in [
            "第2節では、この操作を「確認」と呼んでいます。",
            "著者は「手順を強調する」という言葉を紹介しています。",
            "本書は（資料が『確認を戒める』とする）記述を紹介しています。",
            "著者は「本書は手順を強調する」と書いた。",
            "本書は（資料が確認を戒める）記述を紹介しています。",
            "この論文では改善が認められた。",
            "記事は確認の方法を説明します。",
            "日本がこの方針を認めた。",
            "脚本は困難を認めています。",
            "先に認めています。",
            "著者は確認の必要性を述べた。別の人が強調する。",
            "`著者は強調しています` は例文です。",
        ] {
            assert!(run(&SourceStance, md).is_empty(), "{md}");
        }
        let md = "著者は「確認を強調する」という語を戒めています。";
        assert_eq!(matched(md, &run(&SourceStance, md)), ["戒めて"]);
    }

    #[test]
    fn keeps_predicates_inside_the_character_window() {
        let inside = format!("資料は{}強調する。", "あ".repeat(36));
        let outside = format!("資料は{}強調する。", "あ".repeat(40));
        assert_eq!(run(&SourceStance, &inside).len(), 1);
        assert!(run(&SourceStance, &outside).is_empty());
    }

    #[test]
    fn handles_subject_boundaries_all_items_and_multiple_predicates() {
        use crate::diagnostic::Metric;
        for (md, item) in [
            ("今回この資料では確認の必要性を強調しています。", "強調する"),
            ("本書は再度認めています。", "認める"),
            ("著者は再三警告する。", "警告する"),
            ("この章は一度念を押す。", "念を押す"),
            ("ドキュメントでは危険を指摘する。", "指摘する"),
            ("筆者は確認を主張した。", "主張する"),
            ("本章は手順を説いている。", "説く"),
            ("記事は危険に釘を刺した。", "釘を刺す"),
            ("この本は確認の省略を戒めます。", "戒める"),
        ] {
            let findings = run(&SourceStance, md);
            assert_eq!(findings.len(), 1, "{md}");
            assert_eq!(
                findings[0].metrics.get("item"),
                Some(&Metric::Text(item.into()))
            );
        }
        let md = "著者は、本書が認めた点を強調している。";
        let findings = run(&SourceStance, md);
        assert_eq!(matched(md, &findings), ["認めた", "強調し"]);
        let md = "本書は危険を指摘し、対策を強調する。";
        assert_eq!(matched(md, &run(&SourceStance, md)), ["指摘し", "強調する"]);
        assert!(run(&SourceStance, "本書では手順を解説しています。").is_empty());
        let md = format!("資料は{}釘を刺す。", "あ".repeat(39));
        assert_eq!(matched(&md, &run(&SourceStance, &md)), ["釘を刺す"]);
    }

    #[test]
    fn follows_scope_and_requires_selection_even_for_fragments() {
        let md = "# 本書は強調する\n\n- 本書は強調する。\n\n> 本書は強調する。\n";
        assert!(run(&SourceStance, md).is_empty());
        assert_eq!(
            run_with(
                &SourceStance,
                md,
                Options {
                    scope: Scope::ALL,
                    ..Options::default()
                }
            )
            .len(),
            2
        );
        for (experimental, explicit, expected) in
            [(false, false, 0), (true, false, 1), (false, true, 1)]
        {
            let engine = Engine::with_rules(
                vec![Box::new(SourceStance)],
                EngineOptions {
                    experimental,
                    selection: Selection {
                        cli_enable: if explicit { vec!["P25".into()] } else { vec![] },
                        ..Selection::default()
                    },
                    ..EngineOptions::default()
                },
            )
            .unwrap();
            let mut doc = Document::plain_text("本書は確認の必要性を強調しています。");
            assert_eq!(engine.lint(doc.clone()).diagnostics.len(), expected);
            doc.kind = DocumentKind::Fragments;
            assert_eq!(engine.lint(doc).diagnostics.len(), expected);
        }
    }
}
