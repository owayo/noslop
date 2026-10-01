//! P24: 効果の説明を感覚的な言い回しで済ませている箇所。

use crate::diagnostic::{Lane, RuleStatus, Severity};
use crate::rules::RuleMeta;

use super::engine::{Entry, PhraseSpec};

pub(super) static P24: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P24",
        name: "VAGUE_EFFECT",
        title: "効果をぼかす言い回し",
        lane: Lane::Slop,
        status: RuleStatus::Experimental,
        default_severity: Severity::Info,
        summary: "「地味に効く」のように、何への効果かを言わずに効き目を評する言い回しを見直す (実験的)",
        explanation: r"### 何を見るか

「地味に効く」「地味に効きます」「地味に効いた」「地味に効いてきます」などを探します。「効率」「効用」「効能」といった語は対象にしません。「効く」単独や「よく効く」「じわじわ効く」も対象にしません。既定は段落だけを見て、リスト・表・引用は語句ルールのスコープ設定に従います。

### なぜ問題か

効果を感覚的に評するだけでは、何がどう変わるのかが読み手に伝わりません。ただし、同じ文や前後の文で効果を説明している場合や、書き手自身の実感を述べている場合は、そのままで意味が通ります。これは見直しの候補で、書き手や執筆方法の判定には使えません。

### 直し方

効果の対象や、観測できる変化が分かるか確かめます。分かる場合だけ具体的な操作や結果に書き換えてください。原文にない原因・数値・効果を推測で足さないでください。効果がすでに具体的に説明されている場合や、実感として残す理由がある場合は直さなくてかまいません。

### 例

- 直す前: 入力欄の記入例が地味に効きます。日付の書式を確認する問い合わせが減りました。
- 直した後: 入力欄に記入例を置いたところ、日付の書式を確認する問い合わせが減りました。

### 根拠

人の文書 171 本・生成文書 390 本の地の文で下調べすると、「地味に効く」の活用形は人の文書で 0 本、生成文書で 2 本・2 件でした。件数が少なく、ジャンルをそろえた保留のデータで確かめていないため、未校正の実験的なルールとして情報で扱います。効果が具体的かどうかを機械的に判定するものではありません。",
    },
    entries: &[Entry::exp_re(
        r"(?P<m>地味に効(?:いて(?:き(?:ました|ます|た)|くる|い(?:ます|る|た))?|いた|き(?:ました|ます)?|く))(?:$|[^\p{Han}])",
        Severity::Info,
    )],
    message: "「{m}」の効果の対象や、具体的な変化が説明されているか確かめてください",
    hint: "効果が説明されていれば残せます。具体化するときは原文にない原因や効果を足さないでください",
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, DocumentKind};
    use crate::engine::{Engine, EngineOptions, Selection};
    use crate::rules::phrases::engine::PhraseRule;
    use crate::rules::testing::{Options, matched, run, run_with};
    use crate::rules::{Rule, RuleUnit, Scope};

    #[test]
    fn effect_phrases_keep_their_original_spans() {
        for phrase in [
            "地味に効く",
            "地味に効きます",
            "地味に効きました",
            "地味に効いた",
            "地味に効いてきます",
            "地味に効いてきました",
            "地味に効いてくる",
            "地味に効いている",
        ] {
            let md = format!("記入例が**{phrase}**。\n");
            let findings = run(&PhraseRule::new(&P24), &md);
            assert_eq!(matched(&md, &findings), [phrase]);
            assert_eq!(findings[0].severity, Severity::Info);
            assert_eq!(findings[0].status, RuleStatus::Experimental);
        }
        // 活用の境界にある「、」を範囲に含めない。
        let md = "記入例が地味に効き、問い合わせが減った。";
        assert_eq!(
            matched(md, &run(&PhraseRule::new(&P24), md)),
            ["地味に効き"]
        );
        let md = "記入例が地味に**効く**。";
        assert_eq!(
            matched(md, &run(&PhraseRule::new(&P24), md)),
            ["地味に**効く"]
        );
    }

    #[test]
    fn ignores_other_effect_words_and_code() {
        for md in [
            "作業を地味に効率化する。",
            "記入例には地味に効用がある。",
            "この薬はよく効く。",
            "練習の成果がじわじわ効いてくる。",
            "`地味に効く` は表現の例です。",
            "```text\n地味に効く\n```",
            "記入例は地味だ。効果を調べる。",
        ] {
            assert!(run(&PhraseRule::new(&P24), md).is_empty(), "{md}");
        }
    }

    #[test]
    fn follows_phrase_scope_and_accepts_fragments() {
        let md =
            "# 地味に効く\n\n- 地味に効く\n\n> 地味に効く\n\n| 評価 |\n| --- |\n| 地味に効く |\n";
        let rule = PhraseRule::new(&P24);
        assert!(run(&rule, md).is_empty());
        let findings = run_with(
            &rule,
            md,
            Options {
                scope: Scope::ALL,
                ..Options::default()
            },
        );
        assert_eq!(matched(md, &findings), ["地味に効く"; 3]);
        assert_eq!(rule.unit(), RuleUnit::Sentence);
    }

    #[test]
    fn experimental_or_explicit_selection_is_required() {
        let source = "記入例が地味に効きます。";
        for (experimental, explicit, expected) in
            [(false, false, 0), (true, false, 1), (false, true, 1)]
        {
            let engine = Engine::with_rules(
                vec![Box::new(PhraseRule::new(&P24))],
                EngineOptions {
                    experimental,
                    selection: Selection {
                        cli_enable: if explicit { vec!["P24".into()] } else { vec![] },
                        ..Selection::default()
                    },
                    ..EngineOptions::default()
                },
            )
            .unwrap();
            let result = engine.lint(Document::plain_text(source));
            assert_eq!(result.diagnostics.len(), expected);
            let mut doc = Document::plain_text(source);
            doc.kind = DocumentKind::Fragments;
            assert_eq!(engine.lint(doc).diagnostics.len(), expected);
        }
    }
}
