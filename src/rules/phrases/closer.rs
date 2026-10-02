//! P27: 書き手の願望で締める結びの定型。

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::genre::Genre;
use crate::rules::{Rule, RuleContext, RuleMeta, RuleUnit};

use super::engine::{Entry, PhraseRule, PhraseSpec, WEAK_SIGNAL_NOTE};

const fn entries(status: RuleStatus) -> [Entry; 4] {
    [
    Entry { status, ..Entry::re(r#"(?:^|[^「『“"])(?P<m>参考になれば\s*(?:幸いです|うれしいです|嬉しいです))"#, Severity::Info).with_note(WEAK_SIGNAL_NOTE) },
    Entry { status, ..Entry::re(r#"(?:^|[^「『“"])(?P<m>参考にしていただければ\s*(?:幸いです|うれしいです|嬉しいです))"#, Severity::Info).with_note(WEAK_SIGNAL_NOTE) },
    Entry { status, ..Entry::re(r#"(?:^|[^「『“"])(?P<m>お役に立て(?:れば|たなら)\s*(?:幸いです|うれしいです|嬉しいです))"#, Severity::Info).with_note(WEAK_SIGNAL_NOTE) },
    Entry { status, ..Entry::re(r#"(?:^|[^「『“"])(?P<m>一助となれば\s*(?:幸いです|うれしいです|嬉しいです))"#, Severity::Info).with_note(WEAK_SIGNAL_NOTE) },
    ]
}

const EXPERIMENTAL_ENTRIES: [Entry; 4] = entries(RuleStatus::Experimental);
const TECH_ENTRIES: [Entry; 4] = entries(RuleStatus::Stable);

static P27: PhraseSpec = spec(
    RuleStatus::Experimental,
    &EXPERIMENTAL_ENTRIES,
    "「参考になれば幸いです」など、書き手の願望で締める定型を見直す (tech 以外では実験的)",
);
static P27_TECH: PhraseSpec = spec(
    RuleStatus::Stable,
    &TECH_ENTRIES,
    "「参考になれば幸いです」など、書き手の願望で締める定型を見直す (tech では既定)",
);

fn for_genre(genre: Genre) -> &'static PhraseSpec {
    if genre == Genre::Tech {
        &P27_TECH
    } else {
        &P27
    }
}

pub(super) struct StockCloser(PhraseRule);

impl StockCloser {
    pub(super) fn new(genre: Genre) -> Self {
        Self(PhraseRule::new(for_genre(genre)))
    }
}

impl Rule for StockCloser {
    fn meta(&self) -> &'static RuleMeta {
        self.0.meta()
    }
    fn unit(&self) -> RuleUnit {
        self.0.unit()
    }
    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let mut findings = Vec::new();
        self.0.check(ctx, &mut findings);
        if findings.is_empty() {
            return;
        }
        let mut quotes = Vec::new();
        for (_, block) in ctx.scoped_blocks() {
            let mut stack = Vec::new();
            for (offset, c) in block.text.char_indices() {
                if let Some(position) = stack.iter().rposition(|&(_, close)| close == c) {
                    let (start, _) = stack[position];
                    stack.truncate(position);
                    quotes.push(block.to_source(start..offset + c.len_utf8()));
                } else if let Some(close) = match c {
                    '「' => Some('」'),
                    '『' => Some('』'),
                    '“' => Some('”'),
                    '"' if !block.text[..offset]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_ascii_digit()) =>
                    {
                        Some('"')
                    }
                    _ => None,
                } {
                    stack.push((offset, close));
                }
            }
        }
        out.extend(findings.into_iter().filter(|d| {
            !quotes
                .iter()
                .any(|q| q.start <= d.span.start && d.span.end <= q.end)
        }));
    }
}

const fn spec(status: RuleStatus, entries: &'static [Entry], summary: &'static str) -> PhraseSpec {
    PhraseSpec {
        meta: RuleMeta {
            id: "P27",
            name: "STOCK_CLOSER",
            title: "結びの定型",
            lane: Lane::Slop,
            status,
            default_severity: Severity::Info,
            summary,
            explanation: r"### 何を見るか

「参考になれば」「参考にしていただければ」「お役に立てれば」「お役に立てたなら」「一助となれば」に、「幸いです」「うれしいです」「嬉しいです」が続く結びの定型を探します。条件の語と「幸いです」などの間にある空白や段落内の折り返しも含みます。定型の直前が開きかぎ括弧や引用符の場合と、対応する引用符の内側にある場合は、語句自体への言及として除きます。文書の最後だけに限らず、途中の段落にあるものも同じように拾います。複数の節の結びを取りこぼさないためです。既定は地の文の段落だけで、リスト・表・引用は語句ルールのスコープ設定に従います。

### なぜ問題か

記事の中身から離れた定型だけで閉じると、読み手が次に何をすればよいか伝わりません。ただし、人が書くブログや連絡の挨拶にも使われます。書き手や執筆方法の判定には使えず、弱い手掛かりとして扱います。

### 直し方

結びが必要なら、本文にある内容を使って、何をどこから試せるかを具体的な一文で示します。「参考にしていただければ幸いです」のような依頼文や、挨拶として必要な場合は残せます。原文にない行動を勧めたり、事実や根拠を足したりしないでください。

### 例

- 直す前: まず確認欄を埋めてから依頼を送ります。参考になれば幸いです。
- 直した後: まず確認欄を埋めてから依頼を送ります。

### 根拠

人の文書 176 本中 2 本・生成文書 390 本中 4 本に反応しました。技術記事では人 50 本中 2 本 (4%、片側 95% の Wilson 上限 11.4%)・生成 32 本中 0 本、随筆では人 27 本・生成 65 本でどちらも 0 本でした。技術記事の一致を読み返し、引用への誤一致がなく、結びを見直す情報として使えることを確認したため、tech でのみ既定で有効にします。ほかのジャンルは実験的です。生成文書を見分ける根拠にはせず、必要な挨拶を一律に直すものでもありません。P18 から結びの挨拶を分け、チャットの応答だとは断定しません。",
        },
        entries,
        message: "「{m}」は結びの定型です。本文につながる締め方が必要か確かめてください",
        hint: "結びが必要なら、本文にある内容を使って何をどこから試せるか示してください。必要な挨拶は残せます",
    }
}

#[cfg(test)]
mod tests {
    use super::super::{catalog, engine::PhraseRule};
    use super::*;
    use crate::document::{Document, DocumentKind};
    use crate::engine::{Engine, EngineOptions, Selection};
    use crate::rules::{
        Scope,
        testing::{Options, matched, run, run_with},
    };

    #[test]
    fn flags_originals_and_all_requested_variants() {
        for phrase in [
            "参考になれば幸いです",
            "お役に立てれば幸いです",
            "参考になればうれしいです",
            "参考になれば嬉しいです",
            "一助となれば幸いです",
            "参考にしていただければ幸いです",
            "お役に立てたなら幸いです",
        ] {
            let md = format!("皆さんの**{phrase}**。\n");
            let findings = run(&StockCloser::new(Genre::General), &md);
            assert_eq!(matched(&md, &findings), [phrase]);
            assert_eq!(findings[0].lane, Lane::Slop);
            assert_eq!(findings[0].severity, Severity::Info);
            assert_eq!(findings[0].status, RuleStatus::Experimental);
            assert!(findings[0].message.contains(WEAK_SIGNAL_NOTE));
            assert!(run(&PhraseRule::new(&catalog::P18), &md).is_empty());
        }
    }

    #[test]
    fn detects_each_section_and_mid_document_paragraph() {
        let md = "# 進め方\n\n参考になれば幸いです。\n\n同じ悩みを持つ方の参考になれば幸いです。\n\nこの記事が参考になればうれしいです。\n\n少しでもお役に立てれば幸いです。\n\n## 手順\n\nこの表が手順の参考になれば幸いです。\n\n確認欄を埋めてください。\n";
        assert_eq!(run(&StockCloser::new(Genre::General), md).len(), 5);
        let md = "参考になれば\n幸いです。";
        assert_eq!(
            matched(md, &run(&StockCloser::new(Genre::General), md)),
            ["参考になれば\n幸いです"]
        );
        for md in [
            "参考になる表です。",
            "参考にしてください。",
            "`参考になれば幸いです` は例です。",
        ] {
            assert!(
                run(&StockCloser::new(Genre::General), md).is_empty(),
                "{md}"
            );
        }
    }

    #[test]
    fn skips_direct_quoted_mentions_and_separates_calibration_items() {
        let rule = StockCloser::new(Genre::General);
        for md in [
            "「参考になれば幸いです」で締めない。",
            "「この資料がお役に立てれば幸いです」と書く例です。",
            "『この記事が参考になれば嬉しいです』という文末です。",
            "『参考になれば嬉しいです』は定型です。",
            "\"お役に立てれば幸いです\"と書いた。",
        ] {
            assert!(run(&rule, md).is_empty(), "{md}");
        }
        let a = run(&rule, "参考になれば幸いです。");
        let b = run(&rule, "参考にしていただければ幸いです。");
        assert_ne!(a[0].metrics["item"], b[0].metrics["item"]);
        assert_eq!(
            a[0].message,
            format!(
                "「参考になれば幸いです」は結びの定型です。本文につながる締め方が必要か確かめてください（{WEAK_SIGNAL_NOTE}）"
            )
        );
    }

    #[test]
    fn requires_opt_in_and_can_check_independent_fragments() {
        for (experimental, explicit, expected) in
            [(false, false, 0), (true, false, 1), (false, true, 1)]
        {
            let engine = Engine::with_rules(
                vec![
                    Box::new(StockCloser::new(Genre::General)),
                    Box::new(PhraseRule::new(&catalog::P18)),
                ],
                EngineOptions {
                    experimental,
                    selection: Selection {
                        cli_enable: if explicit { vec!["P27".into()] } else { vec![] },
                        ..Selection::default()
                    },
                    ..EngineOptions::default()
                },
            )
            .unwrap();
            let mut doc = Document::plain_text("参考になれば幸いです。");
            assert_eq!(engine.lint(doc.clone()).diagnostics.len(), expected);
            doc.kind = DocumentKind::Fragments;
            assert_eq!(engine.lint(doc).diagnostics.len(), expected);
        }
        let md = "- 参考になれば幸いです。\n\n> お役に立てれば幸いです。\n";
        let rule = StockCloser::new(Genre::General);
        assert!(run(&rule, md).is_empty());
        assert_eq!(
            run_with(
                &rule,
                md,
                Options {
                    scope: Scope::ALL,
                    ..Options::default()
                }
            )
            .len(),
            2
        );
    }
    #[test]
    fn tech_is_default_and_other_genres_require_opt_in() {
        for genre in Genre::ALL {
            for experimental in [false, true] {
                let engine = Engine::new(EngineOptions {
                    genre,
                    experimental,
                    ..Default::default()
                })
                .unwrap();
                let file = engine.lint(Document::plain_text("参考になれば幸いです。"));
                let findings: Vec<_> = file
                    .diagnostics
                    .iter()
                    .filter(|d| d.rule_id == "P27")
                    .collect();
                assert_eq!(
                    findings.len(),
                    usize::from(genre == Genre::Tech || experimental)
                );
                if let Some(d) = findings.first() {
                    assert_eq!(
                        d.status,
                        if genre == Genre::Tech {
                            RuleStatus::Stable
                        } else {
                            RuleStatus::Experimental
                        }
                    );
                }
            }
        }
        let engine = Engine::new(EngineOptions {
            genre: Genre::Tech,
            selection: Selection {
                no_readability: true,
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        assert!(engine.active_ids().contains(&"P27"));
        let engine = Engine::new(EngineOptions {
            genre: Genre::Tech,
            selection: Selection {
                config_disable: vec!["P27".into()],
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();
        assert!(!engine.active_ids().contains(&"P27"));
    }
    #[test]
    fn asymmetric_quotes_close_even_after_an_unpaired_inch_mark() {
        for genre in [Genre::General, Genre::Tech] {
            let rule = StockCloser::new(genre);
            assert!(
                run(
                    &rule,
                    "「画面は5\"で、皆さんの参考になれば幸いです」と書く例です。"
                )
                .is_empty()
            );
            let md = "画面は5\"です。参考になれば幸いです。\"file\"を開きます。";
            assert_eq!(matched(md, &run(&rule, md)), ["参考になれば幸いです"]);
        }
    }
}
