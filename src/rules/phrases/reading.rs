//! 読みやすさの指摘 (P15 連続漢字、P16 「の」の連鎖、P17 二重否定)。
//!
//! このレーンは AI 臭さを測らない。読み手が一文の中で計算を強いられる箇所を指すだけで、
//! 自然度スコアには入らない。採否の基準は「指摘に従って直した文が読みやすくなるか」。

use std::ops::Range;
use std::sync::LazyLock;

use hasami::CoarsePos;
use regex::{Regex, RegexSet};

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::document::MarkKind;
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

漢字が 7 字以上 (既定) 続く箇所を探します。「々」も漢字に数えます。原文の改行と、太字・斜体・リンクの境目は、読み手に見える語の切れ目なので連なりを切ります (1 行 1 語の並びや、太字の見出しと本文をつないで数えないため)。形態素解析の辞書があれば、固有名詞を含む連なりを除きます。

### なぜ問題か

日本語は分かち書きをしないので、漢字が長く続くと語の切れ目の手掛かりがなくなり、読み手は区切りを推測しながら読むことになります。名詞を連ねて作った圧縮語は、書き手には一語でも読み手には初見の塊です。

### 直し方

語の切れ目が読み取れるか確かめ、助詞や動詞を補って開きます。

### 例

- 直す前: 顧客情報統合管理基盤移行計画を承認した。
- 直した後: 顧客情報をまとめて管理する基盤へ移る計画を承認した。

### 根拠

AI らしさの判定ではなく、読みやすさの指摘です。自然度スコアには入りません。実文書での校正で目安を 7 字以上とし、固有名詞を含む連なりは、分解しようのない名前なので除外していました。

形態素解析の辞書 (hasami) があれば、校正と同じく品詞で固有名詞を見分けて除きます。辞書がなければ固有名詞は見分けられないため、裁判所・研究所・委員会・株式会社などの定番の接尾辞で終わる (または始まる) 連なりだけを除きます。この接尾辞による除外は、辞書があっても行います。改行と装飾の境目で連なりを切るのは、元の検出器も行ごとに、装飾の記号をはさんだまま数えていたためです。元の検出器と同じ文書 373 本で比べると、指した箇所の一致率は辞書なしで 0.76、辞書ありで 0.90 でした (辞書なしは年号や人名を含む連なりを多く指す)。食い違った箇所を 1 件ずつ確かめると、辞書ありの指摘の精度は 0.88 (元の検出器は 0.86) でした。辞書 (IPAdic) が普通の語を固有名詞と解析して取りこぼすことがあり (「所謂」の「所」を姓とするなど)、長い固有名詞が指された場合は、残してかまいません。

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
    /// `breaks` は原文の改行があった位置 (`text` 上のバイト位置)。連なりは改行をまたがない。
    fn find(&self, text: &str, breaks: &[usize]) -> Vec<(Range<usize>, usize)> {
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
            // 段落内の改行は解析用テキストでは何も挟まずにつながるが、1 行 1 語の並び
            // (「対談」「面談」の行) を 1 つの連なりとして数えないよう、改行の位置で切る
            if breaks.contains(&i) {
                flush(&mut run);
            }
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
                // 改行と、太字・斜体・リンクの境目 (読み手には語の切れ目として見える)
                let marks = block
                    .marks
                    .iter()
                    .filter(|m| {
                        matches!(
                            m.kind,
                            MarkKind::Strong
                                | MarkKind::Emphasis
                                | MarkKind::Strikethrough
                                | MarkKind::Link
                        )
                    })
                    .flat_map(|m| [m.range.start, m.range.end]);
                let breaks: Vec<usize> = block
                    .line_breaks
                    .iter()
                    .copied()
                    .chain(marks)
                    .filter(|&at| sentence.range.start < at && at < sentence.range.end)
                    .map(|at| at - sentence.range.start)
                    .collect();
                let candidates = self.find(text, &breaks);
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

読点をはさまずに、短い語をつなぐ「の」が 3 回以上 (既定) 続く箇所を探します。形態素解析の辞書があれば、連体の「の」を数え、隣り合う「の」の間が 2 語以内で、句読点・括弧・中黒などの記号をはさまないものを続いているとみなします。辞書が細かく分ける語 (「必要性」「担当者」「話し方」のような接尾辞の付いた語、「三万七千」のような数、カタカナ語) は 1 語として数え、空白は数えません。分数の「分の」(「3分の1」) と、「目の前」のような「の」を含む決まった語の「の」は数えません。

### なぜ問題か

「の」は何にでも係るので、連なると、どの語がどの語を修飾しているのかが読み手に委ねられます。動作を名詞に固めた語 (「〜の導入」「〜の実現」) と組み合わさると、誰が何をするのかが見えなくなります。「の」は 2 回までを目安にします。

### 直し方

どこか 1 か所を動詞や「〜を〜する」の形に開きます。

### 例

- 直す前: 新制度の運用の開始の時期を決めた。
- 直した後: 新制度をいつから運用するかを決めた。

### 根拠

AI らしさの判定ではなく、読みやすさの指摘です。自然度スコアには入りません。元の検出器は形態素解析で格助詞の「の」だけを数え、実文書での校正で指した箇所はすべて本当の連鎖でした (再現率は低く、精度は高い)。

形態素解析の辞書 (hasami) があれば、元の検出器と同じ定義で数えます。元の検出器は複合語を 1 語にまとめる分割で数えていたので、IPAdic が細かく分ける語はまとめてから数えます。辞書がなければ、漢字・カタカナ・英数字の語に挟まれた「の」だけを数えます。そのため「この」「その」「もの」の「の」を誤って数えることはありませんが、ひらがなを含む語をはさむ連鎖 (「の家の大きな犬の」) や、端の語がひらがなの連鎖 (「魂の安静のため」) は拾えません。元の検出器と同じ文書 373 本で比べると、元が指した箇所のうち拾えたのは、辞書なしで 38%、辞書ありで 93% でした。食い違った箇所を 1 件ずつ確かめると、辞書ありの指摘の精度は 0.99 でした (元の検出器は 0.95。分数の「の」や「目の前」を数えていた)。

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

    /// 辞書の形態素で数える (元の検出器と同じ定義を、IPAdic の分割に合わせて数える)。
    ///
    /// 連体の「の」が `min_chain` 個以上並び、隣り合う「の」の間の語が 1〜2 個で、間に句読点・
    /// 括弧・記号がない箇所を返す。元の検出器は複合語を 1 語にする分割 (Sudachi のモード C) で
    /// 数えていたので、IPAdic が細かく分ける形は 1 語にまとめてから数える ([`chain_units`])。
    /// 範囲は、最初の「の」の前の名詞のまとまりから、最後の「の」の後の名詞のまとまりまで。
    fn find_with_tokens(&self, text: &str, tokens: &[MorphToken]) -> Vec<(Range<usize>, usize)> {
        let units = chain_units(text, tokens);
        let nos: Vec<usize> = units
            .iter()
            .enumerate()
            .filter(|(_, u)| u.kind == UnitKind::No)
            .map(|(k, _)| k)
            .collect();
        let nominal = |at: usize| units.get(at).filter(|u| u.kind == UnitKind::Nominal);
        let mut out = Vec::new();
        let mut k = 0;
        while k < nos.len() {
            let mut last = k;
            while let Some(&next) = nos.get(last + 1) {
                let between = &units[nos[last] + 1..next];
                if between.is_empty()
                    || between.len() > NO_CHAIN_MAX_GAP
                    || between.iter().any(|u| u.kind == UnitKind::Break)
                {
                    break;
                }
                last += 1;
            }
            let count = last - k + 1;
            if count >= self.min_chain {
                let first = &units[nos[k]];
                let start = nos[k]
                    .checked_sub(1)
                    .and_then(nominal)
                    .map_or(first.range.start, |u| u.range.start);
                let end =
                    nominal(nos[last] + 1).map_or(units[nos[last]].range.end, |u| u.range.end);
                out.push((start..end, count));
            }
            k = last + 1;
        }
        out
    }
}

/// 隣り合う「の」の間に置ける語の最大数 (元の検出器と同じ)。
const NO_CHAIN_MAX_GAP: usize = 2;

/// 「の」の連鎖を数える単位の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UnitKind {
    /// 連体の「の」。
    No,
    /// 名詞のまとまり (連鎖の範囲を広げる先)。
    Nominal,
    /// そのほかの語 (助詞・動詞など)。
    Other,
    /// 連鎖を切る記号 (句読点・括弧・中黒など)。
    Break,
}

/// 「の」の連鎖を数える単位 (1 語、または IPAdic が細かく分けた語をまとめたもの)。
#[derive(Debug)]
struct Unit {
    kind: UnitKind,
    range: Range<usize>,
}

/// 名詞のまとまりを作る品詞。
fn is_nominal(pos: CoarsePos) -> bool {
    matches!(
        pos,
        CoarsePos::Noun
            | CoarsePos::ProperNoun
            | CoarsePos::Pronoun
            | CoarsePos::Numeral
            | CoarsePos::NounSuffix
            | CoarsePos::FormalNoun
            | CoarsePos::Prefix
    )
}

/// 形態素の並びを、「の」の連鎖を数える単位に分ける。
///
/// - 空白は数えない (hasami は空白を記号のトークンとして出す。owayo/hasami#5)
/// - 格助詞の「の」と、用言のあとで名詞の前に来る準体助詞の「の」(「聴くと伝えるの両立」) を
///   連体の「の」とする
/// - 「分の」(IPAdic の助数詞) は、後ろが数詞なら分数 (「3分の1」) として前後の数と 1 語にし、
///   そうでなければ「分」と連体の「の」に分ける (「30分の早歩き」)
/// - IPAdic が細かく分ける形を 1 語にまとめる ([`joins`])
/// - 句読点・括弧・そのほかの記号 (中黒など) は連鎖を切る。コードなどの置き換え文字は名詞とみなす
fn chain_units(text: &str, tokens: &[MorphToken]) -> Vec<Unit> {
    let tokens: Vec<&MorphToken> = tokens
        .iter()
        .filter(|t| !text[t.range.clone()].trim().is_empty())
        .collect();
    let surface = |t: &MorphToken| &text[t.range.clone()];
    // 「目の前」の「の」(辞書が 3 語に分ける決まった語の中の「の」)
    let fixed_no = |k: usize| {
        surface(tokens[k]) == "の"
            && k.checked_sub(1)
                .zip(tokens.get(k + 1))
                .is_some_and(|(p, n)| FIXED_NO_WORDS.contains(&(surface(tokens[p]), surface(n))))
    };
    let mut units: Vec<Unit> = Vec::new();
    for (k, &t) in tokens.iter().enumerate() {
        let prev = k.checked_sub(1).map(|p| tokens[p]);
        let next = tokens.get(k + 1).map(|n| n.pos);
        if (fixed_no(k) || k.checked_sub(1).is_some_and(fixed_no))
            && let Some(last) = units.last_mut()
        {
            last.range.end = t.range.end;
            last.kind = UnitKind::Nominal;
            continue;
        }
        if t.pos == CoarsePos::NounSuffix
            && surface(t) == "分の"
            && next != Some(CoarsePos::Numeral)
        {
            let split = t.range.end - "の".len();
            match units.last_mut() {
                Some(last) if prev.is_some_and(|p| p.pos == CoarsePos::Numeral) => {
                    last.range.end = split;
                    last.kind = UnitKind::Nominal;
                }
                _ => units.push(Unit {
                    kind: UnitKind::Nominal,
                    range: t.range.start..split,
                }),
            }
            units.push(Unit {
                kind: UnitKind::No,
                range: split..t.range.end,
            });
            continue;
        }
        let nominalizer = t.pos == CoarsePos::FormalNoun
            && prev.is_some_and(|p| {
                matches!(
                    p.pos,
                    CoarsePos::Verb | CoarsePos::Adjective | CoarsePos::AuxVerb
                )
            })
            && next.is_some_and(|n| {
                matches!(
                    n,
                    CoarsePos::Noun
                        | CoarsePos::ProperNoun
                        | CoarsePos::Pronoun
                        | CoarsePos::Numeral
                        | CoarsePos::Prefix
                )
            });
        let kind = if surface(t) == "の" && (t.pos == CoarsePos::CaseParticle || nominalizer) {
            UnitKind::No
        } else if is_nominal(t.pos)
            || surface(t).chars().all(|c| c == PLACEHOLDER)
            || is_unit_symbol(surface(t))
        {
            UnitKind::Nominal
        } else if matches!(
            t.pos,
            CoarsePos::Period
                | CoarsePos::Comma
                | CoarsePos::OpenBracket
                | CoarsePos::CloseBracket
                | CoarsePos::Symbol
        ) {
            UnitKind::Break
        } else {
            UnitKind::Other
        };
        if matches!(kind, UnitKind::Nominal | UnitKind::Other)
            && let Some(p) = prev
            && let Some(last) = units.last_mut()
            && matches!(last.kind, UnitKind::Nominal | UnitKind::Other)
            && joins(text, p, t)
        {
            last.range.end = t.range.end;
            if kind == UnitKind::Nominal {
                last.kind = UnitKind::Nominal;
            }
            continue;
        }
        units.push(Unit {
            kind,
            range: t.range.clone(),
        });
    }
    units
}

/// 「の」を含む決まった語のうち、IPAdic が「の」の前後で分ける語 (前の語, 後ろの語)。
/// 「目の当たり」「身の回り」「世の中」「手の内」は辞書に 1 語で入っている。
const FIXED_NO_WORDS: [(&str, &str); 1] = [("目", "前")];

/// 数に付く単位の記号か (「%」「℃」「㎏」「㌔」)。
fn is_unit_symbol(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            matches!(
                c,
                '%' | '％' | '‰' | '°' | '℃' | '℉' | '\u{3300}'..='\u{33FF}'
            )
        })
}

/// `t` が直前の形態素 `prev` と 1 語にまとまるか。
///
/// 元の検出器 (Sudachi のモード C) が 1 語にする形のうち、IPAdic が分けるものに絞る。
/// 名詞どうしは、辞書にない複合語まで 1 語にすると連鎖の間が緩むのでまとめない。
fn joins(text: &str, prev: &MorphToken, t: &MorphToken) -> bool {
    let katakana = |m: &MorphToken| text[m.range.clone()].chars().all(text::is_katakana);
    match (prev.pos, t.pos) {
        // 「必要性」「転職者」「物理的」「向き合い方」「難しさ」
        (
            CoarsePos::Noun
            | CoarsePos::ProperNoun
            | CoarsePos::Pronoun
            | CoarsePos::Numeral
            | CoarsePos::NounSuffix
            | CoarsePos::Verb
            | CoarsePos::Adjective,
            CoarsePos::NounSuffix,
        ) => true,
        // 「お客様」「第三者」のような接頭辞と後ろの語
        (CoarsePos::Prefix, pos) => is_nominal(pos) || pos == CoarsePos::Verb,
        // 「三万七千」
        (CoarsePos::Numeral, CoarsePos::Numeral) => true,
        // 「4㎏」「50%」(hasami は単位の記号を 記号,一般 にする。owayo/hasami#7)
        (CoarsePos::Numeral, CoarsePos::Symbol) => is_unit_symbol(&text[t.range.clone()]),
        // 分数「3分の1」の後ろの数
        (CoarsePos::NounSuffix, CoarsePos::Numeral) => &text[prev.range.clone()] == "分の",
        // カタカナ語の並び (辞書にない語が 2 字ずつに割れた場合を含む。owayo/hasami#4)
        _ => katakana(prev) && katakana(t),
    }
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

「〜ないわけではない」「〜ないとは言えない」「〜ないとも限らない」「〜なくはない」「〜なくもない」「〜ないでもない」「〜ないこともない」のように否定を二重に重ねた言い回しと、「〜ずにはいられない」「〜ないわけにはいかない」「〜ないはずはない」「〜ない人はいない」「無きにしもあらず」のような二重否定の定型、「〜ないのではない」「〜ないからではない」「〜ないということではない」のような否定の入れ子を探します。ます形・過去形 (「〜なかったわけではない」) と、「無い」「ワケ」の表記も含みます。

### なぜ問題か

読み手は頭の中で符号を 2 回反転させてから意味を取ります。断定を避ける効果と引き換えに、読む速さが落ちます。否定の入れ子は読み手の負荷がもっとも大きい型の一つです。

### 直し方

肯定に言い換えられるなら言い換えます。ただし畳むときは、意味が反転していないかを必ず確かめます (「防げないわけではない」は「防げる場合がある」であって「防げる」ではありません)。控えめな肯定そのものが狙いなら残してかまいません。

### 例

- 直す前: この対策で遅延が防げないわけではない。
- 直した後: この対策で遅延を防げる場合がある。

### 根拠

AI らしさの判定ではなく、読みやすさの指摘です。自然度スコアには入りません。元の検出器は、否定の形態素 (「ない」「無い」「ぬ」「ず」) が近くに 2 つ並ぶ箇所を拾っていました。辞書がないため、否定の形態素を数える代わりに定型の表層パターンで拾います。「〜ないわけではない」の類の 4 つの定型から始め、元の定義の範囲に合わせて、上に挙げた定型と否定の入れ子、「無い」「ワケ」の表記まで広げました。

形の上では否定が 2 つあっても、読み手が符号を計算しない次の形は拾いません。義務・許可の定型 (「〜ないといけない」「〜なければならない」「〜ざるを得ない」「〜なくても構わない」)、必要条件を述べる条件形 (「〜ないと動かない」「〜なければ意味がない」)、推量 (「〜ないかもしれない」)、否定の連体修飾に否定の述語が続くだけの形 (「〜ない人間ではない」)、読点で区切った別々の否定 (「〜ず、〜ない」) です。「〜ないのではないか」「〜ないではないか」「〜ない人はいないか」のように文末が問いや念押しになる形も、否定を 2 回計算させないので除きます。「危なくはない」「少ない人はいない」のように「ない」が否定でない形容詞も除きます。元の検出器と同じ文書 373 本で指摘を 1 件ずつ確かめると、元の検出器の指摘 469 件のうち二重否定か否定の入れ子だったのは 57 件 (精度 0.12) で、noslop の指摘 58 件はすべてそのどちらかでした。",
};

/// 前の否定 (「ない」と漢字の「無い」)。
const NEG: &str = "(?:ない|無い)";
/// 前の否定の過去形。
const NEG_PAST: &str = "(?:なかった|無かった)";
/// 形式名詞 (わけ・こと・もの) と、そのあとの「は」「も」「では」「じゃ」など。
const NOUN_TOPIC: &str = "(?:わけ|訳|ワケ|こと|事|もの)(?:(?:で|に)?(?:は|も)|じゃ)";
/// 「とは」「とも」に続く動詞の否定 (言えない・限らない・言い切れない など)。
const SAY_NOT: &str = "(?:言え|いえ|言い切れ|言いきれ|言われ|限ら|かぎら|思わ|考え)(?:ない|なかった|ません|ませんでした)";
/// 後ろの否定。ます形・過去形のほか、「なさそう」「なかろう」も否定の形態素として数える。
const TAIL: &str = "(?:ない|無い|なかった|無かった|なさそう|なかろう|ありません|ありませんでした)";
/// 「では」「じゃ」に続く否定。「〜ないのではなく、」の中止形も含む。
const TAIL_DEHA: &str = "(?:ない|無い|なかった|無かった|なく|なかろう|ありません|ありませんでした)";

/// 二重否定の型 1 つ。
struct NegationForm {
    re: Regex,
    /// 形容詞の「ない」(「少ない人はいない」) と、文末を問いや念押しにする形
    /// (「〜ないのではないか」) を除くか。
    ///
    /// はじめからある 4 つの定型は、校正したときの挙動を保つため除かない。ただし「なく」で始まる
    /// 形容詞 (「危なくはない」) は、どの型でも除く。
    guarded: bool,
}

/// 二重否定の型の一覧と、それらをまとめた集合。
struct NegationForms {
    /// すべての型の集合。文に現れる型を 1 回の走査で絞ってから、その型だけで位置を求める
    /// (型ごとに文を走査すると、型の数だけ時間がかかる)。
    any: RegexSet,
    forms: Vec<NegationForm>,
}

static DOUBLE_NEGATIVE_FORMS: LazyLock<NegationForms> = LazyLock::new(|| {
    let specs: [(String, bool); 13] = [
        // --- はじめからある定型 (表記ゆれを足したもの) ---
        // ないわけではない / ないことはない / ないものでもない / ないわけじゃない / ないワケでもない
        (format!("{NEG}{NOUN_TOPIC}{TAIL}"), false),
        // ないとは言えない / ないとも限らない / ないとは思わない / ないとは言い切れない
        (format!("{NEG}(?:とは|とも){SAY_NOT}"), false),
        // なくはない / なくもない
        (format!("(?:なく|無く)(?:は|も){TAIL}"), false),
        // ないでもない
        (format!("{NEG}でも{TAIL}"), false),
        // --- 元の定義の範囲に合わせて足した型 ---
        // なかったわけではない / なかったとは言えない
        (format!("{NEG_PAST}{NOUN_TOPIC}{TAIL}"), true),
        (format!("{NEG_PAST}(?:とは|とも){SAY_NOT}"), true),
        // ないではない / ないのではない / ないのではなく / なかったのではない
        (format!("(?:{NEG}|{NEG_PAST})の?では{TAIL_DEHA}"), true),
        // 否定の入れ子: ないからではない / ないということではない / ないせいなのではない
        (
            format!(
                "(?:{NEG}|{NEG_PAST})(?:から|ため|せい|という(?:こと|の)?)(?:なの)?(?:では|じゃ){TAIL_DEHA}"
            ),
            true,
        ),
        // ないはずはない / ないはずがない / ないわけがない
        (
            format!("(?:{NEG}|{NEG_PAST})(?:(?:はず|筈)(?:が|は|も)|(?:わけ|訳|ワケ)が){TAIL}"),
            true,
        ),
        // ずにはいられない / ずにいられない / ないではいられない
        (
            "(?:ずに|ないで)は?(?:いられ|居られ)(?:ない|なかった|なく|ません|ませんでした|ぬ|ず)"
                .to_string(),
            true,
        ),
        // ないわけにはいかない / ない訳に行かない / ないわけにはいきません
        (
            format!(
                "{NEG}(?:わけ|訳|ワケ)に(?:は|も)?(?:(?:いか|行か|ゆか)(?:ない|なかった|なく|ず|ぬ|ん)|(?:いき|行き|ゆき)(?:ません|ませんでした))"
            ),
            true,
        ),
        // ない人はいない / なかった人はいない / ない者はない (全称の二重否定)
        (
            format!(
                "(?:{NEG}|{NEG_PAST})(?:(?:人|者)(?:は|も)(?:いない|いなかった|いません|いませんでした)|者(?:は|も)(?:ない|なかった))"
            ),
            true,
        ),
        // 無きにしもあらず
        ("(?:無|な)きにしも(?:あら|非)ず".to_string(), true),
    ];
    let any = RegexSet::new(specs.iter().map(|(pattern, _)| pattern))
        .unwrap_or_else(|e| panic!("二重否定の正規表現が不正です: {e}"));
    let forms = specs
        .into_iter()
        .map(|(pattern, guarded)| NegationForm {
            re: Regex::new(&pattern)
                .unwrap_or_else(|e| panic!("二重否定の正規表現が不正です ({pattern}): {e}")),
            guarded,
        })
        .collect();
    NegationForms { any, forms }
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

/// 一致の直後が、文を問いや念押しにする形か (「〜ないのではないか」「〜ないではないか」
/// 「〜ないからではないでしょうか」「〜ありませんか？」)。
///
/// 問いや念押しの「ではないか」は否定として読まれないので、読み手は符号を 2 回計算しない。
/// 「から」(理由) と「かも」(推量) の「か」は問いではない。
fn ends_as_question(after: &str) -> bool {
    let rest = after.strip_prefix(['の', 'ん']).unwrap_or(after);
    let rest = ["です", "でしょう", "だろう"]
        .iter()
        .find_map(|p| rest.strip_prefix(p))
        .unwrap_or(rest);
    if rest.starts_with(['?', '？']) {
        return true;
    }
    rest.strip_prefix('か')
        .is_some_and(|next| !next.starts_with(['ら', 'も']))
}

pub(super) struct DoubleNegative;

impl DoubleNegative {
    fn find(text: &str) -> Vec<Range<usize>> {
        let mut ranges: Vec<Range<usize>> = Vec::new();
        let forms = &*DOUBLE_NEGATIVE_FORMS;
        for idx in forms.any.matches(text).iter() {
            let form = &forms.forms[idx];
            for m in form.re.find_iter(text) {
                let (before, after) = (&text[..m.start()], &text[m.end()..]);
                let matched = m.as_str();
                // 「危なくはない」「少ない人はいない」の「ない」は否定ではない
                let negation_may_be_adjective = if form.guarded {
                    matched.starts_with('な')
                } else {
                    matched.starts_with("なく")
                };
                if negation_may_be_adjective
                    && NON_NEGATION_ADJECTIVE_STEMS
                        .iter()
                        .any(|stem| before.ends_with(stem))
                {
                    continue;
                }
                if form.guarded
                    && (ends_as_question(after)
                        // 「〜ない者はないがしろにされる」の後ろの「ない」は否定ではない
                        || (matched.ends_with("ない") && after.starts_with("がしろ")))
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
    fn p15_runs_do_not_cross_line_breaks() {
        // 1 行 1 語の並びは、解析用テキストではつながっても別々の語
        let md = "候補は次のとおり。\n対談記事\n面談記録\n座談会\n";
        assert!(run(&KanjiRun::default(), md).is_empty());
        // 行の中の連なりは、改行の前後に文が続いても数える
        let md = "今回は\n顧客情報統合管理基盤を\n移す。\n";
        assert_eq!(
            matched(md, &run(&KanjiRun::default(), md)),
            vec!["顧客情報統合管理基盤"]
        );
        // 太字の境目も語の切れ目として見える
        let md = "目的:売上計画達成**期間:来年度**\n";
        assert!(run(&KanjiRun::default(), md).is_empty());
        let md = "新たな**顧客情報統合管理基盤**を作る。\n";
        assert_eq!(
            matched(md, &run(&KanjiRun::default(), md)),
            vec!["顧客情報統合管理基盤"]
        );
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

    /// 同梱の辞書 (IPAdic) で、辞書なしの近似との違いを確かめる。
    #[cfg(feature = "bundled-dict")]
    #[test]
    fn the_bundled_dictionary_judges_by_part_of_speech() {
        let dict = crate::morph::Morphology::bundled().unwrap();
        let md = "俺の魂の安静のために祈る。\n";
        assert_eq!(
            matched(md, &run_with_morphology(&NoChain::default(), md, &dict)),
            vec!["俺の魂の安静のため"]
        );
        let md = "茶の間の茶箪笥の上に置く。\n";
        assert!(run_with_morphology(&NoChain::default(), md, &dict).is_empty());
        let md = "昭和八年七月発行の雑誌を読んだ。\n";
        assert!(run_with_morphology(&KanjiRun::default(), md, &dict).is_empty());
        let md = "顧客情報統合管理基盤移行計画を承認した。\n";
        assert_eq!(
            run_with_morphology(&KanjiRun::default(), md, &dict).len(),
            1
        );
    }

    /// 同梱の辞書 (IPAdic) で、細かく分かれた語をまとめて「の」の間を数えることを確かめる。
    #[cfg(feature = "bundled-dict")]
    #[test]
    fn p16_counts_words_split_by_the_bundled_dictionary_as_one() {
        let dict = crate::morph::Morphology::bundled().unwrap();
        let chains = |md: &str| -> Vec<String> {
            matched(md, &run_with_morphology(&NoChain::default(), md, &dict))
                .into_iter()
                .map(str::to_string)
                .collect()
        };
        // 接尾辞 (「必要性」「担当者」「話し方」)・カタカナ語・「30分」を 1 語として数える
        for (md, want) in [
            (
                "計画の見直しの必要性などの確認を急ぐ。\n",
                "計画の見直しの必要性などの確認",
            ),
            (
                "前回の担当者との話し方の違いを見る。\n",
                "前回の担当者との話し方の違い",
            ),
            (
                "来期のクオリティの目標の設定を決める。\n",
                "来期のクオリティの目標の設定",
            ),
            (
                "毎朝の30分の散歩の習慣を続ける。\n",
                "毎朝の30分の散歩の習慣",
            ),
            (
                "開発の ツールの 設定の変更を行う。\n",
                "開発の ツールの 設定の変更",
            ),
            (
                "話すと聞くの両立のための工夫がいる。\n",
                "の両立のための工夫",
            ),
            (
                "設定は `config` の `path` の値の型を見る。\n",
                "`config` の `path` の値の型",
            ),
            ("前月の4㎏の差の原因を探る。\n", "前月の4㎏の差の原因"),
        ] {
            assert_eq!(chains(md), vec![want], "{md}");
        }
        // 分数の「の」、中黒で区切った「の・」、準体助詞の「のは」、「目の前」の「の」は数えない
        for md in [
            "予算の3分の1の額を充てる。\n",
            "自分の目の前の作業に戻る。\n",
            "週二回の・市場の調査の担当になる。\n",
            "読むのは次の人の番だ。\n",
        ] {
            assert!(chains(md).is_empty(), "{md}");
        }
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

    #[test]
    fn p17_flags_fixed_double_negative_forms() {
        // 装飾をまたぐ一致は、原文の装飾記号ごと範囲に含める
        let md = "この話を聞くと、**笑わずには**いられない。ここまで来たら、引き受けないわけにはいかない。\
                  手順どおりなら、動かないはずはない。新しい道具を試したくない人はいない。\
                  改善の余地は無きにしもあらずだ。あのときは、祈らずにいられなかった。\n";
        let d = run(&DoubleNegative, md);
        assert_eq!(
            matched(md, &d),
            vec![
                "ずには**いられない",
                "ないわけにはいかない",
                "ないはずはない",
                "ない人はいない",
                "無きにしもあらず",
                "ずにいられなかった",
            ]
        );
        assert!(d.iter().all(|d| d.severity == Severity::Info));

        let md = "彼の苦労を思うと、同情しないではいられない。頼まれた以上、行かない訳に行かない。\
                  これだけ遅れれば、上司が怒らないわけがありません。\n";
        assert_eq!(
            matched(md, &run(&DoubleNegative, md)),
            vec![
                "ないではいられない",
                "ない訳に行かない",
                "ないわけがありません",
            ]
        );
    }

    #[test]
    fn p17_flags_nested_negations() {
        let md = "会議が要らないのではない。失敗したのは、道具を知らないからではない。\
                  予算が無いということではない。手を抜かないのではなく、抜き方を知らないのだ。\
                  準備が足りなかったからではなく、手順が古かった。\n";
        assert_eq!(
            matched(md, &run(&DoubleNegative, md)),
            vec![
                "ないのではない",
                "ないからではない",
                "無いということではない",
                "ないのではなく",
                "なかったからではなく",
            ]
        );
    }

    #[test]
    fn p17_flags_notation_and_tense_variants() {
        let md = "反対する理由が無いわけではない。抜け道がないワケでもない。\
                  迷いが無いでもなかった。当時も、知らなかったわけではない。\
                  失敗しないとは言い切れない。反対が出ないこともなさそうだ。\
                  彼は手順を知らなかったのではない。あの映画を観て泣かなかった人はいない。\n";
        assert_eq!(
            matched(md, &run(&DoubleNegative, md)),
            vec![
                "無いわけではない",
                "ないワケでもない",
                "無いでもなかった",
                "なかったわけではない",
                "ないとは言い切れない",
                "ないこともなさそう",
                "なかったのではない",
                "なかった人はいない",
            ]
        );
    }

    #[test]
    fn p17_skips_forms_that_are_not_double_negatives() {
        for md in [
            // 義務・許可
            "申請書は今日中に出さなくてはいけません。\n",
            "約束は守らねばならぬ。\n",
            "手順は必ず守らなければならない。\n",
            "無理に参加しなくても構わない。\n",
            // 推量
            "明日は雨が降らないかもしれない。\n",
            // 別々の否定が近くにあるだけ
            "彼は約束を守らない人間ではない。\n",
            "何も言わず、動かない。\n",
        ] {
            assert!(run(&DoubleNegative, md).is_empty(), "{md}");
        }
    }

    #[test]
    fn p17_skips_questions_adjectives_and_other_words() {
        for md in [
            // 文末が問いや念押しになる形
            "遅れの原因は、人手が足りないからではないか。\n",
            "このままでは、期限に間に合わないのではないでしょうか。\n",
            "それでは、誰も得をしないではないか。\n",
            "あの判断は間違っていなかったのではないか。\n",
            "まだ申請していない人はいないか確かめる。\n",
            "急いでいるときに、パスワードを思い出せなかったことはありませんか？\n",
            // 「ない」が否定ではない形容詞
            "この町に危ない人はいない。\n",
            "参加者が少ないのではない。\n",
            // 後ろの「ない」が「ないがしろ」の一部
            "声を上げない者はないがしろにされる。\n",
            // 既定ではリストを見ない
            "- 笑わずにはいられない。\n",
        ] {
            assert!(run(&DoubleNegative, md).is_empty(), "{md}");
        }
        // 理由の「から」と推量の「かも」の「か」は問いではない
        let md = "理由が分からないのではないから、説明は要らない。\
                  この機能を使わない人はいないかもしれない。\n";
        assert_eq!(
            matched(md, &run(&DoubleNegative, md)),
            vec!["ないのではない", "ない人はいない"]
        );
    }
}
