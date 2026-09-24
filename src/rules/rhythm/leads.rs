//! R08 REPEATED_SENTENCE_LEAD: 同じ書き出しの文が繰り返される。

use std::collections::HashMap;
use std::ops::Range;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::document::{Document, Sentence};
use crate::genre::Genre;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};
use crate::text;

use super::{option_count, unknown_option};

static META: RuleMeta = RuleMeta {
    id: "R08",
    name: "REPEATED_SENTENCE_LEAD",
    title: "文頭の型の反復",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "同じ書き出しの文が何度も繰り返されている",
    explanation: EXPLANATION,
};

const EXPLANATION: &str = "\
### 何を見るか

地の文の文頭 (記号・括弧・インラインコードを除いた最初の数文字) を比べ、同じ書き出しが 6 回以上 \
(エッセイは 5 回、技術文書・ビジネス文書は 7 回) 繰り返されている文を指します。「また、」「そして、」\
のように読点で終わる短い書き出しはそれ自体を、それ以外は最初の 4 字を比べます。「この」「その」の\
ような 2 字の共通部分だけの一致では反応しません。

### なぜ問題か

「また、」「さらに、」で文をつなぎ続けたり、同じ主語で書き出し続けたりすると、文章が同じ入口の\
繰り返しに見えます。生成された文章は、最初の数文で選んだ組み立てをなぞり続けがちです。

### 直し方

繰り返しの 3 回目あたりで書き出しを変えてください。前の文の内容そのものでつなげる、問いかけや\
引用から入る、主語を省く、などの手があります。対句や列挙のリズムとして意図した反復なら残して\
かまいません。

### 例

- 直す前: 「また、ログの保存期間を延ばした。また、検索の上限を引き上げた。また、通知の文面を見直した。」
- 直した後: 「ログの保存期間を延ばし、検索の上限も引き上げた。通知の文面は利用者の声を受けて書き直した。」

### 根拠

実験的です。形態素 2 つ分の書き出しを数える方式でも、人の文書の方がむしろ反復が多く (エッセイで\
人の 92% に対し生成文書の 35%)、意図した反復と区別できないため、閾値を引き上げて参考情報に留めて\
いました。辞書を使わないこの近似 (先頭の文字列の比較) はまだ校正していません。製品名や技術用語で\
始まる文の反復は、技術文書では自然なことが多い点にも注意してください。
";

/// 読点で終わる書き出しとして扱う最大の字数 (読点を含む)。
const MAX_COMMA_LEAD: usize = 6;
/// 比べる書き出しの最大の字数 (表示用に共通部分を伸ばす上限)。
const MAX_LEAD_CHARS: usize = 8;

/// 文の書き出し (文の解析用テキスト上の範囲と、比較に使う文字列)。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Lead {
    range: Range<usize>,
    key: String,
    /// 読点で終わる短い書き出しか。
    comma: bool,
}

/// 文の書き出しを取り出す。記号・括弧・プレースホルダ・空白は飛ばす。
fn lead_of(sentence: &str, lead_chars: usize) -> Option<Lead> {
    let start = sentence
        .char_indices()
        .find(|&(_, c)| text::is_japanese(c) || text::is_alnum(c))
        .map(|(i, _)| i)?;
    let rest = &sentence[start..];
    let chars: Vec<(usize, char)> = rest.char_indices().collect();
    if let Some(pos) = chars
        .iter()
        .take(MAX_COMMA_LEAD)
        .position(|&(_, c)| text::is_comma(c))
        && pos >= 2
    {
        let end = chars[pos].0 + chars[pos].1.len_utf8();
        return Some(Lead {
            range: start..start + end,
            key: rest[..end].to_string(),
            comma: true,
        });
    }
    if chars.len() < lead_chars {
        return None;
    }
    let end = chars.get(lead_chars).map_or(rest.len(), |&(i, _)| i);
    Some(Lead {
        range: start..start + end,
        key: rest[..end].to_string(),
        comma: false,
    })
}

/// 文の書き出しから、最大 `MAX_LEAD_CHARS` 字の共通部分を表示用に求める。
fn common_prefix(texts: &[&str], min_chars: usize) -> String {
    let first: Vec<char> = texts[0].chars().take(MAX_LEAD_CHARS).collect();
    let mut len = first.len();
    for t in &texts[1..] {
        let common = t
            .chars()
            .zip(first.iter())
            .take_while(|(a, b)| a == *b)
            .count();
        len = len.min(common);
    }
    let len = len.max(min_chars.min(first.len()));
    let mut out: String = first[..len].iter().collect();
    // 上限で打ち切ったときは、その先も共通している可能性があるので省略記号を付ける
    if len == MAX_LEAD_CHARS && texts.iter().all(|t| t.chars().count() > MAX_LEAD_CHARS) {
        out.push('…');
    }
    out
}

/// 書き出しごとに文の番号をまとめる (最初に現れた順を保つ)。
fn group_leads(leads: &[Option<Lead>]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut slots: HashMap<&str, usize> = HashMap::new();
    for (i, lead) in leads.iter().enumerate() {
        if let Some(lead) = lead {
            let slot = *slots.entry(lead.key.as_str()).or_insert_with(|| {
                groups.push(Vec::new());
                groups.len() - 1
            });
            groups[slot].push(i);
        }
    }
    groups
}

/// R08 REPEATED_SENTENCE_LEAD。
pub struct RepeatedSentenceLead {
    /// 発火する最小の反復回数。
    min_count: usize,
    /// 読点で終わらない書き出しを比べる字数。
    lead_chars: usize,
}

impl RepeatedSentenceLead {
    pub fn new(genre: Genre) -> Self {
        let min_count = match genre {
            Genre::General => 6,
            Genre::Essay => 5,
            Genre::Tech | Genre::Business => 7,
        };
        Self {
            min_count,
            lead_chars: 4,
        }
    }

    /// 地の文の文と、それぞれの書き出し。
    fn leads<'a>(&self, doc: &'a Document) -> (Vec<&'a Sentence>, Vec<Option<Lead>>) {
        let sentences: Vec<&Sentence> = doc.prose_sentences().collect();
        let leads = sentences
            .iter()
            .map(|s| lead_of(doc.sentence_text(s), self.lead_chars))
            .collect();
        (sentences, leads)
    }
}

impl Rule for RepeatedSentenceLead {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_count" => self.min_count = option_count(key, value)?.max(2),
            "lead_chars" => {
                let n = option_count(key, value)?;
                if !(3..=MAX_LEAD_CHARS).contains(&n) {
                    return Err(format!(
                        "`{key}` には 3 以上 {MAX_LEAD_CHARS} 以下の整数を指定してください"
                    ));
                }
                self.lead_chars = n;
            }
            _ => return Err(unknown_option(&META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_count", self.min_count.to_string()),
            ("lead_chars", self.lead_chars.to_string()),
        ]
    }

    /// 同じ書き出しで始まる文の数の最大を `min_count` と比べる値として返す
    /// (これ以上の書き出しがあれば指摘する)。書き出しを取れる文がない文書では値を返さない。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let (_, leads) = self.leads(ctx.doc);
        group_leads(&leads)
            .iter()
            .map(Vec::len)
            .max()
            .map(|n| {
                vec![Measure::new(
                    "max_lead_repeats",
                    n as f64,
                    "min_count",
                    Fires::AtOrAbove,
                )]
            })
            .unwrap_or_default()
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let doc = ctx.doc;
        let (sentences, leads) = self.leads(doc);

        for members in group_leads(&leads) {
            if members.len() < self.min_count {
                continue;
            }
            let first_lead = leads[members[0]].as_ref().expect("grouped lead");
            let key = first_lead.key.as_str();
            let display = if first_lead.comma {
                key.to_string()
            } else {
                let texts: Vec<&str> = members
                    .iter()
                    .map(|&i| {
                        let lead = leads[i].as_ref().expect("grouped lead");
                        &doc.sentence_text(sentences[i])[lead.range.start..]
                    })
                    .collect();
                common_prefix(&texts, self.lead_chars)
            };
            let spans: Vec<Span> = members
                .iter()
                .map(|&i| {
                    let s = sentences[i];
                    let lead = leads[i].as_ref().expect("grouped lead");
                    doc.blocks[s.block].to_source(
                        (s.range.start + lead.range.start)..(s.range.start + lead.range.end),
                    )
                })
                .collect();
            let n = members.len();
            let tech_note = if key
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
            {
                "。製品名や技術用語による自然な反復の可能性もあります"
            } else {
                ""
            };
            for (&i, &span) in members.iter().zip(&spans) {
                out.push(
                    META.diagnostic(
                        span,
                        format!("文頭「{display}」が {n} 回繰り返されています{tech_note}"),
                    )
                    .with_hint(
                        "書き出しを変えるか、前の文の内容でつなぐ・問いかけや引用から入るなど、\
                         文の入り口を組み替えてください (意図した反復なら残してかまいません)",
                    )
                    .with_context(sentences[i].span)
                    .with_related(spans.clone())
                    .with_metric("count", n)
                    .with_metric("lead", display.clone()),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{matched, run};

    fn repeated(lead: &str, n: usize) -> String {
        (0..n)
            .map(|i| format!("{lead}{i}番目の手順を確かめた。"))
            .collect::<String>()
            + "\n"
    }

    #[test]
    fn lead_extraction() {
        let l = lead_of("また、手順を確かめた。", 4).unwrap();
        assert_eq!(l.key, "また、");
        assert!(l.comma);
        let l = lead_of("**「この機能は便利だ。", 4).unwrap();
        assert_eq!(l.key, "この機能");
        assert!(lead_of("短い。", 4).is_none());
        assert_eq!(lead_of("\u{FFFC}を実行する。", 4).unwrap().key, "を実行す");
    }

    #[test]
    fn flags_six_repeated_leads() {
        let md = repeated("また、", 6);
        let d = run(&RepeatedSentenceLead::new(Genre::General), &md);
        assert_eq!(d.len(), 6);
        assert!(d[0].message.contains("「また、」が 6 回"));
        assert_eq!(matched(&md, &d)[0], "また、");
        assert_eq!(d[0].related.len(), 6);
    }

    #[test]
    fn threshold_depends_on_genre() {
        let md = repeated("また、", 5);
        assert!(run(&RepeatedSentenceLead::new(Genre::General), &md).is_empty());
        assert_eq!(run(&RepeatedSentenceLead::new(Genre::Essay), &md).len(), 5);
        let md6 = repeated("また、", 6);
        assert!(run(&RepeatedSentenceLead::new(Genre::Tech), &md6).is_empty());
    }

    #[test]
    fn short_common_prefixes_do_not_group() {
        let md = "この機能は便利だ。この方法は速い。この点で有利だ。この画面は見やすい。この手順は短い。この設定は簡単だ。\n";
        assert!(run(&RepeatedSentenceLead::new(Genre::General), md).is_empty());
    }

    #[test]
    fn longer_shared_leads_are_shown_in_full() {
        let md = (0..6)
            .map(|i| format!("この機能は{i}回目の更新で速くなった。"))
            .collect::<String>();
        let d = run(&RepeatedSentenceLead::new(Genre::General), &md);
        assert_eq!(d.len(), 6);
        assert!(d[0].message.contains("「この機能は」"));
    }

    #[test]
    fn long_shared_leads_are_truncated_with_an_ellipsis() {
        let md = (0..6)
            .map(|i| format!("アレクサンドロス先生は{i}回目の講義でも同じ話をした。"))
            .collect::<String>();
        let d = run(&RepeatedSentenceLead::new(Genre::General), &md);
        assert_eq!(d.len(), 6);
        assert!(
            d[0].message.contains("「アレクサンドロス…」"),
            "{}",
            d[0].message
        );
    }

    #[test]
    fn technical_terms_get_a_note() {
        let md = (0..6)
            .map(|i| format!("Rustは{i}回目の比較でも速かった。"))
            .collect::<String>();
        let d = run(&RepeatedSentenceLead::new(Genre::General), &md);
        assert_eq!(d.len(), 6);
        assert!(d[0].message.contains("技術用語"));
    }

    #[test]
    fn configure_options() {
        let mut r = RepeatedSentenceLead::new(Genre::General);
        r.configure("min_count", &toml::Value::Integer(3)).unwrap();
        assert_eq!(run(&r, &repeated("そして、", 3)).len(), 3);
        assert!(
            r.configure("lead_chars", &toml::Value::Integer(20))
                .is_err()
        );
        assert!(r.configure("x", &toml::Value::Integer(3)).is_err());
    }
}
