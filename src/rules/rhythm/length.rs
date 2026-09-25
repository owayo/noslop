//! R03 LONG_SENTENCE: 一文が長すぎる。

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::document::{Document, Sentence};
use crate::genre::Genre;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta, RuleUnit};
use crate::segment;
use crate::text::reading_length;

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
括弧の中に句点 (。！？ など) がある文は、括弧の中の句点でも区切り、区切った断片のうち最も長いものの\
字数で判定します。指す範囲もその断片です。

### なぜ問題か

主語・条件・理由・結論を一文に詰め込むと、読み手は文末の述語にたどり着くまで構造全体を覚えて\
おかなければなりません。一方で、長い文そのものは AI らしさの証拠になりません (人の書いた良い\
文章にも長い文はあります)。このルールは AI 臭さではなく読みやすさの指摘として扱います。

### 直し方

一文に一つの主張だけが入っているかを確かめ、意味の切れ目で分けてください。分けたことで字数が\
増えるのはかまいません。字数を減らすこと自体を目的にしないでください。

### 例

- 直す前: 「申請が差し戻された場合は理由欄を確認したうえで必要な書類を添付し直してから再申請して\
いただく必要がありますが、期限を過ぎた申請は受け付けられないため注意してください。」
- 直した後: 「申請が差し戻されたら、まず理由欄を確認してください。必要な書類を添付し直してから\
再申請します。期限を過ぎた申請は受け付けられません。」

### 根拠

読みやすさの指摘として校正しています。生成文書約 1 万文の文長の分布で、90 字を超える文は\
上位 8% ほどでした。目で確かめると、130 字を超える文は分けたほうがほぼ例外なく読みやすく、90 字台から \
100 字を少し超えるあたりは、分けるべきかどうかの判断が分かれました。指摘は見直しのきっかけにすぎず、\
読んで問題がなければ直さずに済むため 90 字にしています。エッセイは\
長い一文が書き手の呼吸であることが多く、人の文書での反応が目立ったため 110 字に緩めています。\
長文は生成文書を見分ける手掛かりにはなりませんでした (検出率 1%)。

元の校正は括弧の中でも文を切る数え方だったので、それに合わせて括弧の中の句点でも区切って数えます。\
会話の 1 行が「……。……。……。」と続くとき、中の一つ一つの文は短いのに、全体を 1 つの長文と数えない\
ためです。その代わり、外側の文の途中に長い引用が挟まる文も断片ごとに数えるので、引用の前後に\
またがる外側の構文の長さは測りません。
";

/// R03 LONG_SENTENCE。
pub struct LongSentence {
    /// この字数を超える断片 (括弧の中の句点でも区切った文の一部) があれば指す。
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
    fn unit(&self) -> RuleUnit {
        RuleUnit::Sentence
    }

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

    /// 対象のブロックの日本語の文を断片に分け、最も長い断片の字数を `max_chars` と比べる値として
    /// 返す (これを超える断片があれば指摘する)。断片の数え方は `check` と同じ (`longest_fragment`)。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        judged_sentences(ctx)
            .map(|(_, longest)| longest.fragment.length)
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
        for (s, Longest { fragment, divided }) in judged_sentences(ctx) {
            if fragment.length <= self.max_chars {
                continue;
            }
            let mut message = format!(
                "一文が {} 字あります (目安 {} 字)",
                fragment.length, self.max_chars
            );
            if divided {
                message.push_str("。括弧の中の句点で区切った断片のうち、最も長いものの字数です");
            }
            out.push(
                META.diagnostic(fragment.span, message)
                    .with_hint(
                        "一文に一つの主張になっているか確かめ、意味の切れ目で文を分けてください \
                         (分けた結果、字数が増えるのはかまいません)",
                    )
                    .with_context(s.span)
                    .with_metric("length", fragment.length)
                    .with_metric("max_chars", self.max_chars),
            );
        }
    }
}

/// 長さを測る断片。R03 だけの数え方で、文分割 (`Document` の文) そのものは変えない。
#[derive(Debug, Clone, Copy)]
struct Fragment {
    /// 原文上の範囲。
    span: Span,
    /// 読み手が読む文字数の近似 ([`reading_length`])。
    length: usize,
}

/// 文の断片のうち、最も長いもの。
#[derive(Debug, Clone, Copy)]
struct Longest {
    fragment: Fragment,
    /// 括弧の中の句点で、文を 2 つ以上の断片に区切ったか。
    divided: bool,
}

/// 判定の対象の文 (対象のブロックにある、日本語を含む文) と、その文の最も長い断片。
///
/// `check` と `measure` はどちらもこれを使い、同じ断片の数え方で指摘と測定値をそろえる。
fn judged_sentences<'a>(
    ctx: &'a RuleContext<'a>,
) -> impl Iterator<Item = (&'a Sentence, Longest)> + 'a {
    ctx.scoped_blocks()
        .flat_map(|(idx, _)| ctx.doc.block_sentences(idx))
        .filter(|s| s.japanese)
        .map(|s| (s, longest_fragment(ctx.doc, s)))
}

/// 文を断片に分け、最も長い断片 (同じ字数なら前のもの) を返す。
///
/// 括弧の中に文末記号がある文 (`embedded_enders`) は、文分割が括弧ごと 1 文にしているので、
/// 括弧の中の文末記号でも区切る ([`segment::fragments`])。文末記号の連なりと、それに隙間なく続く
/// 閉じ括弧までを前の断片に含め、前後の空白は断片から除く。どの文末記号が文末として働くかは文分割と
/// 同じ判定なので、URL の `?id=1`、例外表の語 (`Yahoo!` など)、小数点 (`１．５`) では区切らない。
/// そうでない文は、文全体を 1 つの断片とする。
///
/// 断片は文のテキストだけから求める。解析用テキストには改行の字が残らず、改行で文を区切る設定でも
/// 括弧の外側の改行はすでに文の境界になっている (括弧の内側の改行では断片も区切らない) ので、改行の
/// 位置を渡さなくても同じ断片になる。
fn longest_fragment(doc: &Document, s: &Sentence) -> Longest {
    let whole = Longest {
        fragment: Fragment {
            span: s.span,
            length: s.length,
        },
        divided: false,
    };
    if !s.embedded_enders {
        return whole;
    }
    let block = &doc.blocks[s.block];
    let text = doc.sentence_text(s);
    let ranges = segment::fragments(text);
    let divided = ranges.len() > 1;
    ranges
        .into_iter()
        .map(|r| Fragment {
            // 断片の範囲は文のテキスト上の位置なので、文の始まりを足してブロック上の位置に直す
            span: block.to_source((s.range.start + r.start)..(s.range.start + r.end)),
            length: reading_length(&text[r]),
        })
        .reduce(|longest, f| {
            if f.length > longest.length {
                f
            } else {
                longest
            }
        })
        .map_or(whole, |fragment| Longest { fragment, divided })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Metric;
    use crate::rules::Scope;
    use crate::rules::testing::{
        Options, assert_measures_agree, matched, measure_md, run, run_with,
    };

    /// 会話の 1 行。中の文はどれも 40 字前後だが、括弧ごと 1 文にすると 120 字を超える。
    const DIALOGUE: &str = "「駅の北口に朝七時に集まって点呼を取り、そこから皆で登山口までバスで向かいます。\
        雨が強く降りそうな場合は、前日の夜八時までに中止かどうかの連絡を全員に回します。\
        お弁当と水筒と雨具、それに着替えの靴下は、必ず各自で用意してリュックに入れてきてください。」\n";

    /// 括弧の中に置く、90 字を超える文 (94 字)。
    const LONG_QUOTED: &str = "来月から始まる新しい当番の表は、各部署の希望を聞いたうえで総務の担当者が一度まとめ、\
        そのあと部長会で内容を確認してから全員にメールで配る予定なので、それまでは今の表のとおりに動いてください。";

    /// 短い引用の閉じ括弧の直後から続く、90 字を超える外側の文 (93 字)。
    const LONG_AFTER_QUOTE: &str = "と言ってから、机の上に広げていた会議の資料を一枚ずつ順番にまとめ直し、\
        次の会議で使う分だけを鞄に入れて、残りは明日の朝に担当者へ渡せるよう棚の上の箱にしまってから、\
        電気を消して部屋を出た。";

    /// 括弧はあるが、括弧の中に句点がない長い文 (94 字)。
    const LONG_WITHOUT_ENDERS: &str = "担当者は「来週の点検までに古い部品をすべて交換しておくこと」という指示を受けて、\
        倉庫にある在庫の数を数え直し、足りない部品を業者に発注したうえで、交換の手順書も新しい版に書き直して共有した。";

    fn sentence(chars: usize) -> String {
        format!("{}。", "長".repeat(chars))
    }

    /// 括弧の中の 2 文目が長い文を、装飾のある短い文の後に置いた段落
    /// (解析用テキストと原文で位置がずれ、文の始まりも 0 でない)。
    fn long_inside_quote() -> String {
        format!("**朝礼**で課長が話した。彼は「よく聞いてほしい。{LONG_QUOTED}」と言った。\n")
    }

    fn long_after_quote() -> String {
        format!("彼は「今日はここまで。」{LONG_AFTER_QUOTE}\n")
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
    fn dialogue_of_short_sentences_is_not_long() {
        // 文分割は括弧ごと 1 文にするので、文としては 120 字を超える
        let doc = Document::markdown(DIALOGUE);
        assert_eq!(doc.sentences.len(), 1);
        assert!(doc.sentences[0].embedded_enders);
        assert!(doc.sentences[0].length > 120, "{}", doc.sentences[0].length);
        // 括弧の中の句点で区切ると、最も長い断片 (3 文目と閉じ括弧) でも 46 字
        assert!(run(&LongSentence::new(Genre::General), DIALOGUE).is_empty());
        let m = measure_md(&LongSentence::new(Genre::General), DIALOGUE);
        assert_eq!(m.len(), 1);
        assert_eq!((m[0].threshold_key, m[0].value), ("max_chars", 46.0));
    }

    #[test]
    fn flags_the_long_fragment_inside_brackets() {
        let md = long_inside_quote();
        let d = run(&LongSentence::new(Genre::General), &md);
        assert_eq!(d.len(), 1);
        // 指すのは括弧の中の長い文 (文末記号と直後の閉じ括弧まで)。文脈は文全体
        assert_eq!(matched(&md, &d), vec![format!("{LONG_QUOTED}」")]);
        assert_eq!(
            &md[d[0].context.unwrap().range()],
            format!("彼は「よく聞いてほしい。{LONG_QUOTED}」と言った。")
        );
        assert_eq!(d[0].metrics.get("length"), Some(&Metric::Int(96)));
        assert!(d[0].message.contains("一文が 96 字"), "{}", d[0].message);
        assert!(
            d[0].message
                .contains("括弧の中の句点で区切った断片のうち、最も長いものの字数です"),
            "{}",
            d[0].message
        );
    }

    #[test]
    fn brackets_without_enders_count_the_whole_sentence() {
        let md = format!("{LONG_WITHOUT_ENDERS}\n");
        let d = run(&LongSentence::new(Genre::General), &md);
        assert_eq!(d.len(), 1);
        assert_eq!(matched(&md, &d), vec![LONG_WITHOUT_ENDERS]);
        assert_eq!(d[0].context, Some(d[0].span));
        assert_eq!(d[0].metrics.get("length"), Some(&Metric::Int(94)));
        assert!(!d[0].message.contains("括弧"), "{}", d[0].message);
    }

    #[test]
    fn flags_the_outer_sentence_after_a_short_quote() {
        let md = long_after_quote();
        let d = run(&LongSentence::new(Genre::General), &md);
        assert_eq!(d.len(), 1);
        // 閉じ括弧は前の断片に入るので、指すのはその次の字からの外側の文
        assert_eq!(matched(&md, &d), vec![LONG_AFTER_QUOTE]);
        assert_eq!(d[0].metrics.get("length"), Some(&Metric::Int(93)));
        assert_eq!(&md[d[0].context.unwrap().range()], md.trim_end());
    }

    #[test]
    fn the_rest_of_a_sentence_without_an_ender_is_a_fragment() {
        // 文末記号のない文の終わりも断片にする。断片の前の空白 (全角) は範囲に含めない
        let tail = LONG_AFTER_QUOTE.trim_end_matches('。');
        let md = format!("彼は「今日はここまで。」\u{3000}{tail}\n");
        let d = run(&LongSentence::new(Genre::General), &md);
        assert_eq!(matched(&md, &d), vec![tail]);
        assert_eq!(d[0].metrics.get("length"), Some(&Metric::Int(93)));
        assert_eq!(&md[d[0].context.unwrap().range()], md.trim_end());
    }

    #[test]
    fn a_quote_ending_the_sentence_is_one_fragment() {
        // 括弧の中の句点が閉じ括弧の直前にしかなければ、断片は文全体の 1 つ (区切ったとは言わない)。
        // 閉じ括弧の後にさらに句点が続いても、句点だけの断片は作らず前の断片に含める
        for md in [
            format!("「{LONG_QUOTED}」\n"),
            format!("「{LONG_QUOTED}」。\n"),
        ] {
            let d = run(&LongSentence::new(Genre::General), &md);
            assert_eq!(matched(&md, &d), vec![md.trim_end()]);
            assert_eq!(d[0].metrics.get("length"), Some(&Metric::Int(97)));
            assert!(!d[0].message.contains("括弧"), "{}", d[0].message);
        }
    }

    #[test]
    fn marks_that_do_not_end_sentences_do_not_divide() {
        // 小数点の「．」と例外表の語の「!」は、文分割と同じく区切らない
        // (区切ると、どちらも 90 字に届かない断片になる)
        for (tail, length) in [
            (
                "と答えてから、先月の売上が前の年の同じ月と比べて約１．５倍に伸びた理由を、\
                 店ごとの客数と客単価の推移に分けて表にまとめ、午後の会議で説明できるよう、\
                 配る資料の順番も前もって整えておいた。",
                92,
            ),
            (
                "と答えてから、今朝のYahoo!ニュースで読んだ新しい制度の記事を印刷し、\
                 要点に線を引いたうえで、午後の会議で皆に配れるよう人数分の写しを用意して、\
                 会議室の机の上にきちんとそろえておいた。",
                93,
            ),
        ] {
            let md = format!("彼女は「分かりました。」{tail}\n");
            let d = run(&LongSentence::new(Genre::General), &md);
            assert_eq!(matched(&md, &d), vec![tail]);
            assert_eq!(d[0].metrics.get("length"), Some(&Metric::Int(length)));
        }
    }

    #[test]
    fn measure_agrees_with_check_on_divided_sentences() {
        let docs: Vec<Document> = [
            DIALOGUE.to_string(),
            long_inside_quote(),
            long_after_quote(),
            format!("{LONG_WITHOUT_ENDERS}\n"),
        ]
        .into_iter()
        .map(Document::markdown)
        .collect();
        let agreement =
            assert_measures_agree(|| Box::new(LongSentence::new(Genre::General)), &docs);
        assert_eq!((agreement.fired, agreement.quiet), (3, 1));
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
