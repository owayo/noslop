//! R03 LONG_SENTENCE: 一文が長すぎる。

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::document::Sentence;
use crate::genre::Genre;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};

use super::{option_count, unknown_option};

static META: RuleMeta = RuleMeta {
    id: "R03",
    name: "LONG_SENTENCE",
    title: "長すぎる一文",
    lane: Lane::Readability,
    status: RuleStatus::Stable,
    default_severity: Severity::Info,
    summary: "一文が目安 (90 字、エッセイは 110 字) より長く、読み手に構造の保持を強いている",
    explanation: EXPLANATION,
};

const EXPLANATION: &str = "\
### 何を見るか

一文の長さ (空白と文末記号を除いた文字数) が 90 字 (エッセイは 110 字) を超える文を指します。\
括弧の中に句点がある文は、括弧ごと 1 文として数えます。

### なぜ問題か

主語・条件・理由・結論を一文に詰め込むと、読み手は文末の述語にたどり着くまで構造全体を覚えて\
おかなければなりません。一方で、長い文そのものは AI らしさの証拠になりません (人の書いた良い\
文章にも長い文はあります)。このルールは AI 臭さではなく読解負荷の指さしとして扱い、自然度\
スコアには入れません。

### 直し方

一文に一つの主張だけが入っているかを確かめ、意味の切れ目で分けてください。分けたことで字数が\
増えるのはかまいません。字数を減らすこと自体を目的にしないでください。

### 例

- 直す前: 「申請が差し戻された場合は理由欄を確認したうえで必要な書類を添付し直してから再申請して\
いただく必要がありますが、期限を過ぎた申請は受け付けられないため注意してください。」
- 直した後: 「申請が差し戻されたら、まず理由欄を確認してください。必要な書類を添付し直してから\
再申請します。期限を過ぎた申請は受け付けられません。」

### 根拠

読解負荷の指さしとして校正しています。生成文書約 1 万文の文長の分布で、90 字を超える文は\
上位 8% ほどでした。目で確かめると、130 字を超える文は分けたほうがほぼ例外なく読みやすく、90 字台から \
100 字を少し超えるあたりは、分けるべきかどうかの判断が分かれました。指さしは起点にすぎず、見て直さないだけで済むため 90 字にしています。エッセイは\
長い一文が書き手の呼吸であることが多く、人の文書での反応が目立ったため 110 字に緩めています。\
長文は生成文書を見分ける手掛かりにはなりませんでした (検出率 1%)。
";

/// R03 LONG_SENTENCE。
pub struct LongSentence {
    /// この字数を超える文を指す。
    max_chars: usize,
}

impl LongSentence {
    pub fn new(genre: Genre) -> Self {
        let max_chars = match genre {
            Genre::Essay => 110,
            Genre::General | Genre::Tech | Genre::Business => 90,
        };
        Self { max_chars }
    }
}

impl Rule for LongSentence {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "max_chars" => self.max_chars = option_count(key, value)?,
            _ => return Err(unknown_option(&META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![("max_chars", self.max_chars.to_string())]
    }

    /// 対象のブロックの日本語の文のうち、最も長い文の文字数を `max_chars` と比べる値として返す
    /// (これを超える文があれば指摘する)。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        judged_sentences(ctx)
            .map(|s| s.length)
            .max()
            .map(|n| {
                vec![Measure::new(
                    "longest_sentence",
                    n as f64,
                    "max_chars",
                    Fires::Above,
                )]
            })
            .unwrap_or_default()
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for s in judged_sentences(ctx) {
            if s.length <= self.max_chars {
                continue;
            }
            let mut message = format!(
                "一文が {} 字あります (目安 {} 字)",
                s.length, self.max_chars
            );
            if s.embedded_enders {
                message.push_str("。括弧の中の文も含めて 1 文として数えています");
            }
            out.push(
                META.diagnostic(s.span, message)
                    .with_hint(
                        "一文に一つの主張になっているか確かめ、意味の切れ目で文を分けてください \
                         (分けた結果、字数が増えるのはかまいません)",
                    )
                    .with_context(s.span)
                    .with_metric("length", s.length)
                    .with_metric("max_chars", self.max_chars),
            );
        }
    }
}

/// 判定の対象の文 (対象のブロックにある、日本語を含む文)。
fn judged_sentences<'a>(ctx: &'a RuleContext<'a>) -> impl Iterator<Item = &'a Sentence> + 'a {
    ctx.scoped_blocks()
        .flat_map(|(idx, _)| ctx.doc.block_sentences(idx))
        .filter(|s| s.japanese)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Scope;
    use crate::rules::testing::{Options, run, run_with};

    fn sentence(chars: usize) -> String {
        format!("{}。", "長".repeat(chars))
    }

    #[test]
    fn flags_sentences_over_the_limit() {
        let md = format!("{}{}\n", sentence(95), sentence(40));
        let d = run(&LongSentence::new(Genre::General), &md);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, "R03");
        assert!(d[0].message.contains("95 字"));
        assert_eq!(&md[d[0].span.range()], sentence(95));
    }

    #[test]
    fn exactly_the_limit_is_fine() {
        assert!(run(&LongSentence::new(Genre::General), &sentence(90)).is_empty());
    }

    #[test]
    fn essay_allows_longer_sentences() {
        let md = sentence(100);
        let essay = Options {
            genre: Genre::Essay,
            ..Options::default()
        };
        assert!(run_with(&LongSentence::new(Genre::Essay), &md, essay).is_empty());
        assert_eq!(run(&LongSentence::new(Genre::General), &md).len(), 1);
        assert_eq!(
            run_with(&LongSentence::new(Genre::Essay), &sentence(115), essay).len(),
            1
        );
    }

    #[test]
    fn list_items_follow_the_scope() {
        let md = format!("- {}\n", sentence(120));
        assert!(run(&LongSentence::new(Genre::General), &md).is_empty());
        let all = Options {
            scope: Scope::ALL,
            ..Options::default()
        };
        assert_eq!(
            run_with(&LongSentence::new(Genre::General), &md, all).len(),
            1
        );
    }

    #[test]
    fn ignores_non_japanese_sentences() {
        let md = format!("{}.\n", "word ".repeat(40));
        assert!(run(&LongSentence::new(Genre::General), &md).is_empty());
    }

    #[test]
    fn mentions_quoted_sentences() {
        let md = format!(
            "彼は「{}。{}。」と言った。\n",
            "言".repeat(50),
            "葉".repeat(50)
        );
        let d = run(&LongSentence::new(Genre::General), &md);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("括弧の中"));
    }

    #[test]
    fn configure_max_chars() {
        let mut r = LongSentence::new(Genre::General);
        r.configure("max_chars", &toml::Value::Integer(30)).unwrap();
        assert_eq!(run(&r, &sentence(40)).len(), 1);
        assert!(r.configure("max_chars", &toml::Value::Integer(0)).is_err());
        assert!(r.configure("other", &toml::Value::Integer(1)).is_err());
    }
}
