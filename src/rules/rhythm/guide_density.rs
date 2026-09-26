//! R17: 解説に使う定型句が地の文に密集している。

use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::genre::Genre;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};
use crate::text;

use super::{option_count, option_f64_in, unknown_option};

static META: RuleMeta = RuleMeta {
    id: "R17",
    name: "GUIDE_CLICHE_DENSITY",
    title: "解説の定型句の密集",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "安心させる言葉・重要性の強調・効果の説明などの定型句が、地の文に密集する文書を指摘する (実験的)",
    explanation: r"### 何を見るか

地の文が 1500 字以上 (min_chars) あり、解説の定型句が 1000 字あたり 2 件以上 (min_per_1000_chars) 現れる文書を指します。安心させる言葉、重要性の強調、効果の説明、解説の予告、個人への勧め、成功の比喩の 6 系統のうち、3 系統以上があることも条件です。

例えば「必要はありません」「が大切です」「やすくなります」「ここでは〜紹介します」「自分に合った」「第一歩」を数えます。安心・重要性・効果・予告を言い切る型は文末に限り、「重要だった」「やすくなるかどうか」「につながる道」は数えません。「大切なのは、」のような書き出しは別に数えます。同じ箇所を重複して数えません。字数には見出し・リスト・表・引用ブロックを含めず、空白と文末記号も除きます。括弧・引用符・コードや URL のプレースホルダを含む文は、字数には含めますが定型句を数えません。

general と tech で有効です。最初の該当箇所に 1 件を出し、残りを関連箇所として添えます。

### なぜ問題か

助言や効用の定型句が続くと、手順や条件が乏しくても説明が充実しているように見えます。ただし、個々の語句は人の解説にもよく出る普通の表現です。単語だけでは書き手や執筆方法を推定できません。

### 直し方

安心させる言葉や効果を述べた部分に、具体的な手順・条件・例外があるか確認してください。必要な説明は残します。定型句を別の語に置き換えるだけの修正は不要です。

### 例

- 直す前: 各節で「無理なく」「が大切です」「やすくなります」と勧め、何をどう変えるかは書かない。
- 直した後: 読者が行う手順と、効果を見込める条件を各節に書く。

### 根拠

実験的なルールです。探索用の一般記事で、人の 26 文書中 0 件、生成した 68 文書中 24 件を指摘しました。密度 2 は校正側の人 19 件・生成 40 件で選んだ暫定値です。取り分けた検証側では人 7 件中 0 件、生成 28 件中 12 件でしたが、人の誤検知率の片側 95% 上限は 27.9% です。話題と長さをそろえた独立の資料による検証も済んでいないため、既定では動かさず、情報レベルで扱います。測定条件は docs/validation-2026-09.md にまとめています。",
};

// 1 系統だけの反復と、複数の定型的な働きかけが重なる文書とを分ける。
static FAMILIES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?P<m>必要は(?:ありません|ない)|(?:ても|でも)(?:大丈夫(?:です)?|構いません|かまいません|問題ありません))(?:[。！!]|$)",
        r"(?P<m>が(?:大切|大事|重要)(?:です|だ|である))(?:[。！!]|$)|(?:大切|大事|重要|肝心)なのは[、，]",
        r"(?P<m>につなが(?:ります|る)|やすくな(?:ります|る)|可能にな(?:ります|る)|を防げ(?:ます|る))(?:[。！!]|$)",
        r"(?P<m>(?:ここ|本記事|この記事)では[、，]?[^。！？\n]{0,40}(?:紹介|解説|説明|整理)します)(?:[。！!]|$)",
        r"無理(?:なく|のない)|自分に合(?:った|う)",
        r"第一歩|秘訣|(?:鍵|カギ)(?:に|と)な(?:ります|る)",
    ].iter().map(|s| Regex::new(s).expect("guide phrase regex")).collect()
});

struct Profile {
    chars: usize,
    hits: Vec<(Span, Span)>,
}

impl Profile {
    fn density(&self) -> f64 {
        self.hits.len() as f64 * 1000.0 / self.chars as f64
    }
}

pub struct GuideClicheDensity {
    min_chars: usize,
    min_per_1000_chars: f64,
}

impl Default for GuideClicheDensity {
    fn default() -> Self {
        Self {
            min_chars: 1500,
            min_per_1000_chars: 2.0,
        }
    }
}

impl GuideClicheDensity {
    fn profile(&self, ctx: &RuleContext<'_>) -> Option<Profile> {
        let mut profile = Profile {
            chars: 0,
            hits: Vec::new(),
        };
        let mut families = [false; 6];
        for sentence in ctx.doc.prose_sentences() {
            profile.chars += sentence.length;
            let value = ctx.doc.sentence_text(sentence);
            if value.chars().any(|c| {
                text::closing_bracket(c).is_some()
                    || text::is_closing_bracket(c)
                    || c == text::PLACEHOLDER
            }) {
                continue;
            }
            let mut matches = Vec::new();
            for (family, pattern) in FAMILIES.iter().enumerate() {
                matches.extend(pattern.captures_iter(value).filter_map(|captures| {
                    let m = captures.name("m").or_else(|| captures.get(0))?;
                    Some((m.start(), m.end(), family))
                }));
            }
            matches.sort_by_key(|&(start, end, _)| (start, std::cmp::Reverse(end)));
            let mut previous_end = 0;
            for (start, end, family) in matches {
                if start < previous_end {
                    continue;
                }
                previous_end = end;
                families[family] = true;
                let span = ctx.doc.blocks[sentence.block]
                    .to_source(sentence.range.start + start..sentence.range.start + end);
                profile.hits.push((span, sentence.span));
            }
        }
        (profile.chars >= self.min_chars && families.iter().filter(|&&v| v).count() >= 3)
            .then_some(profile)
    }
}

impl Rule for GuideClicheDensity {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn allowed_in(&self, genre: Genre) -> bool {
        matches!(genre, Genre::General | Genre::Tech)
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_chars" => self.min_chars = option_count(key, value)?,
            "min_per_1000_chars" => {
                self.min_per_1000_chars = option_f64_in(key, value, 0.1, 1000.0)?
            }
            _ => return Err(unknown_option(&META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_chars", self.min_chars.to_string()),
            ("min_per_1000_chars", self.min_per_1000_chars.to_string()),
        ]
    }

    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        self.profile(ctx).map_or_else(Vec::new, |p| {
            vec![Measure::new(
                "cliches_per_1000_chars",
                p.density(),
                "min_per_1000_chars",
                Fires::AtOrAbove,
            )]
        })
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let Some(p) = self.profile(ctx) else {
            return;
        };
        if p.density() < self.min_per_1000_chars {
            return;
        }
        let (span, context) = p.hits[0];
        out.push(META.diagnostic(span, format!("解説の定型句が {} 字に {} 件あります (1000 字あたり {:.1} 件)", p.chars, p.hits.len(), p.density()))
            .with_hint("人の解説にもよく出る語句です。勧めや効果に具体的な手順・条件が伴っているか確認してください")
            .with_context(context).with_related(p.hits.iter().skip(1).map(|&(s, _)| s).collect())
            .with_metric("prose_chars", p.chars).with_metric("cliche_count", p.hits.len()).with_metric("cliches_per_1000_chars", p.density()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{matched, run};

    const GUIDE: &str =
        "すべて変える必要はありません。手順の確認が大切です。誤りを見つけやすくなります。";

    #[test]
    fn requires_length_density_and_several_families() {
        let rule = GuideClicheDensity::default();
        let dense = GUIDE.repeat(50);
        let d = run(&rule, &dense);
        assert_eq!(matched(&dense, &d), ["必要はありません"]);
        assert_eq!(d[0].related.len(), 149);
        assert_eq!(&dense[d[0].related[0].range()], "が大切です");
        assert!(run(&rule, GUIDE).is_empty());
        assert!(run(&rule, &"確認が大切です。".repeat(200)).is_empty());
        assert!(run(&rule, &format!("{GUIDE}{}", "記録を読んだ。".repeat(300))).is_empty());
        let md = dense.replacen("必要はありません", "**必要はありません**", 1);
        assert_eq!(matched(&md, &run(&rule, &md)), ["必要はありません"]);
    }

    #[test]
    fn excludes_past_tense_questions_and_relative_clauses() {
        let rule = GuideClicheDensity::default();
        let record = "当時は現場の判断が重要だった。担当者は変える必要はないかを会議で確認した。作業が見やすくなるかどうかは未確認だ。港につながる道を調べた。事故を防げるかを尋ねた。".repeat(30);
        assert!(run(&rule, &record).is_empty());
        let guide =
            "その方法でも大丈夫です。大切なのは、記録を残すことです。誤りを見つけやすくなります。"
                .repeat(50);
        let d = run(&rule, &guide);
        assert_eq!(matched(&guide, &d), ["でも大丈夫です"]);
        assert_eq!(&guide[d[0].related[0].range()], "大切なのは、");
    }

    #[test]
    fn ignores_nonprose_and_quoted_examples_even_in_long_documents() {
        let rule = GuideClicheDensity::default();
        for md in [
            format!("- {}\n", GUIDE.repeat(40)),
            format!("> {}\n", GUIDE.repeat(40)),
            format!("```text\n{}\n```\n", GUIDE.repeat(40)),
            format!("# {}\n", GUIDE.repeat(40)),
            format!("| 項目 |\n| --- |\n| {} |\n", GUIDE.repeat(40)),
            format!("「{GUIDE}」と書かれていた。").repeat(40),
        ] {
            assert!(run(&rule, &md).is_empty());
        }
    }

    #[test]
    fn validates_options_and_exposes_density_for_calibration() {
        let mut rule = GuideClicheDensity::default();
        rule.configure("min_chars", &toml::Value::Integer(1))
            .unwrap();
        let doc = crate::document::Document::markdown(GUIDE);
        let ctx = RuleContext::new(&doc);
        let measures = rule.measure(&ctx);
        assert_eq!(measures.len(), 1);
        for threshold in [1.0, 1000.0] {
            rule.configure("min_per_1000_chars", &toml::Value::Float(threshold))
                .unwrap();
            assert_eq!(
                measures[0].fires_at(threshold),
                !run(&rule, GUIDE).is_empty()
            );
        }
        assert!(
            rule.configure("min_chars", &toml::Value::Integer(0))
                .is_err()
        );
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                rule.configure("min_per_1000_chars", &toml::Value::Float(bad))
                    .is_err()
            );
        }
        assert!(rule.configure("unknown", &toml::Value::Integer(1)).is_err());
        assert!(!rule.allowed_in(Genre::Essay));
        assert!(!rule.allowed_in(Genre::Business));
    }
}
