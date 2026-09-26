//! R18: 名詞句を三つ以上並べる体言止めの反復。

use super::{option_count, unknown_option};
use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::genre::Genre;
use crate::morph::MorphToken;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};
use crate::text;
use hasami::CoarsePos;
use std::ops::Range;

static META: RuleMeta = RuleMeta {
    id: "R18",
    name: "REPEATED_NOMINAL_LIST",
    title: "名詞句を並べる体言止めの反復",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "名詞句を三つ以上並べて終える文が繰り返される箇所を指摘する (実験的)",
    explanation: r"### 何を見るか

地の文で、読点「、」「，」によって三つ以上の名詞句を並べ、句点「。」で終える文が 2 文以上 (min_count。最小 2) あるときに指します。本文は文末記号を除いて 15 字以上、各項目は空白を除いて 3 字以上必要です。general と essay で有効です。

辞書があるときは各項目の最後が名詞か名詞の接尾辞であることを確かめます。名詞を修飾する動詞や形容詞は許します。数詞、係助詞「は」「も」など、括弧・引用符、コード・URL のプレースホルダを含む文は外します。最初の項目が「その結果」「その後」などの接続表現に一致する文も外します。見出し・リスト・表・引用ブロックも数えません。

辞書がない・文を解析できないときは、各項目が漢字・カタカナと「の」だけでできている場合に限る近似を使います。ひらがなの名詞や修飾節を含む列挙を見落とすことがあります。

### なぜ問題か

情景や感覚を名詞句の列挙で何度も締めると、異なる場面にも同じリズムが現れます。一方で、体言止めや列挙は随筆で普通に使う技法でもあります。列挙した内容の意味や書き手の意図は判定していません。

### 直し方

列挙で伝わる情景があるか、繰り返すリズムが狙いに合っているか確認してください。必要なら残し、数だけを変えたり、すべてを「です・ます」に直したりする必要はありません。

### 例

- 直す前: 各場面を「早朝の冷気、駅の階段、店の看板。」のような列挙で繰り返し締める。
- 直した後: 場面ごとに、列挙して見せるか、出来事を動詞で描くかを選ぶ。

### 根拠

実験的なルールです。同梱の IPAdic を固定した探索用の随筆で、人の 27 文書中 0 件、生成した 65 文書中 8 件を指摘しました。取り分けた検証側は人 7 件中 0 件、生成 28 件中 4 件で、人の誤検知率の片側 95% 上限は 27.9% でした。一般記事では人 26 件・生成 68 件ともに指摘はありませんでした。辞書なしでは、同じ随筆 65 件を 1 件も拾えませんでした。辞書あり・なしで検出範囲が異なり、独立した資料での検証も済んでいないため、反復を情報レベルで示すに留めます。測定条件は docs/validation-2026-09.md にまとめています。",
};

fn nominal_part(tokens: &[MorphToken], range: Range<usize>) -> bool {
    let mut last = None;
    for token in tokens
        .iter()
        .filter(|t| t.range.start < range.end && range.start < t.range.end)
    {
        if token.range.start < range.start || token.range.end > range.end {
            return false;
        }
        if matches!(
            token.pos,
            CoarsePos::Numeral | CoarsePos::BindingParticle | CoarsePos::FinalParticle
        ) {
            return false;
        }
        if token.pos != CoarsePos::Symbol {
            last = Some(token.pos);
        }
    }
    matches!(
        last,
        Some(
            CoarsePos::Noun | CoarsePos::ProperNoun | CoarsePos::NounSuffix | CoarsePos::FormalNoun
        )
    )
}

/// 辞書なしでは仮名の活用語を名詞と取り違えないよう、範囲を狭くする。
fn nominal_without_dictionary(part: &str) -> bool {
    part.chars()
        .all(|c| text::is_kanji(c) || text::is_katakana(c) || c == 'の' || c.is_whitespace())
        && part
            .chars()
            .next_back()
            .is_some_and(|c| text::is_kanji(c) || text::is_katakana(c))
}

fn lists(ctx: &RuleContext<'_>) -> Vec<Span> {
    ctx.doc
        .sentences
        .iter()
        .enumerate()
        .filter_map(|(index, sentence)| {
            if !sentence.japanese || !ctx.doc.blocks[sentence.block].is_prose() {
                return None;
            }
            let value = ctx.doc.sentence_text(sentence).trim_end();
            let core = value.strip_suffix('。')?;
            if text::reading_length(core) < 15
                || core.chars().any(|c| {
                    text::closing_bracket(c).is_some()
                        || text::is_closing_bracket(c)
                        || c == text::PLACEHOLDER
                        || c.is_numeric()
                        || text::is_sentence_ender(c)
                })
            {
                return None;
            }
            let parts: Vec<_> = core.split(['、', '，']).collect();
            if parts.len() < 3 {
                return None;
            }
            let tokens = ctx.morph.and_then(|m| m.sentence(index));
            let mut start = 0;
            for (part_index, raw) in parts.into_iter().enumerate() {
                let part = raw.trim();
                if part_index == 0
                    && matches!(
                        part,
                        "その結果"
                            | "この結果"
                            | "その後"
                            | "この後"
                            | "そのため"
                            | "このため"
                            | "その一方"
                            | "その反面"
                    )
                {
                    return None;
                }
                if text::reading_length(part) < 3 {
                    return None;
                }
                let part_start = start + raw.len() - raw.trim_start().len();
                if !tokens.map_or_else(
                    || nominal_without_dictionary(part),
                    |ts| nominal_part(ts, part_start..part_start + part.len()),
                ) {
                    return None;
                }
                start += raw.len() + '、'.len_utf8();
            }
            Some(sentence.span)
        })
        .collect()
}

pub struct RepeatedNominalList {
    min_count: usize,
}

impl Default for RepeatedNominalList {
    fn default() -> Self {
        Self { min_count: 2 }
    }
}

impl Rule for RepeatedNominalList {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }
    fn uses_morphology(&self) -> bool {
        true
    }
    fn allowed_in(&self, genre: Genre) -> bool {
        matches!(genre, Genre::General | Genre::Essay)
    }
    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        if key != "min_count" {
            return Err(unknown_option(&META, key));
        }
        let n = option_count(key, value)?;
        if n < 2 {
            return Err("`min_count` には 2 以上の整数を指定してください".into());
        }
        self.min_count = n;
        Ok(())
    }
    fn options(&self) -> Vec<(&'static str, String)> {
        vec![("min_count", self.min_count.to_string())]
    }
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let n = lists(ctx).len();
        // 単発は反復ではない。許される閾値は 2 以上なので、掃引にも含めない。
        if n < 2 {
            Vec::new()
        } else {
            vec![Measure::new(
                "nominal_list_sentences",
                n as f64,
                "min_count",
                Fires::AtOrAbove,
            )]
        }
    }
    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let found = lists(ctx);
        if found.len() < self.min_count {
            return;
        }
        for &span in &found {
            out.push(META.diagnostic(span, format!("名詞句を三つ以上並べる体言止めが {} 文に現れます", found.len()))
                .with_hint("人の随筆にもある書き方です。列挙で伝わる情景と、反復するリズムが狙いに合うか確認してください")
                .with_context(span).with_metric("nominal_list_sentences", found.len()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::morph::{Morphology, testing::morphology};
    use crate::rules::testing::{matched, run, run_with_morphology};

    const NOMINAL: &str = "早朝の冷気、駅の階段、店の看板。";

    fn dictionary() -> Morphology {
        morphology(&[
            ("早朝の冷気", "名詞,一般,*,*"),
            ("駅の階段", "名詞,一般,*,*"),
            ("店の看板", "名詞,一般,*,*"),
            ("川の流れ", "名詞,一般,*,*"),
            ("森のざわめき", "名詞,一般,*,*"),
            ("土のにおい", "名詞,一般,*,*"),
            ("その結果", "名詞,副詞可能,*,*"),
            ("その後", "名詞,副詞可能,*,*"),
            ("見る", "動詞,自立,*,*"),
            ("です", "助動詞,*,*,*"),
            ("も", "助詞,係助詞,*,*"),
        ])
    }

    #[test]
    fn detects_repeated_lists_and_preserves_source_positions() {
        let s = format!("{NOMINAL}川の流れ、森のざわめき、土のにおい。");
        let rule = RepeatedNominalList::default();
        assert_eq!(
            matched(&s, &run_with_morphology(&rule, &s, &dictionary())),
            [NOMINAL, "川の流れ、森のざわめき、土のにおい。"]
        );
        // 仮名の名詞がある二文目は、辞書なしの近似では数えない。
        assert!(run(&rule, &s).is_empty());
        let md = format!("**早朝の冷気**、駅の階段、店の看板。\n\n{NOMINAL}");
        let d = run_with_morphology(&rule, &md, &dictionary());
        assert_eq!(
            matched(&md, &d),
            ["早朝の冷気**、駅の階段、店の看板。", NOMINAL]
        );
        assert_eq!(run(&rule, &NOMINAL.repeat(2)).len(), 2);
    }

    #[test]
    fn excludes_predicates_numbers_short_items_and_nonprose() {
        let rule = RepeatedNominalList::default();
        for md in [
            NOMINAL.to_string(),
            "早朝の冷気、駅の階段、店の看板です。".repeat(2),
            "早朝の冷気も、駅の階段、店の看板。".repeat(2),
            "早朝の冷気、駅の階段、看板を見る。".repeat(2),
            "赤色、白色、青色。".repeat(2),
            "早朝の冷気、駅の階段。".repeat(2),
            "10時の時計、20時の時計、30分の休憩。".repeat(2),
            "その結果、早朝の冷気、駅の階段。".repeat(2),
            "その後、早朝の冷気、森のざわめき。".repeat(2),
            format!("- {NOMINAL}\n").repeat(2),
            format!("> {NOMINAL}\n").repeat(2),
            format!("# {NOMINAL}\n").repeat(2),
            format!("```text\n{}\n```\n", NOMINAL.repeat(2)),
            format!("「{NOMINAL}」").repeat(2),
            "早朝の冷気、`駅の階段`、店の看板。".repeat(2),
        ] {
            assert!(
                run_with_morphology(&rule, &md, &dictionary()).is_empty(),
                "{md}"
            );
        }
    }

    #[test]
    fn permits_noun_modifiers_and_excludes_kanji_numerals() {
        let dictionary = morphology(&[
            ("風", "名詞,一般,*,*"),
            ("に", "助詞,格助詞,一般,*"),
            ("揺れる", "動詞,自立,*,*"),
            ("看板", "名詞,一般,*,*"),
            ("木陰", "名詞,一般,*,*"),
            ("の", "助詞,連体化,*,*"),
            ("小道", "名詞,一般,*,*"),
            ("朝", "名詞,一般,*,*"),
            ("広場", "名詞,一般,*,*"),
            ("三", "名詞,数,*,*"),
            ("匹", "名詞,接尾,助数詞,*"),
            ("子犬", "名詞,一般,*,*"),
        ]);
        let rule = RepeatedNominalList::default();
        let prose = "風に揺れる看板、木陰の小道、朝の広場。".repeat(2);
        assert_eq!(run_with_morphology(&rule, &prose, &dictionary).len(), 2);
        assert!(run(&rule, &prose).is_empty());
        let numeric = "三匹の子犬、木陰の小道、朝の広場。".repeat(2);
        assert!(run_with_morphology(&rule, &numeric, &dictionary).is_empty());
    }

    #[test]
    fn calibration_matches_check_with_dictionary_and_overrides() {
        let dictionary = dictionary();
        let mut rule = RepeatedNominalList::default();
        for count in [1, 2, 3] {
            let doc = crate::document::Document::markdown(NOMINAL.repeat(count));
            let morph = dictionary.for_document(&doc);
            let ctx = RuleContext {
                morph: Some(&morph),
                ..RuleContext::new(&doc)
            };
            for threshold in [2, 3, 4] {
                rule.configure("min_count", &toml::Value::Integer(threshold))
                    .unwrap();
                let mut d = Vec::new();
                rule.check(&ctx, &mut d);
                assert_eq!(
                    rule.measure(&ctx)
                        .iter()
                        .any(|m| m.fires_at(threshold as f64)),
                    !d.is_empty()
                );
            }
        }
        assert!(
            rule.configure("min_count", &toml::Value::Integer(1))
                .is_err()
        );
        assert!(
            rule.configure("min_count", &toml::Value::Float(2.0))
                .is_err()
        );
        assert!(rule.configure("unknown", &toml::Value::Integer(2)).is_err());
        assert!(!rule.allowed_in(Genre::Tech));
        assert!(!rule.allowed_in(Genre::Business));
    }
}
