//! R21: 読点で区切った短い宣言の反復を、見出しと独立した短い段落で数える。

use crate::diagnostic::{Diagnostic, Lane, Severity, Signal, Span};
use crate::document::DocumentKind;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};
use crate::text;

use super::{option_count, unknown_option};

static META: RuleMeta = RuleMeta {
    id: "R21",
    name: "REPEATED_SHORT_SLOGAN",
    title: "読点で区切った短い決め台詞の反復",
    lane: Lane::Slop,
    signal: Some(Signal::Moderate),
    default_severity: Severity::Info,
    summary: "読点で区切った短い見出し・一文段落の型が、文書内で繰り返される箇所を指摘する",
    explanation: r"### 何を見るか

見出しと地の文の一文段落を対象に、文末記号と空白を除いて 30 字以内で、括弧の外に読点「、」「，」がちょうど一つある行を数えます。前後とも日本語を含み、前半が「は」「を」「に」「も」か動詞らしい非過去の末尾で終わり、後半が 2〜15 字で、名詞らしい末尾か普通体の非過去の動詞・「ない」で終わることも条件です。候補が 3 ブロック以上 (min_count で変更可能。最小 2) に現れるとき、各箇所を指します。単発では指摘しません。

二つの品物名を並べるだけの行や、後半が「という」「といった」などの説明句で始まる行、手順の導入（「次に」「まず」など）は除きます。因果・条件・逆接を表す末尾 (「ので」「から」「ため」「なら」など) や、連用形らしい末尾 (「し」「き」「め」など) の前半は除きます。後半が過去形・丁寧体・疑問形のもの、算用数字・コードや URL のプレースホルダを含むもの、括弧の対応が取れないものも除きます。リスト・表・引用・脚注・複数文の段落・独立した断片は数えません。辞書は使わず字面で推定するため、因果関係や品詞を正確に判定するものではありません。

### なぜ問題か

短い行を読点で二つに割る型が続くと、具体的な出来事や仕組みを説明する代わりに、見出しや節の結びが決め台詞に寄ることがあります。人も使う書き方なので、型の反復だけで書き手や執筆方法は判断できません。

### 直し方

見出しには、やったこと・起きたことをそのまま書けるか確認してください。結びの一行は、数字や具体的な事実を伝える役割がある場合に残します。読点を消すだけの修正や、すべての見出しを同じ書き方に変える修正は不要です。

### 例

- 直す前: 各節で「点検を、仕組みで続ける」「小さな記録が示す、次の一手」「待っている間に、機会は失われる」と締める。
- 直した後: 点検を自動で実行する時刻や、記録を読んで決める手順を、それぞれの節に書く。

### 根拠

技術文書の人 50 本と生成 32 本で、反復回数 2〜5 を比較しました。既定の 3 回では、人 1 本（2.0%、片側 95% 上限 8.5%）と生成 0 本に一致しました。取り分けた保留側では人 19 本中 1 本（片側 95% 上限 20.5%）、生成 6 本中 0 本です。条件の見直しに同じ 50 本を使っているため、保留分も独立した検証ではありません。AI の文章を見分ける根拠にはなっていません。30 字と 3 ブロックは暫定値のままとし、スライドの同条件の対照も足りないため、情報レベルで型を見直す候補として扱います。

人の文書 440 本と生成文書 390 本では、指摘が出た文書が人 3 本・生成 1 本と少なく強さを測れないため、手掛かりの強さは中程度としています。",
};

/// 括弧内の読点を数えず、前後を分ける読点を一つだけ返す。
fn comma(value: &str) -> Option<usize> {
    let mut brackets = Vec::new();
    let mut at = None;
    for (i, c) in value.char_indices() {
        if c == text::PLACEHOLDER
            || c.is_ascii_digit()
            || ('０'..='９').contains(&c)
            || matches!(c, '?' | '？')
        {
            return None;
        }
        if let Some(close) = text::closing_bracket(c) {
            brackets.push(close);
        } else if text::is_closing_bracket(c) {
            if brackets.pop() != Some(c) {
                return None;
            }
        } else if matches!(c, '、' | '，') && brackets.is_empty() && at.replace(i).is_some() {
            return None;
        }
    }
    brackets.is_empty().then_some(at).flatten()
}

fn candidate(value: &str) -> bool {
    if text::reading_length(value) > 30 {
        return false;
    }
    let Some(at) = comma(value) else {
        return false;
    };
    let head = value[..at].trim_end();
    let tail = text::strip_sentence_end(&value[at + '、'.len_utf8()..]).trim();
    if !text::contains_japanese(head)
        || !text::contains_japanese(tail)
        || !(2..=15).contains(&text::reading_length(tail))
    {
        return false;
    }
    let joins = [
        "ので",
        "から",
        "ため",
        "ために",
        "ば",
        "たら",
        "なら",
        "ても",
        "でも",
        "のに",
        "けど",
        "が",
        "と",
        "場合",
        "場合に",
        "とき",
        "ときに",
        "際",
        "際に",
        "前",
        "前に",
        "後",
        "後に",
        "には",
        "ように",
        "たびに",
        "までに",
        "うちに",
        "ずに",
        "つつ",
    ];
    if joins.iter().any(|s| head.ends_with(s))
        || head.ends_with([
            'し', 'き', 'ぎ', 'み', 'び', 'り', 'ち', 'え', 'め', 'て', 'で',
        ])
        || tail.ends_with(['た', 'だ', 'か'])
        || ["です", "ます", "でしょう", "ましょう"]
            .iter()
            .any(|s| tail.ends_with(s))
    {
        return false;
    }
    // 二つの品物名や、引用を言い換える説明は短い宣言の型と分ける。
    let head_ends_as_clause = head.ends_with([
        'は', 'を', 'に', 'も', 'る', 'う', 'ぐ', 'す', 'つ', 'ぬ', 'む', 'ぶ',
    ]) || head.ends_with("ない")
        || head.ends_with("いく");
    if !head_ends_as_clause
        || [
            "次に",
            "続いて",
            "その後",
            "まず",
            "最後に",
            "はじめに",
            "例えば",
            "たとえば",
            "具体的には",
        ]
        .contains(&head)
        || ["という", "といった", "との", "って"]
            .iter()
            .any(|s| tail.starts_with(s))
    {
        return false;
    }
    text::is_nominal_ending(tail)
        || tail.ends_with(['る', 'う', 'く', 'ぐ', 'す', 'つ', 'ぬ', 'む', 'ぶ'])
        || tail.ends_with("ない")
}

fn candidates(ctx: &RuleContext<'_>) -> Vec<(Span, Span)> {
    if ctx.doc.kind != DocumentKind::Prose {
        return Vec::new();
    }
    ctx.doc
        .blocks
        .iter()
        .filter_map(|b| {
            if b.in_quote
                || b.in_footnote
                || !(b.is_heading() || b.is_prose())
                || b.sentences.len() != 1
            {
                return None;
            }
            let s = &ctx.doc.sentences[b.sentences.start];
            candidate(ctx.doc.sentence_text(s)).then_some((s.span, b.span))
        })
        .collect()
}

pub struct RepeatedShortSlogan {
    min_count: usize,
}

impl Default for RepeatedShortSlogan {
    fn default() -> Self {
        Self { min_count: 3 }
    }
}

impl Rule for RepeatedShortSlogan {
    fn meta(&self) -> &'static RuleMeta {
        &META
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
        let n = candidates(ctx).len();
        if n == 0 {
            Vec::new()
        } else {
            vec![Measure::new(
                "slogan_blocks",
                n as f64,
                "min_count",
                Fires::AtOrAbove,
            )]
        }
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let found = candidates(ctx);
        if found.len() < self.min_count {
            return;
        }
        for &(span, context) in &found {
            out.push(META.diagnostic(span, format!("読点で区切った短い見出し・段落の型が {} 箇所で繰り返されています", found.len()))
                .with_context(context)
                .with_hint("見出しには、やったこと・起きたことをそのまま書けるか確認してください。結びの一行は、具体的な事実を伝える役割があれば残します")
                .with_metric("slogan_blocks", found.len()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{matched, run};

    const SAMPLE: &str = "## 点検を、仕組みで続ける\n\n小さな記録が示す、次の一手\n\n待っている間に、機会は失われる\n";

    #[test]
    fn points_to_repeated_headings_and_short_paragraphs() {
        let d = run(&RepeatedShortSlogan::default(), SAMPLE);
        assert_eq!(
            matched(SAMPLE, &d),
            [
                "点検を、仕組みで続ける",
                "小さな記録が示す、次の一手",
                "待っている間に、機会は失われる"
            ]
        );
        assert!(d.iter().all(|d| d.severity == Severity::Info));
        assert!(run(&RepeatedShortSlogan::default(), "点検を、仕組みで続ける\n").is_empty());
        let html = SAMPLE.replace(
            "小さな記録が示す、次の一手",
            "<p>小さな記録が示す、次の一手</p>",
        );
        let d = run(&RepeatedShortSlogan::default(), &html);
        assert_eq!(d.len(), 3);
        assert_eq!(&html[d[1].span.range()], "小さな記録が示す、次の一手");
    }

    #[test]
    fn ignores_causal_conditional_procedural_past_and_polite_explanations() {
        for value in [
            "設定が足りないので、処理を停止する。",
            "設定が足りないので、処理を停止します。",
            "ログを確認して、原因を記録した。",
            "入力を確かめ、処理を開始する。",
            "部品を準備して、取り付ける。",
            "設定を変える前に、手順を読む。",
            "次に、処理を開始する。",
            "使うには、次を実行する。",
            "点検を、毎日続けましょう。",
            "交換するたびに、点検を実行する。",
            "確認しつつ、項目を記録する。",
            "鉛筆、消しゴム",
            "発送者、受取人",
            "返り値を確認する、という手順の説明。",
            "動作が遅く、といった状態を知らせる。",
            "値が足りなければ、処理を止める。",
            "設定を変えたら、処理を再開する。",
            "確認する前に、手順を読みます。",
            "記録があっても、原因は分からない。",
            "作業の結果は、担当者が記録した。",
            "点検を、毎日続けます。",
            "点検は、毎日行うのか？",
            "開始は9時で、終了は10時です。",
        ] {
            let md = format!("{value}\n\n").repeat(3);
            assert!(run(&RepeatedShortSlogan::default(), &md).is_empty(), "{md}");
        }
    }

    #[test]
    fn counts_only_single_sentence_headings_and_prose() {
        for md in [
            "- 点検を、仕組みで続ける\n".repeat(3),
            "> 点検を、仕組みで続ける\n>\n".repeat(3),
            "| 点検を、仕組みで続ける |\n| --- |\n| 点検を、仕組みで続ける |\n| 点検を、仕組みで続ける |\n".into(),
            format!("```text\n{SAMPLE}\n```\n"),
            "[^a]: 点検を、仕組みで続ける\n\n[^b]: 小さな記録が示す、次の一手\n\n[^c]: 待っている間に、機会は失われる\n".into(),
            "点検を、仕組みで続ける。日程も変える。\n\n".repeat(3),
            "点検の結果を毎回残す仕組みを今月から全員で続けていくために、記録の置き場所をそろえる。\n\n".repeat(3),
            "点検を、`command`で続ける\n\n".repeat(3),
        ] {
            assert!(run(&RepeatedShortSlogan::default(), &md).is_empty(), "{md}");
        }
    }

    #[test]
    fn handles_comma_and_bracket_boundaries() {
        for value in [
            "「点検を、毎日続ける」と引用した。",
            "点検を、毎日、確実に続ける",
            "「点検を、仕組みで続ける",
            "点検を、仕組みで続ける」",
            "Check first, then continue.",
        ] {
            assert!(!candidate(value), "{value}");
        }
        assert!(candidate("点検を，仕組みで続ける"));
        assert!(candidate("「赤、青」を、手掛かりにする"));
    }

    #[test]
    fn length_boundary_ignores_the_final_period_and_whitespace() {
        let at_limit = format!("{}点検を、仕組みで続ける", "定".repeat(19));
        assert_eq!(text::reading_length(&at_limit), 30);
        for ending in ["", "。", "。 "] {
            assert!(candidate(&format!("{at_limit}{ending}")));
            assert!(!candidate(&format!("定{at_limit}{ending}")));
        }
    }

    #[test]
    fn validates_and_applies_the_repetition_threshold() {
        let md = "点検を、仕組みで続ける\n\n小さな記録が示す、次の一手\n";
        let mut rule = RepeatedShortSlogan::default();
        assert!(run(&rule, md).is_empty());
        rule.configure("min_count", &toml::Value::Integer(2))
            .unwrap();
        assert_eq!(run(&rule, md).len(), 2);
        assert!(
            rule.configure("min_count", &toml::Value::Integer(1))
                .is_err()
        );
        assert!(
            rule.configure("min_count", &toml::Value::Float(2.5))
                .is_err()
        );
        assert!(rule.configure("unknown", &toml::Value::Integer(3)).is_err());
    }
}
