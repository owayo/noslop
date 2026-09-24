//! R11 SELF_ANSWER: 「〜でしょうか？ それは〜」と、自分で立てた問いに自分で答える 2 文。

use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::document::Sentence;
use crate::genre::Genre;
use crate::rules::{Rule, RuleContext, RuleMeta};
use crate::text;

use super::strip_sentence_end;

static META: RuleMeta = RuleMeta {
    id: "R11",
    name: "SELF_ANSWER",
    title: "問いと自答",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "問いの文のすぐ後に「それは」「答えは」で始まる文が続き、自分で立てた問いに自分で答えている",
    explanation: EXPLANATION,
};

const EXPLANATION: &str = "\
### 何を見るか

地の文で隣り合う 2 文のうち、前の文が問い (「？」「?」「でしょうか」「だろうか」「のか」で終わる) で、\
次の文が「それは」「答えは」「理由は」「結論は」「実は」で始まるものを指します。段落をまたいでも\
数えます。

### なぜ問題か

読み手が抱いてもいない問いを書き手が立て、すぐに自分で答える組み立ては、生成された文章に繰り返し\
現れる型です。一度なら話の導入になりますが、節ごとに出てくると、答えの前に毎回もったいぶった間が\
挟まります。

### 直し方

問いの文を消し、答えを先に書いてください。問いを残すなら、読み手が本当に抱く疑問に言い換え、\
答えの書き出しの「それは」を外します。

### 例

- 直す前: 「なぜ締め日が月末なのでしょうか？ それは、経理の集計が翌月の第 1 営業日に始まるからです。」
- 直した後: 「締め日を月末にしているのは、経理の集計が翌月の第 1 営業日に始まるからです。」

### 根拠

実験的です。生成された文章の精読で繰り返し見つかった型ですが、人の文章と生成された文章を見分ける\
力はコーパスで測っていません。
";

/// 次の文: 問いへの答えを切り出す書き出し。
static ANSWER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:それは|答えは|理由は|結論は|実は)").expect("answer regex"));

/// 問いで終わる語尾 (文末記号を除いたあと)。
const QUESTION_ENDINGS: &[&str] = &["でしょうか", "だろうか", "のか"];

/// 問いの文か (「？」「?」で終わるか、問いの語尾で終わる)。
fn is_question(sentence: &str) -> bool {
    let tail =
        sentence.trim_end_matches(|c: char| text::is_closing_bracket(c) || c.is_whitespace());
    tail.ends_with(['？', '?'])
        || QUESTION_ENDINGS
            .iter()
            .any(|e| strip_sentence_end(sentence).ends_with(e))
}

/// R11 SELF_ANSWER。
pub struct SelfAnswer;

impl SelfAnswer {
    pub fn new(_genre: Genre) -> Self {
        Self
    }
}

impl Rule for SelfAnswer {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let doc = ctx.doc;
        let sentences: Vec<&Sentence> = doc.prose_sentences().collect();
        for pair in sentences.windows(2) {
            let (first, second) = (pair[0], pair[1]);
            if !is_question(doc.sentence_text(first)) {
                continue;
            }
            let Some(lead) = ANSWER_RE.find(doc.sentence_text(second).trim_start()) else {
                continue;
            };
            out.push(
                META.diagnostic(
                    first.span,
                    format!(
                        "問いの文のすぐ後に「{}」で始まる文が続き、自分で立てた問いに自分で答えています",
                        lead.as_str()
                    ),
                )
                .with_hint("問いの文を消して答えを先に書くか、読み手が本当に抱く疑問に言い換えてください")
                .with_context(Span::new(first.span.start, second.span.end))
                .with_related(vec![second.span]),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{matched, run};

    fn rule() -> SelfAnswer {
        SelfAnswer::new(Genre::General)
    }

    #[test]
    fn flags_a_question_followed_by_its_own_answer() {
        let md = "なぜ締め日が月末なのでしょうか？それは、経理の集計が翌月の第 1 営業日に始まるからです。\n";
        let d = run(&rule(), md);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, "R11");
        assert_eq!(d[0].status, RuleStatus::Experimental);
        assert_eq!(d[0].severity, Severity::Info);
        assert_eq!(matched(md, &d), vec!["なぜ締め日が月末なのでしょうか？"]);
        assert!(d[0].message.contains("「それは」"));
        assert_eq!(d[0].related.len(), 1);
    }

    #[test]
    fn works_with_plain_questions_and_across_paragraphs() {
        let md = "本当にこの項目は必要なのか。\n\n実は、半分の項目は誰も読んでいない。\n";
        assert_eq!(run(&rule(), md).len(), 1);
        let md = "では、どう直すべきだろうか。答えは、記入例を先に見せることだ。\n";
        assert_eq!(run(&rule(), md).len(), 1);
        let md = "「締め日はいつ？」\n\n理由は、毎月変わるからだ。\n";
        assert_eq!(run(&rule(), md).len(), 1);
    }

    #[test]
    fn ignores_questions_without_self_answers() {
        let md = "なぜ遅いのか？担当者が一人しかいないからだ。\n\nそれは難しい判断だった。理由は二つある。\n";
        assert!(run(&rule(), md).is_empty());
        let list = "- なぜ遅いのか？\n- それは担当者が一人だからだ。\n";
        assert!(run(&rule(), list).is_empty());
    }

    #[test]
    fn has_no_options() {
        let mut r = rule();
        assert!(r.configure("x", &toml::Value::Integer(1)).is_err());
        assert!(r.options().is_empty());
    }
}
