//! R20: 全文が対象のときだけ、地の文の敬体と常体の混在を見る。

use std::ops::Range;

use hasami::CoarsePos;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::morph::MorphToken;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};
use crate::text;

use super::{option_count, unknown_option};

static META: RuleMeta = RuleMeta {
    id: "R20",
    name: "MIXED_WRITING_STYLE",
    title: "敬体と常体の混在",
    lane: Lane::Readability,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "全文を検査するとき、地の文のです・ます調と言い切りが混ざっているかを見る (実験的)",
    explanation: r"### 何を見るか

全文がチェック対象の文書で、段落をまたいで敬体と常体を数えます。既定はそれぞれ2文以上あるときに1件出します。`min_each` で各文体の最小文数を変えられます。です・ますの活用形と、だ・であるなどの言い切りを見ます。辞書があれば、動詞・形容詞・助動詞で終わる文も常体に含めます。辞書がなければ、する・した・される・ある・いるなど、限られた語尾で近似します。

見出し・リスト・表・引用ブロック、括弧の内側の引用・会話・補足、注記、体言止めのラベル、コードのコメントやセルなどの独立した断片は比べません。本文中に引用があっても、引用の外側の文末を見ます。部分差分では検査自体を実行しません。新規ファイルなど、空白以外の原文がすべて対象なら、差分の入口でも実行します。

### なぜ問題か

地の文で文体が意図せず変わると、書き手の語り方が途中で変わったように見えます。一方で、節ごとの書き分けや呼びかけなど、混在が必要な場合もあります。混在を誤りや執筆方法の証拠と断定せず、読みやすさを確かめる情報として扱います。

### 直し方

示した敬体と常体の文末を見比べ、意図した書き分けでなければ本文の文体に合わせてください。どちらの文体にも一律に寄せません。必要な引用・会話・見出しを本文に合わせて直す必要はありません。

### 例

- 直す前: 設定を読み込みます。項目を確認します。処理を開始する。結果を表示する。
- 直した後: 設定を読み込みます。項目を確認します。処理を開始します。結果を表示します。

### 根拠

各文体2文以上・同梱の辞書の条件で、人の文書176本中68本、生成文書390本中36本に反応しました。技術文書では人50本中38本、生成32本中5本です。人の文章にも多く見られる混在を、執筆方法の判別には使いません。この対照には書き分けの意図のラベルがないため、反応率を誤検知率とはみなしません。意図した混在と書き誤りを判別できず、語尾の近似と最小文数を既定で有効にするための校正も終えていないため、実験的な情報の指摘として、明示的に有効にしたときだけ動きます。",
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Style {
    Polite,
    Plain,
}

/// 括弧の内側を同じバイト数の空白にする。辞書の位置を原文の文と合わせたまま、外側の文末を探す。
fn outside_brackets(value: &str) -> String {
    let mut stack = Vec::new();
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        let inside = !stack.is_empty();
        if stack.last() == Some(&c) {
            stack.pop();
        } else if let Some(close) = match c {
            '"' => Some('"'),
            '“' => Some('”'),
            '‘' => Some('’'),
            _ => text::closing_bracket(c),
        } {
            stack.push(close);
        }
        if inside || !stack.is_empty() {
            out.extend(std::iter::repeat_n(' ', c.len_utf8()));
        } else {
            out.push(c);
        }
    }
    out
}

fn classify(value: &str, tokens: Option<&[MorphToken]>) -> Option<(Style, Range<usize>)> {
    let visible = outside_brackets(value);
    let mut core = text::strip_sentence_end(&visible).trim_end();
    // 文末の終助詞は語尾の活用と分ける。推量の「かもしれない」は末尾ではないので外れない。
    while let Some(c) = core.chars().next_back() {
        if !"ねよかぞ".contains(c) {
            break;
        }
        core = core[..core.len() - c.len_utf8()].trim_end();
    }
    if core.is_empty() {
        return None;
    }
    let last = tokens.and_then(|t| t.iter().rev().find(|t| t.range.end <= core.len()));
    if last.is_some_and(|t| {
        !matches!(
            t.pos,
            CoarsePos::Verb | CoarsePos::Adjective | CoarsePos::AuxVerb
        )
    }) {
        return None;
    }
    let polite = [
        "ませんでした",
        "ございました",
        "ません",
        "でした",
        "ました",
        "でしょう",
        "ましょう",
        "ございます",
        "ください",
        "です",
        "ます",
    ];
    if let Some(ending) = polite.iter().find(|e| core.ends_with(**e)) {
        return Some((Style::Polite, core.len() - ending.len()..core.len()));
    }
    if let Some(token) = last {
        // 「行い。」「高く。」など、連用形で途切れた断片は数えない。
        let terminal = core.chars().next_back().is_some_and(|c| match token.pos {
            CoarsePos::Verb => "うくぐすつぬぶむる".contains(c),
            CoarsePos::Adjective => c == 'い',
            CoarsePos::AuxVerb => "うるすぬたいんだ".contains(c),
            _ => false,
        });
        if terminal {
            return Some((Style::Plain, token.range.start..core.len()));
        }
        return None;
    }
    let plain = [
        "であった",
        "である",
        "であろう",
        "ではなかった",
        "ではない",
        "だった",
        "だろう",
        "のだ",
        "そうだ",
        "する",
        "した",
        "される",
        "された",
        "できる",
        "できた",
        "なる",
        "なった",
        "ある",
        "あった",
        "いる",
        "いた",
    ];
    if let Some(ending) = plain.iter().find(|e| core.ends_with(**e)) {
        return Some((Style::Plain, core.len() - ending.len()..core.len()));
    }
    if let Some(prefix) = core.strip_suffix('だ')
        && prefix.chars().next_back().is_some_and(text::is_kanji)
    {
        return Some((Style::Plain, core.len() - 'だ'.len_utf8()..core.len()));
    }
    None
}

#[derive(Debug)]
pub(super) struct MixedWritingStyle {
    min_each: usize,
}

impl Default for MixedWritingStyle {
    fn default() -> Self {
        Self { min_each: 2 }
    }
}

fn endings(ctx: &RuleContext<'_>) -> (Vec<Span>, Vec<Span>) {
    let mut polite = Vec::new();
    let mut plain = Vec::new();
    for (index, sentence) in ctx.doc.sentences.iter().enumerate() {
        let block = &ctx.doc.blocks[sentence.block];
        if !sentence.japanese || !block.is_prose() {
            continue;
        }
        let value = ctx.doc.sentence_text(sentence);
        if ["※", "注:", "注：", "注記:", "注記："]
            .iter()
            .any(|prefix| block.text.trim_start().starts_with(prefix))
        {
            continue;
        }
        let tokens = ctx.morph.and_then(|m| m.sentence(index));
        let Some((style, range)) = classify(value, tokens) else {
            continue;
        };
        let span =
            block.to_source(sentence.range.start + range.start..sentence.range.start + range.end);
        match style {
            Style::Polite => polite.push(span),
            Style::Plain => plain.push(span),
        }
    }
    (polite, plain)
}

impl Rule for MixedWritingStyle {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }
    fn requires_full_document(&self) -> bool {
        true
    }
    fn uses_morphology(&self) -> bool {
        true
    }
    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_each" => self.min_each = option_count(key, value)?,
            _ => return Err(unknown_option(&META, key)),
        }
        Ok(())
    }
    fn options(&self) -> Vec<(&'static str, String)> {
        vec![("min_each", self.min_each.to_string())]
    }
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let (polite, plain) = endings(ctx);
        if polite.is_empty() && plain.is_empty() {
            return Vec::new();
        }
        vec![Measure::new(
            "minority_sentences",
            polite.len().min(plain.len()) as f64,
            "min_each",
            Fires::AtOrAbove,
        )]
    }
    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let (polite, plain) = endings(ctx);
        if polite.len() < self.min_each || plain.len() < self.min_each {
            return;
        }
        let (primary, related) = if polite.len() > plain.len()
            || (polite.len() == plain.len() && polite[0].start < plain[0].start)
        {
            (plain[0], polite[0])
        } else {
            (polite[0], plain[0])
        };
        out.push(META.diagnostic(primary, format!("地の文に敬体が{}文、常体が{}文あります。文末「{}」と「{}」を見比べ、文体の書き分けが意図したものか確かめてください", polite.len(), plain.len(), ctx.doc.slice(polite[0]), ctx.doc.slice(plain[0])))
            .with_related(vec![related])
            .with_hint("意図した書き分けなら残せます。統一する場合は本文の文体に合わせ、引用や会話まで一律に変えないでください")
            .with_metric("polite_sentences", polite.len())
            .with_metric("plain_sentences", plain.len())
            .with_metric("min_each", self.min_each));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{assert_measure_consistent, run, run_with_morphology};
    const MIXED: &str =
        "設定を読み込みます。項目を確認します。\n\n処理を開始する。結果は画面に表示される。\n";
    #[test]
    fn mixed_paragraphs_show_both_endings() {
        let rule = MixedWritingStyle::default();
        let found = run(&rule, MIXED);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].lane, Lane::Readability);
        assert_eq!(found[0].related.len(), 1);
        assert_eq!(found[0].metrics["polite_sentences"], 2usize.into());
        assert_eq!(found[0].metrics["plain_sentences"], 2usize.into());
        assert_measure_consistent(
            &rule,
            &crate::document::Document::markdown(MIXED),
            &[],
            "混在",
        );
    }
    #[test]
    fn skips_uniform_text_and_independent_quoted_styles() {
        let rule = MixedWritingStyle::default();
        for text in [
            "読み込みます。確認します。表示します。",
            "処理する。確認する。表示する。",
            "設定です。項目です。\n\n「処理する。表示する。」\n\n> 処理する。表示する。\n\n- 処理する。表示する。\n\n## 処理する。表示する。\n\n| 値 |\n| --- |\n| 処理する。表示する。 |",
            "設定です。項目です。\n\n処理（常体だ）。表示（常体である）。",
            "設定です。項目です。\n\n※ 処理する。表示する。実行する。\n\n注：表示する。実行する。確認する。",
            "設定です。項目です。\n\nまだ。ただ。設定の確認。項目の一覧。",
        ] {
            assert!(run(&rule, text).is_empty(), "{text}");
        }
        assert_eq!(run(&rule, "設定は「正常だ」です。内容は『適切だ』です。\n\n処理する（説明です）。表示する（説明です）。").len(), 1);
    }
    #[test]
    fn minimum_each_is_configurable_and_measures_agree() {
        let mut rule = MixedWritingStyle::default();
        assert!(run(&rule, "設定です。処理する。").is_empty());
        rule.configure("min_each", &toml::Value::Integer(1))
            .unwrap();
        assert_eq!(run(&rule, "設定です。処理する。").len(), 1);
        assert_measure_consistent(
            &rule,
            &crate::document::Document::markdown("設定です。処理する。"),
            &[],
            "各1文",
        );
        assert!(
            rule.configure("min_each", &toml::Value::Integer(0))
                .is_err()
        );
    }
    #[test]
    fn dictionary_distinguishes_nouns_and_plain_verbs() {
        let rule = MixedWritingStyle::default();
        let morph = crate::morph::testing::morphology(&[
            ("読む", "動詞,自立,*,*"),
            ("書く", "動詞,自立,*,*"),
            ("です", "助動詞,*,*,*"),
            ("はだ", "名詞,一般,*,*"),
            ("ただ", "副詞,一般,*,*"),
            ("し", "動詞,自立,*,*"),
            ("行い", "動詞,自立,*,*"),
            ("高く", "形容詞,自立,*,*"),
            ("高い", "形容詞,自立,*,*"),
            ("低い", "形容詞,自立,*,*"),
        ]);
        assert_eq!(
            run_with_morphology(&rule, "設定です。項目です。読む。書く。", &morph).len(),
            1
        );
        assert_eq!(
            run_with_morphology(&rule, "設定です。項目です。高い。低い。", &morph).len(),
            1
        );
        assert!(
            run_with_morphology(
                &rule,
                "設定です。項目です。はだ。ただ。し。し。行い。高く。",
                &morph
            )
            .is_empty()
        );
    }
}
