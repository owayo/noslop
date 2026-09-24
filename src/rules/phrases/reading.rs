//! 読解負荷の指さし (P15 連続漢字、P16 「の」の連鎖、P17 二重否定)。
//!
//! このレーンは AI 臭さを測らない。読み手が一文の中で計算を強いられる箇所を指すだけで、
//! 自然度スコアには入らない。採否の基準は「指摘に従って直した文が読みやすくなるか」。

use std::ops::Range;
use std::sync::LazyLock;

use hasami::CoarsePos;
use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::morph::MorphToken;
use crate::rules::{Rule, RuleContext, RuleMeta, option_usize, quote};
use crate::text::{self, PLACEHOLDER};

use super::engine::diagnostic;

// ---------------------------------------------------------------------------
// P15 KANJI_RUN
// ---------------------------------------------------------------------------

static P15_META: RuleMeta = RuleMeta {
    id: "P15",
    name: "KANJI_RUN",
    title: "連続漢字",
    lane: Lane::Readability,
    status: RuleStatus::Stable,
    default_severity: Severity::Info,
    summary: "漢字が 7 字以上続き、語の切れ目が読み取りにくい箇所を指す",
    explanation: r"### 何を見るか

漢字が 7 字以上 (既定) 続く箇所を探します。「々」も漢字に数えます。形態素解析の辞書があれば、固有名詞を含む連なりを除きます。

### なぜ問題か

日本語は分かち書きをしないので、漢字が長く続くと語の切れ目の手掛かりがなくなり、読み手は区切りを推測しながら読むことになります。名詞を連ねて作った圧縮語は、書き手には一語でも読み手には初見の塊です。

### 直し方

語の切れ目が読み取れるか確かめ、助詞や動詞を補って開きます。

### 例

- 直す前: 顧客情報統合管理基盤移行計画を承認した。
- 直した後: 顧客情報をまとめて管理する基盤へ移る計画を承認した。

### 根拠

AI らしさの判定ではなく、読みにくさの指さしです。自然度スコアには入りません。実文書での校正で目安を 7 字以上とし、固有名詞を含む連なりは、分解しようのない名前なので除外していました。

形態素解析の辞書 (hasami) があれば、校正と同じく品詞で固有名詞を見分けて除きます。辞書がなければ固有名詞は見分けられないため、裁判所・研究所・委員会・株式会社などの定番の接尾辞で終わる (または始まる) 連なりだけを除きます。この接尾辞による除外は、辞書があっても行います。元の検出器と同じ文書で比べると、指した箇所の一致率は辞書なしで 0.69、辞書ありで 0.81 でした (辞書なしは年号や人名を含む連なりを多く指す)。長い固有名詞が指された場合は、残してかまいません。

### 設定

- `min_length`: 指す連続漢字の最小字数 (既定 7)",
};

/// 固有名詞 (機関名・法人名) によくある接尾辞。これで終わる連なりは直せない名前とみなす。
const INSTITUTION_SUFFIXES: &[&str] = &[
    "裁判所",
    "研究所",
    "研究科",
    "研究室",
    "委員会",
    "審議会",
    "協議会",
    "評議会",
    "連合会",
    "株式会社",
    "有限会社",
    "合同会社",
    "大学院",
    "事務局",
    "事務所",
];

/// 法人格 (前に付くことがある)。
const INSTITUTION_PREFIXES: &[&str] = &["株式会社", "有限会社", "合同会社"];

pub(super) struct KanjiRun {
    min_length: usize,
}

impl Default for KanjiRun {
    fn default() -> Self {
        Self { min_length: 7 }
    }
}

impl KanjiRun {
    fn find(&self, text: &str) -> Vec<(Range<usize>, usize)> {
        let mut out = Vec::new();
        let mut run: Option<(usize, usize, usize)> = None; // (start, end, chars)
        let mut flush = |run: &mut Option<(usize, usize, usize)>| {
            if let Some((s, e, n)) = run.take()
                && n >= self.min_length
            {
                let word = &text[s..e];
                let institution = INSTITUTION_SUFFIXES.iter().any(|x| word.ends_with(x))
                    || INSTITUTION_PREFIXES.iter().any(|x| word.starts_with(x));
                if !institution {
                    out.push((s..e, n));
                }
            }
        };
        for (i, c) in text.char_indices() {
            if text::is_kanji(c) {
                run = Some(match run {
                    Some((s, _, n)) => (s, i + c.len_utf8(), n + 1),
                    None => (i, i + c.len_utf8(), 1),
                });
            } else {
                flush(&mut run);
            }
        }
        flush(&mut run);
        out
    }
}

impl Rule for KanjiRun {
    fn meta(&self) -> &'static RuleMeta {
        &P15_META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_length" => {
                let v = option_usize(key, value)?;
                if v < 2 {
                    return Err("`min_length` は 2 以上にしてください".into());
                }
                self.min_length = v;
                Ok(())
            }
            _ => Err(format!("P15 に `{key}` という設定項目はありません")),
        }
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![("min_length", self.min_length.to_string())]
    }

    fn uses_morphology(&self) -> bool {
        true
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for (idx, block) in ctx.scoped_blocks() {
            let first = block.sentences.start;
            for (i, sentence) in ctx.doc.block_sentences(idx).iter().enumerate() {
                let text = &block.text[sentence.range.clone()];
                let candidates = self.find(text);
                if candidates.is_empty() {
                    continue;
                }
                // 辞書があれば、固有名詞を含む連なり (分解しようのない名前) を品詞で除く
                let tokens = ctx.morph.and_then(|m| m.sentence(first + i));
                for (range, n) in candidates {
                    if tokens.is_some_and(|t| contains_proper_noun(t, &range)) {
                        continue;
                    }
                    let matched = &text[range.clone()];
                    let abs =
                        (sentence.range.start + range.start)..(sentence.range.start + range.end);
                    let d = diagnostic(
                        &P15_META,
                        block.to_source(abs),
                        sentence.span,
                        matched,
                        format!("漢字が {n} 字続いています (「{}」)", quote(matched)),
                        "語の切れ目が読み取れるか確かめ、助詞や動詞を補って開いてください",
                        P15_META.default_severity,
                        P15_META.status,
                    )
                    .with_metric("length", n);
                    out.push(d);
                }
            }
        }
    }
}

/// `range` に重なる形態素に固有名詞があるか。
fn contains_proper_noun(tokens: &[MorphToken], range: &Range<usize>) -> bool {
    tokens.iter().any(|t| {
        t.pos == CoarsePos::ProperNoun && t.range.start < range.end && range.start < t.range.end
    })
}

// ---------------------------------------------------------------------------
// P16 NO_CHAIN
// ---------------------------------------------------------------------------

static P16_META: RuleMeta = RuleMeta {
    id: "P16",
    name: "NO_CHAIN",
    title: "「の」の連鎖",
    lane: Lane::Readability,
    status: RuleStatus::Stable,
    default_severity: Severity::Info,
    summary: "「AのBのCのD」のように「の」が 3 回以上続き、係り受けが潰れる箇所を指す",
    explanation: r"### 何を見るか

読点をはさまずに、短い語をつなぐ「の」が 3 回以上 (既定) 続く箇所を探します。形態素解析の辞書があれば、格助詞の「の」を数え、隣り合う「の」の間が 2 形態素以内のものを続いているとみなします。

### なぜ問題か

「の」は何にでも係るので、連なると、どの語がどの語を修飾しているのかが読み手に委ねられます。動作を名詞に固めた語 (「〜の導入」「〜の実現」) と組み合わさると、誰が何をするのかが見えなくなります。「の」は 2 回までを目安にします。

### 直し方

どこか 1 か所を動詞や「〜を〜する」の形に開きます。

### 例

- 直す前: 新制度の運用の開始の時期を決めた。
- 直した後: 新制度をいつから運用するかを決めた。

### 根拠

AI らしさの判定ではなく、読みにくさの指さしです。自然度スコアには入りません。元の検出器は形態素解析で格助詞の「の」だけを数え、実文書での校正で指した箇所はすべて本当の連鎖でした (再現率は低く、精度は高い)。

形態素解析の辞書 (hasami) があれば、元の検出器と同じ定義で数えます。辞書がなければ、漢字・カタカナ・英数字の語に挟まれた「の」だけを数えます。そのため「この」「その」「もの」の「の」を誤って数えることはありませんが、ひらがなを含む語をはさむ連鎖 (「の家の大きな犬の」) や、端の語がひらがなの連鎖 (「魂の安静のため」) は拾えません。元の検出器と同じ文書で比べると、元が指した箇所のうち拾えたのは、辞書なしで 39%、辞書ありで 90% でした。

### 設定

- `min_chain`: 指す「の」の最小回数 (既定 3)",
};

/// 「の」の連鎖を作る語の文字か (漢字・カタカナ・英数字・プレースホルダ)。
fn is_chain_word_char(c: char) -> bool {
    text::is_kanji(c) || text::is_katakana(c) || text::is_alnum(c) || c == PLACEHOLDER
}

/// 連鎖に数える語の最大字数 (長い塊は複数の語とみなさず連鎖を切る)。
const NO_CHAIN_MAX_WORD_CHARS: usize = 10;

/// 名詞の送り仮名として語に含めるひらがな (「見直し」「取り組み」「支払い」)。
///
/// 直後が漢字などの語の文字か「の」のときだけ送り仮名とみなす (「見直して」の「し」は含めない)。
const OKURIGANA: &[char] = &[
    'し', 'り', 'み', 'き', 'ち', 'い', 'え', 'け', 'げ', 'べ', 'び', 'め', 'れ', 'ぎ', 'じ',
];

pub(super) struct NoChain {
    min_chain: usize,
}

impl Default for NoChain {
    fn default() -> Self {
        Self { min_chain: 3 }
    }
}

impl NoChain {
    /// (範囲, 「の」の回数) を返す。
    fn find(&self, text: &str) -> Vec<(Range<usize>, usize)> {
        let chars: Vec<(usize, char)> = text.char_indices().collect();
        let n = chars.len();
        let byte_end = |k: usize| chars.get(k).map_or(text.len(), |&(p, _)| p);
        let mut out = Vec::new();
        let mut i = 0;
        while i < n {
            if !is_chain_word_char(chars[i].1) {
                i += 1;
                continue;
            }
            let start = chars[i].0;
            let mut count = 0;
            let mut j = i;
            let mut last_word_end;
            loop {
                let word_start = j;
                loop {
                    while j < n && is_chain_word_char(chars[j].1) {
                        j += 1;
                    }
                    let okurigana = j > word_start
                        && j + 1 < n
                        && OKURIGANA.contains(&chars[j].1)
                        && (is_chain_word_char(chars[j + 1].1) || chars[j + 1].1 == 'の');
                    if !okurigana {
                        break;
                    }
                    j += 1;
                    if chars[j].1 == 'の' {
                        break;
                    }
                }
                last_word_end = j;
                let word_len = j - word_start;
                // 「の」1 字をはさんで次の語が続くか
                let continues = word_len <= NO_CHAIN_MAX_WORD_CHARS
                    && j + 1 < n
                    && chars[j].1 == 'の'
                    && is_chain_word_char(chars[j + 1].1);
                if !continues {
                    break;
                }
                count += 1;
                j += 1;
            }
            if count >= self.min_chain {
                out.push((start..byte_end(last_word_end), count));
            }
            i = last_word_end.max(i + 1);
        }
        out
    }

    /// 辞書の形態素で数える (元の検出器と同じ定義)。
    ///
    /// 格助詞の「の」が `min_chain` 個以上並び、隣り合う「の」の間の形態素が 1〜2 個で、間に
    /// 句読点・括弧がない箇所を返す。範囲は、最初の「の」の前の名詞のまとまりから、最後の「の」の
    /// 後の名詞のまとまりまで。
    fn find_with_tokens(&self, text: &str, tokens: &[MorphToken]) -> Vec<(Range<usize>, usize)> {
        let nos: Vec<usize> = tokens
            .iter()
            .enumerate()
            .filter(|(_, t)| t.pos == CoarsePos::CaseParticle && &text[t.range.clone()] == "の")
            .map(|(k, _)| k)
            .collect();
        let mut out = Vec::new();
        let mut k = 0;
        while k < nos.len() {
            let mut last = k;
            while let Some(&next) = nos.get(last + 1) {
                let between = &tokens[nos[last] + 1..next];
                if between.is_empty()
                    || between.len() > NO_CHAIN_MAX_GAP
                    || between.iter().any(|t| breaks_chain(t.pos))
                {
                    break;
                }
                last += 1;
            }
            let count = last - k + 1;
            if count >= self.min_chain {
                let start = nominal_edge(tokens, nos[k], Direction::Before);
                let end = nominal_edge(tokens, nos[last], Direction::After);
                out.push((tokens[start].range.start..tokens[end].range.end, count));
            }
            k = last + 1;
        }
        out
    }
}

/// 隣り合う「の」の間に置ける形態素の最大数 (元の検出器と同じ)。
const NO_CHAIN_MAX_GAP: usize = 2;

/// 連鎖を切る品詞 (句読点・括弧)。
fn breaks_chain(pos: CoarsePos) -> bool {
    matches!(
        pos,
        CoarsePos::Period | CoarsePos::Comma | CoarsePos::OpenBracket | CoarsePos::CloseBracket
    )
}

/// 名詞のまとまりを作る品詞 (範囲を「の」の前後の語まで広げるのに使う)。
fn is_nominal_part(pos: CoarsePos) -> bool {
    matches!(
        pos,
        CoarsePos::Noun
            | CoarsePos::ProperNoun
            | CoarsePos::Pronoun
            | CoarsePos::Numeral
            | CoarsePos::NounSuffix
            | CoarsePos::FormalNoun
            | CoarsePos::Prefix
            | CoarsePos::Symbol
    )
}

#[derive(Clone, Copy)]
enum Direction {
    Before,
    After,
}

/// `at` の「の」の前 (後) にある名詞のまとまりの端の形態素 (`NO_CHAIN_MAX_GAP` 個まで)。
/// まとまりがなければ `at` を返す。
fn nominal_edge(tokens: &[MorphToken], at: usize, direction: Direction) -> usize {
    let mut edge = at;
    for _ in 0..NO_CHAIN_MAX_GAP {
        let next = match direction {
            Direction::Before => edge.checked_sub(1),
            Direction::After => Some(edge + 1).filter(|&n| n < tokens.len()),
        };
        match next {
            Some(n) if is_nominal_part(tokens[n].pos) => edge = n,
            _ => break,
        }
    }
    edge
}

impl Rule for NoChain {
    fn meta(&self) -> &'static RuleMeta {
        &P16_META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_chain" => {
                let v = option_usize(key, value)?;
                if v < 2 {
                    return Err("`min_chain` は 2 以上にしてください".into());
                }
                self.min_chain = v;
                Ok(())
            }
            _ => Err(format!("P16 に `{key}` という設定項目はありません")),
        }
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![("min_chain", self.min_chain.to_string())]
    }

    fn uses_morphology(&self) -> bool {
        true
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for (idx, block) in ctx.scoped_blocks() {
            let first = block.sentences.start;
            for (i, sentence) in ctx.doc.block_sentences(idx).iter().enumerate() {
                let text = &block.text[sentence.range.clone()];
                let found = match ctx.morph {
                    // 「の」の字が足りなければ連鎖はありえないので、解析しない
                    Some(_) if text.matches('の').count() < self.min_chain => Vec::new(),
                    Some(m) => match m.sentence(first + i) {
                        Some(tokens) => self.find_with_tokens(text, tokens),
                        None => self.find(text),
                    },
                    None => self.find(text),
                };
                for (range, count) in found {
                    let matched = &text[range.clone()];
                    let abs =
                        (sentence.range.start + range.start)..(sentence.range.start + range.end);
                    let d = diagnostic(
                        &P16_META,
                        block.to_source(abs),
                        sentence.span,
                        matched,
                        format!("「の」が {count} 回続いています (「{}」)", quote(matched)),
                        "どこか 1 か所を動詞や「〜を〜する」の形に開いてください",
                        P16_META.default_severity,
                        P16_META.status,
                    )
                    .with_metric("chain", count);
                    out.push(d);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// P17 DOUBLE_NEGATIVE
// ---------------------------------------------------------------------------

static P17_META: RuleMeta = RuleMeta {
    id: "P17",
    name: "DOUBLE_NEGATIVE",
    title: "二重否定",
    lane: Lane::Readability,
    status: RuleStatus::Stable,
    default_severity: Severity::Info,
    summary: "「〜ないわけではない」「〜ないとは言えない」のように否定を重ね、真偽の計算を強いる箇所を指す",
    explanation: r"### 何を見るか

「〜ないわけではない」「〜ないとは言えない」「〜ないとも限らない」「〜なくはない」「〜なくもない」「〜ないでもない」「〜ないこともない」のように、否定を二重に重ねた言い回しを探します。ます形・過去形も含みます。

### なぜ問題か

読み手は頭の中で符号を 2 回反転させてから意味を取ります。断定を避ける効果と引き換えに、読む速さが落ちます。否定の入れ子は読み手の負荷がもっとも大きい型の一つです。

### 直し方

肯定に言い換えられるなら言い換えます。ただし畳むときは、意味が反転していないかを必ず確かめます (「防げないわけではない」は「防げる場合がある」であって「防げる」ではありません)。控えめな肯定そのものが狙いなら残してかまいません。

### 例

- 直す前: この対策で遅延が防げないわけではない。
- 直した後: この対策で遅延を防げる場合がある。

### 根拠

AI らしさの判定ではなく、読みにくさの指さしです。自然度スコアには入りません。義務を表す定型 (「〜ないといけない」「〜なければならない」「〜ざるを得ない」) と、必要条件を述べる条件形 (「〜ないと動かない」「〜なければ意味がない」) は、形の上では否定が 2 つあっても読み手が符号を計算しないため拾いません。辞書がないため、否定の形態素を数える代わりに定型の表層パターンで拾います。「危なくはない」のように「ない」が否定でない形容詞は除きます。",
};

static DOUBLE_NEGATIVE_RES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // ないわけではない / ないことはない / ないものでもない / ないわけじゃない
        r"ない(?:わけ|訳|こと|事|もの)(?:(?:で|に)?(?:は|も)|じゃ)(?:ない|なかった|ありません|ありませんでした)",
        // ないとは言えない / ないとも限らない / ないとは思わない
        r"ない(?:とは|とも)(?:言え|いえ|限ら|かぎら|思わ|考え)(?:ない|なかった|ません|ませんでした)",
        // なくはない / なくもない
        r"なく(?:は|も)(?:ない|なかった|ありません|ありませんでした)",
        // ないでもない
        r"ないでも(?:ない|なかった|ありません|ありませんでした)",
    ]
    .iter()
    .map(|p| Regex::new(p).expect("double negative regex"))
    .collect()
});

/// 「ない」が否定ではない形容詞の語幹 (「危なくはない」を二重否定とみなさない)。
const NON_NEGATION_ADJECTIVE_STEMS: &[&str] = &[
    "危",
    "少",
    "切",
    "儚",
    "はか",
    "あっけ",
    "さりげ",
    "えげつ",
    "だらし",
    "ぎこち",
    "せわし",
    "やるせ",
];

pub(super) struct DoubleNegative;

impl DoubleNegative {
    fn find(text: &str) -> Vec<Range<usize>> {
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for re in DOUBLE_NEGATIVE_RES.iter() {
            for m in re.find_iter(text) {
                let before = &text[..m.start()];
                if m.as_str().starts_with("なく")
                    && NON_NEGATION_ADJECTIVE_STEMS
                        .iter()
                        .any(|stem| before.ends_with(stem))
                {
                    continue;
                }
                ranges.push(m.start()..m.end());
            }
        }
        ranges.sort_by_key(|r| (r.start, std::cmp::Reverse(r.end)));
        let mut out: Vec<Range<usize>> = Vec::new();
        for r in ranges {
            if out.last().is_some_and(|last| r.start < last.end) {
                continue;
            }
            out.push(r);
        }
        out
    }
}

impl Rule for DoubleNegative {
    fn meta(&self) -> &'static RuleMeta {
        &P17_META
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for (idx, block) in ctx.scoped_blocks() {
            for sentence in ctx.doc.block_sentences(idx) {
                let text = &block.text[sentence.range.clone()];
                for range in Self::find(text) {
                    let matched = &text[range.clone()];
                    let abs =
                        (sentence.range.start + range.start)..(sentence.range.start + range.end);
                    out.push(diagnostic(
                        &P17_META,
                        block.to_source(abs),
                        sentence.span,
                        matched,
                        format!("「{}」は否定を二重に重ねた言い回しです", quote(matched)),
                        "肯定に言い換えられるなら、意味が反転していないか確かめたうえで言い換えてください。控えめな肯定が狙いなら残してかまいません",
                        P17_META.default_severity,
                        P17_META.status,
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::morph::testing::morphology;
    use crate::rules::testing::{matched, run, run_with_morphology};

    #[test]
    fn p15_flags_long_kanji_runs() {
        let md = "顧客情報統合管理基盤移行計画を承認した。\n";
        let d = run(&KanjiRun::default(), md);
        assert_eq!(matched(md, &d), vec!["顧客情報統合管理基盤移行計画"]);
        assert!(d[0].message.contains("14 字"));
    }

    #[test]
    fn p15_ignores_short_runs_and_institutions() {
        for md in [
            "情報管理基盤を作る。\n",
            "東京地方裁判所に提出した。\n",
            "株式会社日本経済新聞社の記事。\n",
        ] {
            assert!(run(&KanjiRun::default(), md).is_empty(), "{md}");
        }
    }

    #[test]
    fn p15_threshold_is_configurable() {
        let mut rule = KanjiRun::default();
        rule.configure("min_length", &toml::Value::Integer(4))
            .unwrap();
        let md = "情報管理基盤を作る。\n";
        assert_eq!(matched(md, &run(&rule, md)), vec!["情報管理基盤"]);
        assert!(
            rule.configure("min_length", &toml::Value::Integer(1))
                .is_err()
        );
        assert!(rule.configure("nope", &toml::Value::Integer(4)).is_err());
        assert_eq!(rule.options(), vec![("min_length", "4".to_string())]);
    }

    #[test]
    fn p16_flags_chains_of_no() {
        let md = "新制度の運用の開始の時期を決めた。\n";
        let d = run(&NoChain::default(), md);
        assert_eq!(matched(md, &d), vec!["新制度の運用の開始の時期"]);
        assert!(d[0].message.contains("3 回"));
    }

    #[test]
    fn p16_counts_nouns_with_okurigana() {
        let md = "規程の見直しの検討の開始が急がれる。取り組みの進め方の案の共有。\n";
        let d = run(&NoChain::default(), md);
        assert_eq!(
            matched(md, &d),
            vec!["規程の見直しの検討の開始", "取り組みの進め方の案の共有"]
        );
        // 「見直して」の「し」は送り仮名とみなさないので連鎖が切れる
        let md = "規程の見直して検討の開始の時期。\n";
        assert!(run(&NoChain::default(), md).is_empty());
    }

    #[test]
    fn p16_ignores_short_chains_and_hiragana_words() {
        for md in [
            "運用コストの削減の実現を目指す。\n",
            "そのもののこの形を見る。\n",
            "東京の大学の、先生の本を読む。\n",
        ] {
            assert!(run(&NoChain::default(), md).is_empty(), "{md}");
        }
    }

    #[test]
    fn p16_threshold_is_configurable() {
        let mut rule = NoChain::default();
        rule.configure("min_chain", &toml::Value::Integer(2))
            .unwrap();
        let md = "運用コストの削減の実現を目指す。\n";
        assert_eq!(matched(md, &run(&rule, md)), vec!["運用コストの削減の実現"]);
    }

    // --- 形態素解析の辞書を使う判定 ---

    const NO: (&str, &str) = ("の", "助詞,連体化,*,*");
    const PERIOD: (&str, &str) = ("。", "記号,句点,*,*");
    const COMMA: (&str, &str) = ("、", "記号,読点,*,*");

    #[test]
    fn rules_that_count_parts_of_speech_use_the_dictionary() {
        assert!(KanjiRun::default().uses_morphology());
        assert!(NoChain::default().uses_morphology());
        assert!(!DoubleNegative.uses_morphology());
    }

    #[test]
    fn p16_with_a_dictionary_counts_case_particles_between_any_words() {
        let dict = morphology(&[
            ("俺", "名詞,代名詞,一般,*"),
            NO,
            ("魂", "名詞,一般,*,*"),
            ("安静", "名詞,形容動詞語幹,*,*"),
            ("ため", "名詞,非自立,副詞可能,*"),
            ("に", "助詞,格助詞,一般,*"),
            ("祈る", "動詞,自立,*,*"),
            PERIOD,
        ]);
        // 辞書なしでは、ひらがなの語「ため」で連鎖が切れて 2 回に数える
        let md = "俺の魂の安静のために祈る。\n";
        assert!(run(&NoChain::default(), md).is_empty());
        let d = run_with_morphology(&NoChain::default(), md, &dict);
        assert_eq!(matched(md, &d), vec!["俺の魂の安静のため"]);
        assert_eq!(d[0].metrics["chain"], crate::diagnostic::Metric::Int(3));
    }

    #[test]
    fn p16_with_a_dictionary_does_not_count_no_inside_a_word() {
        let dict = morphology(&[
            ("茶の間", "名詞,一般,*,*"),
            NO,
            ("茶箪笥", "名詞,一般,*,*"),
            ("上", "名詞,非自立,副詞可能,*"),
            ("に", "助詞,格助詞,一般,*"),
            ("置く", "動詞,自立,*,*"),
            PERIOD,
        ]);
        // 辞書なしでは「茶の間」の「の」も数えて 3 回になる
        let md = "茶の間の茶箪笥の上に置く。\n";
        assert_eq!(run(&NoChain::default(), md).len(), 1);
        assert!(run_with_morphology(&NoChain::default(), md, &dict).is_empty());
    }

    #[test]
    fn p16_with_a_dictionary_breaks_chains_at_long_gaps_and_punctuation() {
        let dict = morphology(&[
            ("猫", "名詞,一般,*,*"),
            NO,
            ("家", "名詞,一般,*,*"),
            ("庭", "名詞,一般,*,*"),
            ("木", "名詞,一般,*,*"),
            ("とても", "副詞,一般,*,*"),
            ("大きな", "連体詞,*,*,*"),
            ("だ", "助動詞,*,*,*"),
            COMMA,
            PERIOD,
        ]);
        let rule = NoChain::default();
        // 隣り合う「の」の間が 3 形態素
        let md = "猫の家のとても大きな庭の木だ。\n";
        assert!(run_with_morphology(&rule, md, &dict).is_empty());
        // 読点をはさむ
        let md = "猫の家の、庭の木だ。\n";
        assert!(run_with_morphology(&rule, md, &dict).is_empty());
        // 間が 2 形態素以内なら続く
        let md = "猫の家の大きな庭の木だ。\n";
        assert_eq!(
            matched(md, &run_with_morphology(&rule, md, &dict)),
            vec!["猫の家の大きな庭の木"]
        );
    }

    #[test]
    fn p15_with_a_dictionary_skips_runs_with_proper_nouns() {
        let dict = morphology(&[
            ("昭和", "名詞,固有名詞,一般,*"),
            ("八", "名詞,数,*,*"),
            ("年", "名詞,接尾,助数詞,*"),
            ("七", "名詞,数,*,*"),
            ("月", "名詞,一般,*,*"),
            ("発行", "名詞,サ変接続,*,*"),
            ("顧客", "名詞,一般,*,*"),
            ("情報", "名詞,一般,*,*"),
            ("統合", "名詞,サ変接続,*,*"),
            ("管理", "名詞,サ変接続,*,*"),
            ("基盤", "名詞,一般,*,*"),
            ("を", "助詞,格助詞,一般,*"),
            ("見る", "動詞,自立,*,*"),
            PERIOD,
        ]);
        let md = "昭和八年七月発行。\n";
        assert_eq!(run(&KanjiRun::default(), md).len(), 1);
        assert!(run_with_morphology(&KanjiRun::default(), md, &dict).is_empty());
        let md = "顧客情報統合管理基盤を見る。\n";
        assert_eq!(
            matched(md, &run_with_morphology(&KanjiRun::default(), md, &dict)),
            vec!["顧客情報統合管理基盤"]
        );
    }

    #[test]
    fn p17_flags_litotes() {
        let md = "遅延が防げないわけではない。負荷が増えないとは言えません。わからなくもない。\n";
        let d = run(&DoubleNegative, md);
        assert_eq!(
            matched(md, &d),
            vec!["ないわけではない", "ないとは言えません", "なくもない"]
        );
    }

    #[test]
    fn p17_skips_obligation_conditionals_and_adjectives() {
        for md in [
            "今日中に終えないといけない。\n",
            "手順を守らなければならない。\n",
            "中止せざるを得ない。\n",
            "設定しないと動かない。\n",
            "確認しなければ意味がない。\n",
            "この道は危なくはない。\n",
            "行かないし、来ない。\n",
        ] {
            assert!(run(&DoubleNegative, md).is_empty(), "{md}");
        }
    }
}
