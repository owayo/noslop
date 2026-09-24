//! 段落を単位に見るルール。
//!
//! - R07 UNIFORM_PARAGRAPHS: どの段落もほぼ同じ文数でできている
//! - R09 PARAGRAPH_LEAD_CONJUNCTION: 段落の先頭が接続詞で始まる段落が多い

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::document::Block;
use crate::genre::Genre;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};
use crate::text;

use super::{mean_sd, option_count, option_f64_in, unknown_option};

/// 地の文の段落 (添字つき)。日本語の文を 1 つも含まない段落 (英語の段落など) は除く。
///
/// 母集団を、ほかのリズム系ルールの [`crate::document::Document::prose_sentences`]
/// (日本語の地の文) とそろえる。英語の段落が混ざると段落の文数の統計が崩れ、英語だけの
/// 文書 (README の英語版など) にまで指摘が出るため。
fn prose_paragraphs<'a>(ctx: &RuleContext<'a>) -> Vec<(usize, &'a Block)> {
    ctx.doc
        .blocks
        .iter()
        .enumerate()
        .filter(|&(idx, b)| b.is_prose() && japanese_sentences(ctx, idx) > 0)
        .collect()
}

/// 段落の日本語の文の数。
fn japanese_sentences(ctx: &RuleContext<'_>, idx: usize) -> usize {
    ctx.doc
        .block_sentences(idx)
        .iter()
        .filter(|s| s.japanese)
        .count()
}

// ---------------------------------------------------------------------------
// R07 UNIFORM_PARAGRAPHS
// ---------------------------------------------------------------------------

static UNIFORM_META: RuleMeta = RuleMeta {
    id: "R07",
    name: "UNIFORM_PARAGRAPHS",
    title: "段落の均質さ",
    lane: Lane::Slop,
    status: RuleStatus::Stable,
    default_severity: Severity::Info,
    summary: "どの段落もほぼ同じ文数でできていて、段落の厚みに濃淡がない",
    explanation: UNIFORM_EXPLANATION,
};

const UNIFORM_EXPLANATION: &str = "\
### 何を見るか

地の文の段落が 4 つ以上ある文書で、段落ごとの文数の変動係数 (標準偏差 ÷ 平均) が 0.15 未満、\
つまりどの段落もほぼ同じ文数でできているときに 1 件出します。数えるのは日本語の文だけで、\
日本語の文を含まない段落 (英語の段落など) は段落の数にも入れません。

### なぜ問題か

人が書くと、じっくり語る段落もあれば、短く済ませる段落もあります。生成された文章は「3 文で 1 段落」\
のような型を繰り返しやすく、どの論点にも同じ厚みを配ってしまいます。濃淡のなさは、どこが重要なのか\
を読み手に伝えません。

### 直し方

最も伝えたい論点の段落には経緯や数値を足して厚くし、前提の確認にすぎない段落は短く切り上げて\
ください。段落の文数は事前に決めず、内容の重さで決めます。

### 例

- 直す前: どの段落も「主張・理由・まとめ」の 3 文でできている。
- 直した後: 判断の分かれ目になった論点の段落に経緯と数値を足して 6 文にし、前提の確認は 1 文の\
段落にまとめる。

### 根拠

既定で動かす検出器ですが、人の文書との差を示す数値はまだ十分ではないため、情報に留めています。\
段落の文数だけを見るので、一文ずつ段落を分ける書式の文書でも反応します。
";

/// R07 UNIFORM_PARAGRAPHS。
pub struct UniformParagraphs {
    min_paragraphs: usize,
    cv_threshold: f64,
}

impl UniformParagraphs {
    pub fn new(_genre: Genre) -> Self {
        Self {
            min_paragraphs: 4,
            cv_threshold: 0.15,
        }
    }

    /// 段落ごとの (日本語の) 文数と、その (平均, 変動係数)。段落が足りないか平均が 0 以下なら `None`。
    fn stats(
        ctx: &RuleContext<'_>,
        paragraphs: &[(usize, &Block)],
        min_paragraphs: usize,
    ) -> Option<(Vec<usize>, f64, f64)> {
        if paragraphs.len() < min_paragraphs {
            return None;
        }
        let counts: Vec<usize> = paragraphs
            .iter()
            .map(|&(idx, _)| japanese_sentences(ctx, idx))
            .collect();
        let values: Vec<f64> = counts.iter().map(|&c| c as f64).collect();
        let (mean, sd) = mean_sd(&values)?;
        if mean <= 0.0 {
            return None;
        }
        Some((counts, mean, sd / mean))
    }
}

impl Rule for UniformParagraphs {
    fn meta(&self) -> &'static RuleMeta {
        &UNIFORM_META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_paragraphs" => self.min_paragraphs = option_count(key, value)?.max(2),
            "cv_threshold" => self.cv_threshold = option_f64_in(key, value, 0.0, 10.0)?,
            _ => return Err(unknown_option(&UNIFORM_META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_paragraphs", self.min_paragraphs.to_string()),
            ("cv_threshold", self.cv_threshold.to_string()),
        ]
    }

    /// 地の文の段落が `min_paragraphs` 以上ある文書で、段落ごとの文数の変動係数を
    /// `cv_threshold` と比べる値として返す (これより小さいと指摘する)。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let paragraphs = prose_paragraphs(ctx);
        Self::stats(ctx, &paragraphs, self.min_paragraphs)
            .map(|(_, _, cv)| {
                vec![Measure::new(
                    "paragraph_sentence_cv",
                    cv,
                    "cv_threshold",
                    Fires::Below,
                )]
            })
            .unwrap_or_default()
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let paragraphs = prose_paragraphs(ctx);
        let Some((counts, mean, cv)) = Self::stats(ctx, &paragraphs, self.min_paragraphs) else {
            return;
        };
        if cv >= self.cv_threshold {
            return;
        }
        let shown: Vec<String> = counts.iter().take(12).map(|c| c.to_string()).collect();
        let mut list = shown.join(", ");
        if counts.len() > 12 {
            list.push_str(", …");
        }
        let (first_idx, first_block) = paragraphs[0];
        let span = ctx
            .doc
            .block_sentences(first_idx)
            .first()
            .map_or(first_block.span, |s| s.span);
        out.push(
            UNIFORM_META
                .diagnostic(
                    span,
                    format!(
                        "地の文の段落 {} 個が、どれもほぼ同じ文数でできています (文数 {list}、変動係数 {cv:.2})",
                        paragraphs.len()
                    ),
                )
                .with_hint(
                    "重要な段落は具体例や数値で厚くし、補足の段落は短く切り上げるなど、段落の厚みに\
                     差をつけてください",
                )
                .with_related(paragraphs.iter().map(|(_, b)| b.span).collect())
                .with_metric("paragraphs", paragraphs.len())
                .with_metric("mean_sentences", mean)
                .with_metric("cv", cv)
                .with_metric("counts", counts.iter().map(usize::to_string).collect::<Vec<_>>().join(",")),
        );
    }
}

// ---------------------------------------------------------------------------
// R09 PARAGRAPH_LEAD_CONJUNCTION
// ---------------------------------------------------------------------------

static CONJ_META: RuleMeta = RuleMeta {
    id: "R09",
    name: "PARAGRAPH_LEAD_CONJUNCTION",
    title: "段落頭の接続詞",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "段落の頭が「しかし」「また」などの接続詞で始まる段落が 3 割以上ある",
    explanation: CONJ_EXPLANATION,
};

const CONJ_EXPLANATION: &str = "\
### 何を見るか

地の文の段落が 3 つ以上ある文書で、段落の先頭が接続詞 (しかし・また・そして・そのため・さらに・\
つまり・一方・一方で・このように・なぜなら・したがって・ただし) で始まる段落が 3 割以上あるとき、\
該当する段落を指します。「またたく間に」「一方的に」のように、別の語の一部になっているものは数えません。

### なぜ問題か

段落の頭を接続詞でそろえると、段落どうしの関係を内容で示さず、接続詞に預けたままになりがちです。\
「また」「さらに」を重ねると、並列の話題を水増ししているようにも見えます。

### 直し方

前の段落の内容を受ける一文から入れば、接続詞がなくても流れが追えることが多いです。逆接や条件の\
ように論理に欠かせない接続詞は残してください。

### 例

- 直す前: 「また、夜間の対応も見直した。」で始まる段落が続く。
- 直した後: 「夜間の対応は、一次受付を外部に任せる形に変えた。」と、話題そのものから段落を始める。

### 根拠

実験的です。段落頭の接続詞の比率は、人の文書の誤検知 3.6% に対して生成文書の検出が 6% と、ほとんど\
差が出ませんでした。そのため既定では動かしません。
";

/// 段落頭で数える接続詞 (長いものから照合する)。
const CONJUNCTIONS: &[&str] = &[
    "したがって",
    "このように",
    "一方では",
    "そのため",
    "なぜなら",
    "一方で",
    "しかし",
    "そして",
    "さらに",
    "つまり",
    "ただし",
    "また",
    "一方",
];

/// 接続詞に見えた文字列が、実は別の語の一部か。
///
/// ひらがなで終わる接続詞は直後がひらがななら別の語 (「またたく」「しかしながら」)。
/// 漢字で終わる接続詞は直後が文字なら複合語 (「一方的」「一方通行」「一方は」)。
fn continues_word(conj: &str, after: &str) -> bool {
    let Some(next) = after.chars().next() else {
        return false;
    };
    if conj.chars().next_back().is_some_and(text::is_kanji) {
        text::is_japanese(next) || text::is_alnum(next)
    } else {
        text::is_hiragana(next)
    }
}

/// 段落の先頭の接続詞と、その解析用テキスト上の範囲。
fn leading_conjunction(text: &str) -> Option<(&'static str, std::ops::Range<usize>)> {
    let start = text.len() - text.trim_start().len();
    let rest = &text[start..];
    for conj in CONJUNCTIONS {
        if let Some(after) = rest.strip_prefix(conj) {
            if continues_word(conj, after) {
                continue;
            }
            return Some((conj, start..start + conj.len()));
        }
    }
    None
}

/// R09 PARAGRAPH_LEAD_CONJUNCTION。
pub struct ParagraphLeadConjunction {
    min_paragraphs: usize,
    ratio_threshold: f64,
}

impl ParagraphLeadConjunction {
    pub fn new(_genre: Genre) -> Self {
        Self {
            min_paragraphs: 3,
            ratio_threshold: 0.3,
        }
    }

    /// 接続詞で始まる段落 (接続詞, 原文上の範囲, 段落) と、地の文の段落の数。
    /// 段落が `min_paragraphs` に満たなければ `None`。
    #[allow(clippy::type_complexity)]
    fn lead_hits<'a>(
        &self,
        ctx: &RuleContext<'a>,
    ) -> Option<(Vec<(&'static str, Span, &'a Block)>, usize)> {
        let paragraphs = prose_paragraphs(ctx);
        if paragraphs.len() < self.min_paragraphs {
            return None;
        }
        let hits = paragraphs
            .iter()
            .filter_map(|(_, b)| {
                leading_conjunction(&b.text).map(|(conj, range)| (conj, b.to_source(range), *b))
            })
            .collect();
        Some((hits, paragraphs.len()))
    }
}

impl Rule for ParagraphLeadConjunction {
    fn meta(&self) -> &'static RuleMeta {
        &CONJ_META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_paragraphs" => self.min_paragraphs = option_count(key, value)?,
            "ratio_threshold" => self.ratio_threshold = option_f64_in(key, value, 0.0, 1.0)?,
            _ => return Err(unknown_option(&CONJ_META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_paragraphs", self.min_paragraphs.to_string()),
            ("ratio_threshold", self.ratio_threshold.to_string()),
        ]
    }

    /// 地の文の段落が `min_paragraphs` 以上あり、接続詞で始まる段落が 1 つ以上ある文書で、
    /// その比率を `ratio_threshold` と比べる値として返す (これ以上で指摘する)。
    /// 接続詞で始まる段落がない文書は、閾値をどう変えても指摘しないので値を返さない。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        match self.lead_hits(ctx) {
            Some((hits, paragraphs)) if !hits.is_empty() => vec![Measure::new(
                "conjunction_ratio",
                hits.len() as f64 / paragraphs as f64,
                "ratio_threshold",
                Fires::AtOrAbove,
            )],
            _ => Vec::new(),
        }
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let Some((hits, paragraphs)) = self.lead_hits(ctx) else {
            return;
        };
        let ratio = hits.len() as f64 / paragraphs as f64;
        if hits.is_empty() || ratio < self.ratio_threshold {
            return;
        }
        let related: Vec<Span> = hits.iter().map(|(_, span, _)| *span).collect();
        for (conj, span, block) in &hits {
            let context = ctx.doc.sentences[block.sentences.clone()]
                .first()
                .map_or(block.span, |s| s.span);
            out.push(
                CONJ_META
                    .diagnostic(
                        *span,
                        format!(
                            "段落が接続詞「{conj}」で始まっています (地の文の段落 {} 個のうち {} 個、{:.0}%)",
                            paragraphs,
                            hits.len(),
                            ratio * 100.0
                        ),
                    )
                    .with_hint(
                        "前の段落の内容を受ける一文から入れないか考え、接続詞がなくても流れが\
                         追えるなら削ってください",
                    )
                    .with_context(context)
                    .with_related(related.clone())
                    .with_metric("paragraphs", paragraphs)
                    .with_metric("conjunction_paragraphs", hits.len())
                    .with_metric("ratio", ratio),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{matched, run};

    #[test]
    fn r07_flags_paragraphs_with_the_same_sentence_count() {
        let para = "最初の文を書いた。次の文を書いた。最後の文を書いた。\n\n";
        let md = para.repeat(4);
        let d = run(&UniformParagraphs::new(Genre::General), &md);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, "R07");
        assert!(d[0].message.contains("文数 3, 3, 3, 3"));
        assert_eq!(d[0].related.len(), 4);
    }

    #[test]
    fn r07_stays_quiet_when_counts_vary_or_paragraphs_are_few() {
        let md = "一文だけの段落。\n\n一つ目。二つ目。三つ目。四つ目。五つ目。\n\n二文の段落。もう一文。\n\n一つ目。二つ目。三つ目。四つ目。五つ目。六つ目。\n";
        assert!(run(&UniformParagraphs::new(Genre::General), md).is_empty());
        let few = "一文。二文。\n\n一文。二文。\n\n一文。二文。\n";
        assert!(run(&UniformParagraphs::new(Genre::General), few).is_empty());
    }

    #[test]
    fn r07_ignores_list_items() {
        let md = "段落の一文。\n\n- 項目。\n- 項目。\n- 項目。\n- 項目。\n";
        assert!(run(&UniformParagraphs::new(Genre::General), md).is_empty());
    }

    #[test]
    fn r07_counts_only_japanese_sentences() {
        let rule = UniformParagraphs::new(Genre::General);
        // 英語だけの文書は対象にしない
        let english = "The team reviews each change. It records the result.\n\n".repeat(4);
        assert!(run(&rule, &english).is_empty());

        // 英語の段落と英語の文は数えない (日本語の段落の文数 1, 3, 2, 4 はばらついている)
        let md = "一文だけの段落。\n\nThis paragraph is English. It has two sentences.\n\n\
                  一つ目。二つ目。三つ目。\n\nSee the log. Then retry. Report it.\n\n\
                  二文の段落。もう一文。Plus one English sentence.\n\n\
                  一つ目。二つ目。三つ目。四つ目。\n";
        assert!(
            run(&rule, md).is_empty(),
            "英語の段落を数えると 1,2,3,3,2,4 に見える"
        );

        // 日本語の段落の文数がそろっていれば、英語の段落が挟まっても指摘する
        let md = "最初の文。次の文。\n\nAn English aside. Short.\n\n".repeat(4);
        let d = run(&rule, &md);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("段落 4 個"), "{}", d[0].message);
        assert!(d[0].message.contains("文数 2, 2, 2, 2"), "{}", d[0].message);
    }

    #[test]
    fn r07_configure() {
        let mut r = UniformParagraphs::new(Genre::General);
        r.configure("min_paragraphs", &toml::Value::Integer(3))
            .unwrap();
        let few = "一文。二文。\n\n一文。二文。\n\n一文。二文。\n";
        assert_eq!(run(&r, few).len(), 1);
        assert!(r.configure("cv", &toml::Value::Float(0.1)).is_err());
    }

    #[test]
    fn r09_flags_conjunction_led_paragraphs() {
        let md = "結論から書く。\n\nしかし、例外もある。\n\nまた、費用の問題もある。\n\n最後に日程を決めた。\n";
        let d = run(&ParagraphLeadConjunction::new(Genre::General), md);
        assert_eq!(d.len(), 2);
        assert_eq!(matched(md, &d), vec!["しかし", "また"]);
        assert!(d[0].message.contains("4 個のうち 2 個"));
    }

    #[test]
    fn r09_ignores_words_that_only_start_like_conjunctions() {
        let md = "またたく間に広まった。\n\nしかしながら費用がかかる。\n\n予定どおり進めた。\n";
        assert!(run(&ParagraphLeadConjunction::new(Genre::General), md).is_empty());
    }

    #[test]
    fn r09_stays_quiet_below_the_ratio() {
        let md = "一段落目。\n\n二段落目。\n\n三段落目。\n\n四段落目。\n\nしかし、五段落目。\n";
        assert!(run(&ParagraphLeadConjunction::new(Genre::General), md).is_empty());
    }

    #[test]
    fn r09_prefers_the_longest_conjunction() {
        assert_eq!(
            leading_conjunction("一方で、費用は増えた。").unwrap().0,
            "一方で"
        );
        assert_eq!(
            leading_conjunction("一方、費用は増えた。").unwrap().0,
            "一方"
        );
        assert!(leading_conjunction("一方的に決めた。").is_none());
    }
}
