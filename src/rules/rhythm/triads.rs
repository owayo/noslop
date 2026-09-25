//! R16: 抽象的な評価語の三項列挙が複数の文で反復する。

use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};
use crate::text;

use super::{option_count, unknown_option};

static META: RuleMeta = RuleMeta {
    id: "R16",
    name: "REPEATED_EVALUATIVE_TRIAD",
    title: "評価語の三項列挙の反復",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "短い評価語・抽象語の三項列挙が複数の文に繰り返し現れる箇所を指摘する (実験的)",
    explanation: r"### 何を見るか

地の文で、評価語や抽象語を読点「、」「，」または中黒「・」でちょうど 3 項並べる形が、3 文以上 (min_count で変更可能。最小 2 文) に現れるときに指します。同じ文に複数あっても 1 文と数え、最初の三項列挙を指します。

対象語は「速く・早く・迅速・柔軟・直感的・効率的・効果的・革新的・画期的・安全・便利・簡単・高品質・効率・品質・成長・信頼・安心・価値・持続可能」です。「に」「で」「な」を伴う形も含みます。物品名・数値・手順の動詞は対象語に含めません。4 項以上の列挙、括弧や引用符を含む文、コード・URL などのプレースホルダを含む文は除きます。

### なぜ問題か

評価を三つずつ並べる型が続くと、何がどのように良いのかを説明せずに網羅した印象だけを作れます。ただし、三つの評価軸を比較するために必要な場合もあります。

### 直し方

それぞれの項目が具体的な違いや根拠を伝えているか確認してください。列挙を残す理由があれば残します。三つを二つや四つに変えるだけの修正は不要です。

### 例

- 直す前: 各節で「迅速、柔軟、直感的」「効率、品質、成長」「信頼、安心、価値」と評価を並べる。
- 直した後: 各節で、実際に短縮した作業や減った誤りを説明する。

### 根拠

未校正の実験的なルールです。Wikipedia の編集者向け観察集 Signs of AI writing の Rule of three を着想にしています (https://en.wikipedia.org/wiki/Wikipedia:Signs_of_AI_writing)。日本語の対象語と 3 文の暫定値は独自に選んだもので、誤検知率は測っていません。人も使う列挙なので、単発では指摘しません。",
};

static TRIAD: LazyLock<Regex> = LazyLock::new(|| {
    let item = r"(?:持続可能|直感的|効率的|効果的|革新的|画期的|高品質|迅速|柔軟|安全|便利|簡単|効率|品質|成長|信頼|安心|価値|速く|早く)(?:に|で|な)?";
    Regex::new(&format!(r"{item}(?:\s*[、，・]\s*{item}){{2,}}")).expect("triad regex")
});

fn triads(ctx: &RuleContext<'_>) -> Vec<(Span, Span)> {
    ctx.doc
        .prose_sentences()
        .filter_map(|s| {
            let value = ctx.doc.sentence_text(s);
            if value.chars().any(|c| {
                text::closing_bracket(c).is_some()
                    || text::is_closing_bracket(c)
                    || c == text::PLACEHOLDER
            }) {
                return None;
            }
            let hit = TRIAD.find_iter(value).find(|m| {
                let before = value[..m.start()].trim_end().chars().next_back();
                let after = value[m.end()..].trim_start().chars().next();
                let boundary = |c| text::is_nounish(c) || matches!(c, '、' | '，' | '・');
                m.as_str()
                    .chars()
                    .filter(|c| matches!(c, '、' | '，' | '・'))
                    .count()
                    == 2
                    && !before.is_some_and(boundary)
                    && !after.is_some_and(boundary)
            })?;
            // 「直感的です」の「で」を連用形として飲み込んだ場合は評価語までに戻す。
            let end = if hit.as_str().ends_with('で') && value[hit.end()..].starts_with('す') {
                hit.end() - 'で'.len_utf8()
            } else {
                hit.end()
            };
            let span =
                ctx.doc.blocks[s.block].to_source(s.range.start + hit.start()..s.range.start + end);
            Some((span, s.span))
        })
        .collect()
}

pub struct RepeatedEvaluativeTriad {
    min_count: usize,
}

impl Default for RepeatedEvaluativeTriad {
    fn default() -> Self {
        Self { min_count: 3 }
    }
}

impl Rule for RepeatedEvaluativeTriad {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_count" => {
                let n = option_count(key, value)?;
                if n < 2 {
                    return Err("`min_count` には 2 以上の整数を指定してください".into());
                }
                self.min_count = n;
            }
            _ => return Err(unknown_option(&META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![("min_count", self.min_count.to_string())]
    }

    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let n = triads(ctx).len();
        if n == 0 {
            Vec::new()
        } else {
            vec![Measure::new(
                "triad_sentences",
                n as f64,
                "min_count",
                Fires::AtOrAbove,
            )]
        }
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let found = triads(ctx);
        if found.len() < self.min_count {
            return;
        }
        for &(span, context) in &found {
            out.push(META.diagnostic(span, format!("評価語の三項列挙が {} 文で繰り返されています", found.len()))
                .with_hint("各項目が具体的な違いや根拠を伝えているか確認してください。三つという数だけを変える必要はありません")
                .with_context(context).with_metric("triad_sentences", found.len()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{matched, run};

    const SAMPLE: &str = "操作は速く、柔軟で、直感的です。導入で効率、品質、成長を支えます。運用で信頼、安心、価値を届けます。";

    #[test]
    fn points_to_each_repeated_triad() {
        let d = run(&RepeatedEvaluativeTriad::default(), SAMPLE);
        assert_eq!(
            matched(SAMPLE, &d),
            vec![
                "速く、柔軟で、直感的",
                "効率、品質、成長",
                "信頼、安心、価値"
            ]
        );
        let md = SAMPLE.replace("効率、品質、成長", "**効率**、品質、成長");
        assert_eq!(run(&RepeatedEvaluativeTriad::default(), &md).len(), 3);
    }

    #[test]
    fn ignores_single_triads_real_items_and_longer_lists() {
        for s in [
            "操作は速く、柔軟で、直感的です。",
            "ボルト、ナット、座金を用意する。",
            "入力し、確認し、送信する。",
            "受付は月曜、水曜、金曜です。",
            "値は1、2、3です。",
            "効率、品質、成長、安心を求める。",
            "部材、効率、品質、成長を比べる。",
            "効率、品質、成長、部材を比べる。",
            "「効率、品質、成長」と書いてある。",
        ] {
            let md = if s.starts_with("操作") {
                s.to_string()
            } else {
                s.repeat(3)
            };
            assert!(
                run(&RepeatedEvaluativeTriad::default(), &md).is_empty(),
                "{md}"
            );
        }
    }

    #[test]
    fn uses_prose_and_sentence_count() {
        for md in [
            format!("- {SAMPLE}\n"),
            format!("> {SAMPLE}\n"),
            format!("```\n{SAMPLE}\n```\n"),
        ] {
            assert!(run(&RepeatedEvaluativeTriad::default(), &md).is_empty());
        }
        let s = "効率、品質、成長と信頼、安心、価値を求める。";
        assert!(run(&RepeatedEvaluativeTriad::default(), s).is_empty());
        let mut rule = RepeatedEvaluativeTriad::default();
        rule.configure("min_count", &toml::Value::Integer(2))
            .unwrap();
        assert_eq!(run(&rule, &s.repeat(2)).len(), 2);
        assert!(
            rule.configure("min_count", &toml::Value::Integer(1))
                .is_err()
        );
    }
}
