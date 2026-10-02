//! P26: 行動をぼかす業界語を、業務の対象と組み合わせて拾う。

use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::rules::{Rule, RuleContext, RuleMeta, RuleUnit, quote};

use super::engine::{Entry, Matcher, WEAK_SIGNAL_NOTE, diagnostic};

static META: RuleMeta = RuleMeta {
    id: "P26",
    name: "ACTION_JARGON",
    title: "行動をぼかす業界語",
    lane: Lane::Readability,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "業務の対象に使う「打ち手」「落とし込む」「巻き取る」「握る」を具体化する (実験的)",
    explanation: r"### 何を見るか

「次の打ち手」「打ち手を決める」のような行動の候補と、運用・計画などに「落とし込む」、作業・対応などを「巻き取る」、期限・方針などを「握る」という組み合わせを探します。動詞の活用形も含み、対象と動詞の間は同じ文の 20 字以内で、主題直後の読点を除き、「を」「は」「も」の文字・読点・括弧を挟まない範囲に限ります。助詞の働きは解析しないため、「こちらでも」などを挟む形も拾いません。「運用への落とし込み」のような名詞の形や「仕様として落とし込む」も対象外です。

「ハンドルを握る」「ケーブルを巻き取る」のような物理的な対象には当てません。「打ち手」は同じ文に囲碁・将棋・対局などの語があると数えません。有効にしたときは地の文の段落だけを見て、リスト・表・引用は語句ルールのスコープ設定に従います。

### なぜ問題か

業界語だけで行動をまとめると、読み手が実際にすることを推測しなければなりません。ただし、対象・担当・手順が明らかな文では、同じ語でも意味が通ることがあります。AI らしさではなく、読みやすさの推敲の手掛かりです。

### 直し方

必要なら「反映する」「引き取る」「合意する」など、実際にすることに置き換えます。誰が・いつ・何をするかを本文で確かめてから書き、原文にない担当者や日付は足さないでください。具体的な説明が既にある場合は残せます。

### 例

- 直す前: 方針を運用に落とし込み、残った作業を巻き取ります。
- 直した後: 方針を運用手順に反映し、残った作業を引き取ります。

### 根拠

人の文書 171 本・生成文書 390 本ではどちらも 0 本に反応しました。業務の文章での出方を評価するには足りず、実験的な情報の指摘です。対象の語と距離で近似し、実際に行動が曖昧かは判定しません。業務文書での校正を続けてから、既定で有効にするかを決めます。",
};

const ENTRIES: &[Entry] = &[
    Entry::exp_re(
        r"(?:次の|次なる|今後の|当面の|対応の|改善の|ときの|時の|具体的な|有効な|取るべき)(?P<m>打ち手)",
        Severity::Info,
    ),
    Entry::exp_re(
        r"(?P<m>打ち手)を[^。！？、，,をはも「」『』（）()]{0,20}?(?:決め|考え|検討|選ぶ|選び|用意)",
        Severity::Info,
    ),
    Entry::exp_re(
        r"(?:運用|業務|計画|手順|仕様|設計|施策)に[^。！？、，,をはも「」『』（）()]{0,20}?(?P<m>落とし込[まみむめもん])",
        Severity::Info,
    ),
    Entry::exp_re(
        r"(?:作業|業務|タスク|対応|案件|残務|仕事)[をはも][、，,]?[^。！？、，,をはも「」『』（）()]{0,20}?(?P<m>巻き取[らりるれろっ])",
        Severity::Info,
    ),
    Entry::exp_re(
        r"(?:期限|納期|日程|方針|条件|仕様)[をはも][、，,]?[^。！？、，,をはも「」『』（）()]{0,20}?(?P<m>握[らりるれろっ])",
        Severity::Info,
    ),
];
const ITEMS: [&str; ENTRIES.len()] = ["打ち手", "打ち手", "落とし込む", "巻き取る", "握る"];
// ENTRIES の先頭 2 項目は「打ち手」。対象の追加・順序変更時は ITEMS もそろえる。
const GAME_SENSITIVE: usize = 2;
static GAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"囲碁|将棋|棋士|対局|盤面|碁盤|麻雀|雀士|オセロ|太鼓|鐘")
        .expect("盤上の打ち手の文脈")
});

pub(super) struct ActionJargon {
    matcher: Matcher,
}

impl ActionJargon {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(ENTRIES, true),
        }
    }
}

impl Rule for ActionJargon {
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
                for hit in self.matcher.find(value) {
                    if hit.entry < GAME_SENSITIVE && GAME.is_match(value) {
                        continue;
                    }
                    if hit.entry >= GAME_SENSITIVE
                        && (value[..hit.range.start].ends_with('の')
                            || value[hit.range.end..]
                                .chars()
                                .next()
                                .is_some_and(crate::text::is_kanji))
                    {
                        continue;
                    }
                    let matched = &value[hit.range.clone()];
                    let span = block.to_source(
                        sentence.range.start + hit.range.start
                            ..sentence.range.start + hit.range.end,
                    );
                    out.push(diagnostic(
                        &META, span, sentence.span, matched,
                        format!("「{}」で実際にすることが読み手に伝わるか確かめてください。{WEAK_SIGNAL_NOTE}", quote(matched)),
                        "誰が・いつ・何をするかを確かめ、実際にすることを書いてください。原文にない担当や期限は足さないでください",
                        Severity::Info, RuleStatus::Experimental,
                    ).with_metric("item", ITEMS[hit.entry]));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Metric;
    use crate::document::{Document, DocumentKind};
    use crate::engine::{Engine, EngineOptions, Selection};
    use crate::rules::{
        Scope,
        testing::{Options, matched, run, run_with},
    };

    #[test]
    fn finds_business_actions_and_the_original_spans() {
        for (md, expected, item) in [
            (
                "依頼するときは、期限を過ぎたときの打ち手を先に決めます。",
                "打ち手",
                "打ち手",
            ),
            (
                "遅れが出たら、次の**打ち手**を考えます。",
                "打ち手",
                "打ち手",
            ),
            (
                "決まった方針を運用に落とし込みます。",
                "落とし込み",
                "落とし込む",
            ),
            (
                "残った作業は、こちらで巻き取ります。",
                "巻き取り",
                "巻き取る",
            ),
            ("期限は、先方のリーダーと握っておきます。", "握っ", "握る"),
        ] {
            let findings = run(&ActionJargon::new(), md);
            assert_eq!(matched(md, &findings), [expected], "{md}");
            assert_eq!(
                findings[0].metrics.get("item"),
                Some(&Metric::Text(item.into()))
            );
            assert_eq!(findings[0].lane, Lane::Readability);
            assert_eq!(findings[0].severity, Severity::Info);
            assert_eq!(findings[0].status, RuleStatus::Experimental);
            assert!(findings[0].message.contains(WEAK_SIGNAL_NOTE));
        }
    }

    #[test]
    fn ignores_physical_actions_and_other_objects() {
        for md in [
            "担当は田中さんで、ケーブルの巻き取りを行う。",
            "担当は板前で、毎日握っている。",
            "予算を握る部署に相談する。",
            "麻雀の強い打ち手を集める。",
            "太鼓の次の打ち手を決める。",
            "作業はケーブルの巻き取りを行う。",
            "ハンドルを握る。",
            "ケーブルを巻き取る。",
            "囲碁の打ち手を決める。",
            "将棋の次の打ち手を考える。",
            "作業はケーブルを巻き取ることです。",
            "期限はハンドルを握る前に確認する。",
            "作業の写真を巻き取る装置で撮る。",
            "運用に必要な布を落とし込む。",
            "作業を確認する。別の人が巻き取る。",
            "`期限を握る` は例です。",
        ] {
            assert!(run(&ActionJargon::new(), md).is_empty(), "{md}");
        }
    }

    #[test]
    fn handles_conjugations_shared_context_and_distance() {
        for md in [
            "打ち手を決める。",
            "計画に落とし込む。",
            "計画に落とし込んだ。",
            "対応を巻き取る。",
            "対応を巻き取った。",
            "対応を巻き取らない。",
            "方針を握る。",
            "方針を握った。",
            "方針を握らない。",
        ] {
            assert_eq!(run(&ActionJargon::new(), md).len(), 1, "{md}");
        }
        let md = "次の打ち手を決め、作業を巻き取り、期限を握る。";
        assert_eq!(
            matched(md, &run(&ActionJargon::new(), md)),
            ["打ち手", "巻き取り", "握る"]
        );
        let md = "前置きです。期限を握る。\n\n次の段落です。作業を巻き取る。";
        assert_eq!(
            matched(md, &run(&ActionJargon::new(), md)),
            ["握る", "巻き取る"]
        );
        assert_eq!(
            run(&ActionJargon::new(), "将棋の対局後、作業を巻き取る。").len(),
            1
        );
        let md = format!("期限を{}握る。", "あ".repeat(20));
        assert_eq!(run(&ActionJargon::new(), &md).len(), 1);
        let md = format!("期限を{}握る。", "あ".repeat(21));
        assert!(run(&ActionJargon::new(), &md).is_empty());
    }

    #[test]
    fn scopes_and_opt_in_apply_to_prose_and_fragments() {
        let md = "# 期限を握る\n\n- 期限を握る。\n\n> 期限を握る。\n";
        assert!(run(&ActionJargon::new(), md).is_empty());
        assert_eq!(
            run_with(
                &ActionJargon::new(),
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
                vec![Box::new(ActionJargon::new())],
                EngineOptions {
                    experimental,
                    selection: Selection {
                        cli_enable: if explicit { vec!["P26".into()] } else { vec![] },
                        ..Selection::default()
                    },
                    ..EngineOptions::default()
                },
            )
            .unwrap();
            let mut doc = Document::plain_text("期限を握る。");
            assert_eq!(engine.lint(doc.clone()).diagnostics.len(), expected);
            doc.kind = DocumentKind::Fragments;
            assert_eq!(engine.lint(doc).diagnostics.len(), expected);
        }
    }
}
