//! 構文の型で拾う語句ルール (P13 無生物主語、P14 ダッシュの挿入句、P20 述語とコロンでの列挙の導入)。
//!
//! 辞書の照合だけでは誤爆する (「指示」「表示」の「示」を動詞と取り違える、地名の区間を表す
//! ダッシュを挿入句と取り違える、名詞のラベルとコロンを述語と取り違える) ため、一致の前後や
//! 次のブロックを見て判定する。

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, Severity, Signal};
use crate::document::BlockKind;
use crate::rules::{Rule, RuleContext, RuleMeta, RuleUnit, quote};
use crate::text;
use crate::text::quotes::QuoteRanges;

use super::engine::diagnostic;

// ---------------------------------------------------------------------------
// P13 INANIMATE_SUBJECT
// ---------------------------------------------------------------------------

static P13_META: RuleMeta = RuleMeta {
    id: "P13",
    name: "INANIMATE_SUBJECT",
    title: "無生物主語＋他動詞",
    lane: Lane::Slop,
    signal: Some(Signal::Moderate),
    default_severity: Severity::Info,
    summary: "「この事実は〜を示している」のような、無生物を主語にして他動詞で結ぶ直訳調の構文を指摘する",
    explanation: r"### 何を見るか

「これは」「それが」「この事実は」「そのことは」「〜ことが」のような抽象的な主語のあと、40 字以内に「もたらす」「示す」「意味する」「証明する」「生み出す」「反映する」「示唆する」「物語る」「浮き彫りにする」「後押しする」「暗示する」が来る文を探します。主語のあとで開いた括弧の中の述語 (「これは「〜と証明した」ものではない」) は、主語の述語ではないので数えません。主語が括弧の中にあり、述語より前でその括弧が閉じる文 (「「〜ことが大切」との見解を示した」の「ことが」) も、主語が括弧の中の節に属するので数えません。

「〜ことが示された」「〜がもたらされる」のような受け身は数えません。未然形の「示さ」「もたらさ」「生み出さ」に「れ」が続く形で、主語は動作を受ける側になり、無生物が他動詞の主体になる構文ではないためです。「何も生み出さない」のような否定は数えます。

### なぜ問題か

英語は事実やデータを主語に立てて他動詞でつなぐ構文を好みます (This result shows ...)。これをそのまま日本語に移すと、「結果」や「事実」といった物が自分から何かを示したり生んだりすることになり、判断した人の姿が文から消えます。日本語では人や場面を主語に立てて語るほうが自然なので、こうした文はよそよそしく響きます。

### 直し方

主語を人や状況に戻すか、「〜から分かる」「〜になっている」のような述べ方に変えます。

### 例

- 直す前: この事実は、利用者の関心が移ったことを示している。
- 直した後: この事実から、利用者の関心が移ったと分かる。

### 根拠

受け身と括弧の中の主語を除いても、人の文書 176 本 (一般・技術・業務・随筆・スライド) のうち 11 本 (6.3%) に反応し、生成文書 (2025 年頃のモデルなどの 390 本で 22 本・5.6%、2026 年のモデルの 1,339 本で 29 本・2.2%) と同じかそれ以上でした。人の白書・随筆・技術記事は「これは〜を意味する」「〜を示す」をよく使います (「示す」は人 3 本・生成 15 本、「意味する」は人 3 本・生成 6 本)。生成文書に偏るのは「もたらす」(人 2 本・生成 20 本) くらいで、構文全体では人と生成を見分けられません。辞書を使わず、述語は活用の語幹 (「示し」「もたらさ」など) で照合します。未然形の「もたらさ」「示さ」「生み出さ」は、否定 (「何も生み出さない」) のときだけ拾います。「指示」「表示」のように前に漢字が付く「示」は動詞とみなしません。「暗示する」は人の随筆にも出る語です。

受け身と、括弧の中の主語を除くと、人の文書 176 本の指摘は 27 件から 15 件に、生成文書 1,729 本の指摘は 61 件から 57 件になりました。外れたのは、白書や報告書に多い「〜ことが示された」の形 (人の文書 10 件・生成文書 4 件) と、発言を引いた括弧の中の「ことが」(人の文書 2 件) です。

手掛かりの強さは中程度です。人の文書 440 本と生成文書 390 本で測ると、指摘が出た文書は人 14 本・生成 22 本でした。述語では、「示す」(人 3 本・生成 8 本) は強い手掛かりで、「もたらす」(人 3 本・生成 7 本) と「反映する」(人 4 本・生成 4 本) は中程度でした。",
};

/// 抽象的な主語と主題・主格の助詞。長い候補を先に置く。
static SUBJECT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:この事実|その事実|このこと|そのこと|これ|それ|こと|事実)(?:は|が)")
        .expect("subject regex")
});

/// 直訳調でよく使われる他動詞 (活用の語幹で近似する)。
///
/// 元の定義は述語の原形の一致なので、未然形 (「何も生み出さない」「示される」) も拾う。
static VERB_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"もたら[すしさ]|示唆(?:する|し)|暗示(?:する|し)|示[すしさ]|意味(?:する|し)|証明(?:する|し)|生み出[すしさ]|反映(?:する|し)|物語(?:る|っ)|浮き彫りに(?:する|し)|後押し(?:する|し)",
    )
    .expect("verb regex")
});

/// 主語のあと、他動詞を探す範囲の文字数。
const P13_WINDOW_CHARS: usize = 40;

/// 他動詞の語幹と、項目として数えるときの終止形 (`VERB_RE` の選択肢と同じ並び)。
///
/// `noslop calibrate` が述語ごとに人と生成文書での出方を比べられるよう、診断の `item` に
/// 活用をそろえた形を入れる (「示し」「示さ」はどちらも「示す」)。「示唆」は「示」より先に見る。
const P13_VERB_ITEMS: [(&str, &str); 11] = [
    ("もたら", "もたらす"),
    ("示唆", "示唆する"),
    ("暗示", "暗示する"),
    ("示", "示す"),
    ("意味", "意味する"),
    ("証明", "証明する"),
    ("生み出", "生み出す"),
    ("反映", "反映する"),
    ("物語", "物語る"),
    ("浮き彫り", "浮き彫りにする"),
    ("後押し", "後押しする"),
];

/// 一致した他動詞 (活用形) の終止形。一覧にない形なら一致した形のまま返す。
fn p13_verb_item(verb: &str) -> String {
    P13_VERB_ITEMS
        .iter()
        .find(|(stem, _)| verb.starts_with(stem))
        .map_or_else(|| verb.to_string(), |(_, item)| (*item).to_string())
}

/// P13 の項目 (述語の終止形) のうち、測った手掛かりの強さがルール (中程度) と違うもの。
const P13_ITEM_SIGNALS: &[(&str, Signal)] = &[("示す", Signal::Strong)];

/// 項目の手掛かりの強さ。`overrides` にある項目はその値、ほかはルールの値。
fn item_signal(meta: &RuleMeta, overrides: &[(&str, Signal)], item: &str) -> Option<Signal> {
    overrides
        .iter()
        .find(|(name, _)| *name == item)
        .map(|(_, signal)| *signal)
        .or(meta.signal)
}

pub(super) struct InanimateSubject;

impl InanimateSubject {
    /// 文のテキストから (主語の開始〜述語の終わり, 主語, 述語) を探す。
    fn find(text: &str) -> Vec<(Range<usize>, String, String)> {
        let mut out: Vec<(Range<usize>, String, String)> = Vec::new();
        for subject in SUBJECT_RE.find_iter(text) {
            // この事実は → 事実は のように同じ主語を二重に数えない
            if out.last().is_some_and(|(r, _, _)| subject.start() < r.end) {
                continue;
            }
            let window_end = text[subject.end()..]
                .char_indices()
                .nth(P13_WINDOW_CHARS)
                .map_or(text.len(), |(i, _)| subject.end() + i);
            let window = &text[subject.end()..window_end];
            let verb = VERB_RE.find_iter(window).find(|m| {
                let start = subject.end() + m.start();
                let end = subject.end() + m.end();
                // 「指示」「表示」「提示」のように前に漢字が付く「示」は名詞の一部
                let compound = m.as_str().starts_with('示')
                    && text[..start]
                        .chars()
                        .next_back()
                        .is_some_and(text::is_kanji);
                !compound
                    && !is_passive(m.as_str(), &text[end..])
                    && !crosses_bracket(&text[..subject.start()], &text[subject.end()..start])
            });
            if let Some(verb) = verb {
                let end = subject.end() + verb.end();
                out.push((
                    subject.start()..end,
                    subject.as_str().to_string(),
                    verb.as_str().to_string(),
                ));
            }
        }
        out
    }
}

/// 一致した述語が受け身の形か (「示さ」「もたらさ」「生み出さ」の未然形に「れ」が続く)。
///
/// 「〜ことが示された」「〜がもたらされる」の主語は動作を受ける側で、無生物が他動詞の主体になる
/// 直訳調の構文ではない。否定 (「示さない」) は主体のままなので数える。`rest` は述語の直後から文末まで。
fn is_passive(verb: &str, rest: &str) -> bool {
    verb.ends_with('さ') && rest.starts_with('れ')
}

/// 主語と述語が、括弧の同じ深さにないか。
///
/// 次の 2 つは、括弧が主語と述語を隔てているので、主語の述語とみなさない。
///
/// - 主語のあとで開いて閉じていない括弧の中の述語 (引用や名詞修飾の中の述語。
///   「これは「〜と証明した」ものではない」)
/// - 主語を囲む括弧が、述語より前で閉じる場合 (「「〜ことが大切」との見解を示した」の
///   「ことが」は、括弧の中の節の主語)
///
/// `before` は文頭から主語の手前まで、`between` は主語の後ろから述語の手前まで。
fn crosses_bracket(before: &str, between: &str) -> bool {
    fn step(open: &mut Vec<char>, c: char) -> bool {
        if let Some(close) = text::closing_bracket(c) {
            open.push(close);
        } else if open.last() == Some(&c) {
            open.pop();
            return true;
        }
        false
    }
    let mut open: Vec<char> = Vec::new();
    for c in before.chars() {
        step(&mut open, c);
    }
    let depth = open.len();
    for c in between.chars() {
        if step(&mut open, c) && open.len() < depth {
            return true;
        }
    }
    open.len() > depth
}

impl Rule for InanimateSubject {
    fn unit(&self) -> RuleUnit {
        RuleUnit::Sentence
    }

    fn meta(&self) -> &'static RuleMeta {
        &P13_META
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for (idx, block) in ctx.scoped_blocks() {
            for sentence in ctx.doc.block_sentences(idx) {
                let text = &block.text[sentence.range.clone()];
                for (range, subject, verb) in Self::find(text) {
                    let matched = &text[range.clone()];
                    let abs =
                        (sentence.range.start + range.start)..(sentence.range.start + range.end);
                    let message = format!(
                        "「{}」は無生物の主語「{}」を他動詞「{}」で結ぶ直訳調の構文です",
                        quote(matched),
                        subject.trim_end_matches(['は', 'が']),
                        verb
                    );
                    let item = p13_verb_item(&verb);
                    out.push(
                        diagnostic(
                            &P13_META,
                            block.to_source(abs),
                            sentence.span,
                            matched,
                            message,
                            "人や状況を主語に戻すか、「〜から分かる」「〜になっている」のような述べ方に変えてください",
                            P13_META.default_severity,
                            item_signal(&P13_META, P13_ITEM_SIGNALS, &item),
                        )
                        .with_metric("item", item),
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// P14 EM_DASH
// ---------------------------------------------------------------------------

static P14_META: RuleMeta = RuleMeta {
    id: "P14",
    name: "EM_DASH",
    title: "ダッシュの挿入句",
    lane: Lane::Slop,
    signal: Some(Signal::Weak),
    default_severity: Severity::Info,
    summary: "「〜——つまり〜——〜」のような、ダッシュで挟んだ挿入句を指摘する",
    explanation: r"### 何を見るか

日本語の文の中で、ダッシュ (「——」「―」「—」) を対にして補足を挟む挿入句と、日本語どうしの間に置かれた二倍ダッシュ「——」を探します。同じブロック内で対応する「」・『』・“”・二重引用符の内側に完全に収まる一致と、開き引用符の直後の一致は、語句への言及として除きます。意味による判定ではないため、書名の副題 (『〜——〜』) や、引用された発言で実際に使われたダッシュも指摘しません。

### なぜ問題か

英語の em dash による補足の挿入をそのまま移した書き方です。日本語の文にダッシュの挿入を挟むと、語順と息継ぎのリズムが途中で断ち切られ、翻訳調が目立ちます。

### 直し方

補足は括弧に入れるか、文を分けて書きます。

### 例

- 直す前: 作業は明日——予定どおりに進めば——終わる。
- 直した後: 作業は明日終わる (予定どおりに進めば)。

### 根拠

翻訳調の手癖として知られている書き方です。「東京—大阪」のような区間の表記と区別するため、単独のダッシュの対は、挟まれた部分にひらがなを含むときだけ挿入句とみなします。引用符の内側の一致は、人の文書 176 本で 4 件 (書名の副題と、引いた発言・独白の中のダッシュ)、生成文書 1,729 本で 0 件でした。

手掛かりの強さは「弱い」です。人の文書 440 本と生成文書 390 本で測ると、指摘が出た文書は人 14 本・生成 12 本でした。ダッシュの対で挟んだ挿入句は、項目だけで数えると件数が少なく (人 3 本・生成 1 本)、強さを測れません。",
};

fn is_dash(c: char) -> bool {
    matches!(c, '—' | '―' | '⸺' | '⸻' | '─')
}

/// ダッシュの連なり (テキスト上の範囲と字数)。
fn dash_runs(text: &str) -> Vec<(Range<usize>, usize)> {
    let mut runs = Vec::new();
    let mut current: Option<(usize, usize, usize)> = None; // (start, end, count)
    for (i, c) in text.char_indices() {
        if is_dash(c) {
            current = Some(match current {
                Some((s, e, n)) if e == i => (s, i + c.len_utf8(), n + 1),
                _ => {
                    if let Some((s, e, n)) = current.take() {
                        runs.push((s..e, n));
                    }
                    (i, i + c.len_utf8(), 1)
                }
            });
        } else if let Some((s, e, n)) = current.take() {
            runs.push((s..e, n));
        }
    }
    if let Some((s, e, n)) = current {
        runs.push((s..e, n));
    }
    runs
}

pub(super) struct EmDash;

/// P14 の項目: ダッシュの対で挟んだ挿入句。
const P14_PAIR_ITEM: &str = "ダッシュの対の挿入句";
/// P14 の項目: 日本語のあいだに置いた二倍ダッシュ (対になっていないもの)。
const P14_DOUBLE_ITEM: &str = "日本語のあいだの二倍ダッシュ";

impl EmDash {
    /// 挿入句 (ダッシュの対) と、日本語に挟まれた二倍ダッシュの範囲を、項目の名前と組にして返す。
    fn find(text: &str) -> Vec<(Range<usize>, &'static str)> {
        let runs = dash_runs(text);
        let mut out = Vec::new();
        let mut used = vec![false; runs.len()];
        for i in 0..runs.len().saturating_sub(1) {
            if used[i] {
                continue;
            }
            let (first, first_len) = &runs[i];
            let (second, second_len) = &runs[i + 1];
            let inner = &text[first.end..second.start];
            let inner_chars = inner.chars().count();
            let clause_like = inner.chars().any(text::is_hiragana);
            let doubled = *first_len >= 2 && *second_len >= 2;
            if !inner.trim().is_empty()
                && inner_chars <= 60
                && text::contains_japanese(inner)
                && (doubled || clause_like)
            {
                out.push((first.start..second.end, P14_PAIR_ITEM));
                used[i] = true;
                used[i + 1] = true;
            }
        }
        for (i, (run, len)) in runs.iter().enumerate() {
            if used[i] || *len < 2 {
                continue;
            }
            let before = text[..run.start].chars().next_back();
            let after = text[run.end..].chars().next();
            if before.is_some_and(text::is_japanese) && after.is_some_and(text::is_japanese) {
                out.push((run.clone(), P14_DOUBLE_ITEM));
            }
        }
        out.sort_by_key(|(r, _)| r.start);
        out
    }
}

impl Rule for EmDash {
    fn unit(&self) -> RuleUnit {
        RuleUnit::Sentence
    }

    fn meta(&self) -> &'static RuleMeta {
        &P14_META
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for (idx, block) in ctx.scoped_blocks() {
            let quotes = QuoteRanges::new(&block.text);
            for sentence in ctx.doc.block_sentences(idx) {
                let text = &block.text[sentence.range.clone()];
                if !text::contains_japanese(text) {
                    continue;
                }
                for (range, item) in Self::find(text) {
                    let matched = &text[range.clone()];
                    let abs =
                        (sentence.range.start + range.start)..(sentence.range.start + range.end);
                    if quotes.excludes(&abs) {
                        continue;
                    }
                    out.push(
                        diagnostic(
                            &P14_META,
                            block.to_source(abs),
                            sentence.span,
                            matched,
                            format!("「{}」はダッシュで挟んだ挿入句です", quote(matched)),
                            "補足は括弧に入れるか、文を分けてください",
                            P14_META.default_severity,
                            P14_META.signal,
                        )
                        .with_metric("item", item),
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// P20 COLON_CONTINUATION
// ---------------------------------------------------------------------------

static P20_META: RuleMeta = RuleMeta {
    id: "P20",
    name: "COLON_CONTINUATION",
    title: "述語とコロンでの列挙の導入",
    lane: Lane::Slop,
    signal: Some(Signal::Strong),
    default_severity: Severity::Info,
    summary: "「手順は以下の通りです：」のように、述語のあとにコロンを置いて箇条書きへつなぐ英語的な形を指摘する",
    explanation: r"### 何を見るか

段落が「以下の通りです：」「次の手順で設定します:」のように、述語 (です・ます・ください・通り など) にコロンを付けて終わり、直後に箇条書きが続く箇所を探します。「使い方：」のような名詞とコロンの見出し語は拾いません。

### なぜ問題か

文を言い終えたあとにコロンで続きを示すのは、英語の文章の組み方です。日本語では述語で文が閉じるので、そのあとのコロンは余分な記号になり、翻訳された文書や生成された文書に特有の見た目になります。

### 直し方

コロンを外して句点で文を閉じるか、「手順：」のような名詞のラベルに縮めます。前置きの文がなくても箇条書きの意味が通るなら、文ごと削ります。

### 例

- 直す前: 申請に必要な書類は以下の通りです：
- 直した後: 申請には次の書類が要る。

### 根拠

辞書を使わず、述語は段落の末尾のひらがなの語形 (です・ます・ください・通り・する など) で見分けています。

手掛かりの強さは「強い」です。人の文書 440 本と生成文書 390 本で測ると、指摘が出た文書は人 0 本・生成 22 本でした (「ます：」で 15 本、「です：」で 6 本)。",
};

/// 述語とみなす段落末の語形 (コロンの直前)。長いものを先に置く。
const PREDICATE_ENDINGS: &[&str] = &[
    "ください",
    "でした",
    "ました",
    "ません",
    "とおり",
    "通り",
    "です",
    "ます",
    "する",
    "なる",
    "ある",
    "いる",
    "れる",
];

pub(super) struct ColonContinuation;

impl ColonContinuation {
    /// 段落のテキストの末尾が「述語 + コロン」なら、述語の始まりから末尾までの範囲と、
    /// 述語の語形 (`PREDICATE_ENDINGS` の 1 つ。`noslop calibrate` の項目にする) を返す。
    fn find(text: &str) -> Option<(Range<usize>, &'static str)> {
        let trimmed = text.trim_end();
        let body = trimmed.strip_suffix([':', '：'])?.trim_end();
        let ending = PREDICATE_ENDINGS.iter().find(|e| body.ends_with(**e))?;
        Some(((body.len() - ending.len())..trimmed.len(), *ending))
    }
}

impl Rule for ColonContinuation {
    fn unit(&self) -> RuleUnit {
        RuleUnit::Sentence
    }

    fn meta(&self) -> &'static RuleMeta {
        &P20_META
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let blocks = &ctx.doc.blocks;
        for (idx, block) in ctx.scoped_blocks() {
            if block.kind != BlockKind::Paragraph {
                continue;
            }
            let next_is_list = blocks
                .get(idx + 1)
                .is_some_and(|b| b.kind == BlockKind::ListItem);
            if !next_is_list {
                continue;
            }
            let Some((range, ending)) = Self::find(&block.text) else {
                continue;
            };
            let context = ctx
                .doc
                .block_sentences(idx)
                .last()
                .map_or(block.span, |s| s.span);
            let matched = &block.text[range.clone()];
            out.push(
                diagnostic(
                    &P20_META,
                    block.to_source(range.clone()),
                    context,
                    matched,
                    format!(
                        "「{}」は述語のあとにコロンを置いて箇条書きへつなぐ英語的な形です",
                        quote(matched)
                    ),
                    "コロンを外して句点で文を閉じるか、「手順：」のような名詞のラベルに縮めてください",
                    P20_META.default_severity,
                    P20_META.signal,
                )
                .with_metric("item", format!("{ending}＋コロン")),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::testing::{matched, run};

    #[test]
    fn p13_flags_inanimate_subjects_with_conjugated_verbs() {
        let md = "この事実は、利用者の関心が移ったことを示している。\n";
        let d = run(&InanimateSubject, md);
        assert_eq!(
            matched(md, &d),
            vec!["この事実は、利用者の関心が移ったことを示し"]
        );
        assert!(d[0].message.contains("「この事実」"));
        assert!(d[0].message.contains("「示し」"));

        let md = "このことは、需要が伸びたことを意味する。\n";
        let d = run(&InanimateSubject, md);
        assert_eq!(
            matched(md, &d),
            vec!["このことは、需要が伸びたことを意味する"]
        );

        let md = "この事実は、前提が誤っていたことをもたらした。\n";
        let d = run(&InanimateSubject, md);
        assert_eq!(
            matched(md, &d),
            vec!["この事実は、前提が誤っていたことをもたらし"]
        );
    }

    #[test]
    fn p13_counts_nested_subjects_once() {
        let md = "そのことは市場の変化を反映している。\n";
        let d = run(&InanimateSubject, md);
        assert_eq!(d.len(), 1);
        assert!(matched(md, &d)[0].starts_with("そのことは"));
    }

    #[test]
    fn p13_does_not_mistake_compound_nouns_for_verbs() {
        for md in [
            "これは上司が指示した手順だ。\n",
            "それが画面に表示される。\n",
            "このことは先方に提示した。\n",
        ] {
            assert!(run(&InanimateSubject, md).is_empty(), "{md}");
        }
    }

    #[test]
    fn p13_counts_every_verb_and_signals_by_item() {
        // 「暗示する」も数え、ルールの強さ (中程度) を継ぐ。「示す」だけは強い手掛かり
        assert_eq!(P13_META.signal, Some(Signal::Moderate));
        let md = "この事実は、需要が戻らないことを暗示している。\n";
        let d = run(&InanimateSubject, md);
        assert_eq!(
            matched(md, &d),
            vec!["この事実は、需要が戻らないことを暗示し"]
        );
        assert_eq!(item(&d[0]), "暗示する");
        assert_eq!(d[0].signal, Some(Signal::Moderate));
        let md = "これは需要の減少を示している。それは変化をもたらした。\n";
        let d = run(&InanimateSubject, md);
        let items: Vec<&str> = d.iter().map(item).collect();
        assert_eq!(items, ["示す", "もたらす"]);
        let signals: Vec<_> = d.iter().map(|d| d.signal).collect();
        assert_eq!(signals, [Some(Signal::Strong), Some(Signal::Moderate)]);
        // 活用が違っても項目は「示す」なので、同じ強さになる
        let md = "このことは何も示さない。\n";
        assert_eq!(run(&InanimateSubject, md)[0].signal, Some(Signal::Strong));
    }

    #[test]
    fn p13_ignores_predicates_inside_brackets_opened_after_the_subject() {
        // 括弧の中の述語は引用や名詞修飾の中の述語で、主語「これ」の述語ではない
        for md in [
            "これは「手順が不要だと証明した」ものではない。\n",
            "それは（前提の誤りを示す）資料ではない。\n",
        ] {
            assert!(run(&InanimateSubject, md).is_empty(), "{md}");
        }
        // 括弧が閉じたあとの述語や、括弧ごと主語を含む文は数える
        let md = "これは「速さ」の問題を示している。\n";
        assert_eq!(
            matched(md, &run(&InanimateSubject, md)),
            vec!["これは「速さ」の問題を示し"]
        );
        let md = "彼は「これは誤りを示す」と言った。\n";
        assert_eq!(
            matched(md, &run(&InanimateSubject, md)),
            vec!["これは誤りを示す"]
        );
    }

    #[test]
    fn p13_ignores_subjects_whose_bracket_closes_before_the_predicate() {
        // 主語が括弧の中の節にあり、述語より前で括弧が閉じる文は、主語の述語ではない
        for md in [
            "経団連側は、「生産性を上げることが大切」との見解を示した。\n",
            "会長は「働き手の声を聞くことが重要」と述べ、労使の対話が要るとの認識を示した。\n",
            "「これは誤りだ」と前置きし、根拠を示した。\n",
            "（それが前提）と書いたうえで、手順を示した。\n",
        ] {
            assert!(run(&InanimateSubject, md).is_empty(), "{md}");
        }
        // 括弧の中で述語まで続く文と、括弧より前の主語の文は数える
        for (md, expected) in [
            (
                "会長は「このことは手順の誤りを示している」と述べた。\n",
                "このことは手順の誤りを示し",
            ),
            (
                "このことは「確認が大切」という考えを示している。\n",
                "このことは「確認が大切」という考えを示し",
            ),
            // 主語より前に閉じ括弧だけがある・主語のあとに閉じ括弧だけがある (注や箇条の番号)
            ("注1）これは誤りを示す。\n", "これは誤りを示す"),
            ("これは 2) の誤りを示す。\n", "これは 2) の誤りを示す"),
        ] {
            assert_eq!(
                matched(md, &run(&InanimateSubject, md)),
                vec![expected],
                "{md}"
            );
        }
    }

    #[test]
    fn p13_ignores_passive_predicates_but_keeps_negations() {
        // 未然形に「れ」が続く受け身は、主語が他動詞の主体ではない
        for md in [
            "調査の結果、利用者が増えたことが示された。\n",
            "睡眠を削ることが、集中力を下げると複数の研究で示されている。\n",
            "それがもたらされた経緯を調べる。\n",
            "このことは、新しい市場が生み出される前触れだ。\n",
        ] {
            assert!(run(&InanimateSubject, md).is_empty(), "{md}");
        }
        // 否定は主体のままなので数える
        let md = "このことは何も示さない。\n";
        let d = run(&InanimateSubject, md);
        assert_eq!(matched(md, &d), vec!["このことは何も示さ"]);
        assert_eq!(item(&d[0]), "示す");
        let md = "それは変化をもたらさない。\n";
        assert_eq!(
            matched(md, &run(&InanimateSubject, md)),
            vec!["それは変化をもたらさ"]
        );
        // 受け身を飛ばしても、同じ主語のあとの能動の述語は数える
        let md = "これは、前に示された結果を反映している。\n";
        let d = run(&InanimateSubject, md);
        assert_eq!(matched(md, &d), vec!["これは、前に示された結果を反映し"]);
        assert_eq!(item(&d[0]), "反映する");
    }

    #[test]
    fn p13_requires_the_verb_within_the_window() {
        let md = "これは、長い前置きのあとで、ようやく本題に入るまでにいくつもの段落を費やしてからようやく方針を示す。\n";
        assert!(run(&InanimateSubject, md).is_empty());
    }

    #[test]
    fn p13_ignores_human_subjects() {
        let md = "担当者が手順を示した。\n";
        assert!(run(&InanimateSubject, md).is_empty());
    }

    #[test]
    fn p13_flags_the_irrealis_form_of_umidasu() {
        // 未然形の「生み出さ」も、原形が「生み出す」の述語として拾う
        let md = "同じ議論を繰り返すことは、何も生み出さない。\n";
        let d = run(&InanimateSubject, md);
        assert_eq!(matched(md, &d), vec!["ことは、何も生み出さ"]);
        assert_eq!(item(&d[0]), "生み出す");
        assert!(d[0].message.contains("「生み出さ」"));
        // 既定ではリストを見ない
        let md = "- 同じ議論を繰り返すことは、何も生み出さない。\n";
        assert!(run(&InanimateSubject, md).is_empty());
    }

    #[test]
    fn p13_ignores_forms_outside_the_verb_list() {
        // 可能形の「生み出せる」は述語の一覧にない。人を主語にした文は主語の一覧にない
        for md in [
            "これは新しい価値を生み出せる。\n",
            "担当者は何も生み出さない。\n",
        ] {
            assert!(run(&InanimateSubject, md).is_empty(), "{md}");
        }
    }

    #[test]
    fn p14_flags_dash_parentheticals() {
        let md = "作業は明日——予定どおりに進めば——終わる。\n";
        let d = run(&EmDash, md);
        assert_eq!(matched(md, &d), vec!["——予定どおりに進めば——"]);
        let md = "結論は単純——速さだ。\n";
        assert_eq!(matched(md, &run(&EmDash, md)), vec!["——"]);
        let md = "指摘が出なくなるまで—つまり収束するまで—繰り返す。\n";
        assert_eq!(matched(md, &run(&EmDash, md)), vec!["—つまり収束するまで—"]);
    }

    #[test]
    fn p14_skips_dashes_quoted_as_mentions() {
        // 書名の副題・引用した発言の中のダッシュ・ダッシュそのものへの言及は除く
        for md in [
            "著書『贈与の論理——資本主義のすきま』に入れられなかった文章を載せる。\n",
            "会長は「人口減少と社会構造の転換——を柱に整理した」と述べた。\n",
            "「——そう思えば——気が楽だ。\n",
            "「〜——つまり〜——〜」のような挿入句を探す。\n",
        ] {
            assert!(run(&EmDash, md).is_empty(), "{md}");
        }
        // 引用の外のダッシュは、同じ文に引用があっても拾う
        let md = "作業は明日——予定どおりに進めば——終わる。「早い」と言われた。\n";
        assert_eq!(
            matched(md, &run(&EmDash, md)),
            vec!["——予定どおりに進めば——"]
        );
        let md = "結論は「速さ」で決まる——ほかは要らない。\n";
        assert_eq!(matched(md, &run(&EmDash, md)), vec!["——"]);
        // 閉じない引用符は、開き引用符の直後のダッシュだけを除く
        let md = "「前置き——ここから本題——続く。\n";
        assert_eq!(matched(md, &run(&EmDash, md)), vec!["——ここから本題——"]);
    }

    #[test]
    fn p14_ignores_ranges_and_english() {
        for md in [
            "東京—名古屋—大阪を結ぶ。\n",
            "It works—mostly—as expected.\n",
        ] {
            assert!(run(&EmDash, md).is_empty(), "{md}");
        }
    }

    #[test]
    fn p20_flags_predicate_colon_before_a_list() {
        let md = "申請に必要な書類は以下の通りです：\n\n- 申請書\n- 領収書\n";
        let d = run(&ColonContinuation, md);
        assert_eq!(matched(md, &d), vec!["です："]);
        assert_eq!(d[0].signal, Some(Signal::Strong));
        let ctx = d[0].context.expect("context");
        assert_eq!(&md[ctx.range()], "申請に必要な書類は以下の通りです：");

        // リストが段落に続けて書かれていても (空行なし) 拾う
        let md = "次の手順で設定します:\n- 画面を開く\n- 保存する\n";
        assert_eq!(matched(md, &run(&ColonContinuation, md)), vec!["ます:"]);

        let md = "書類は次のとおり：\n\n- 申請書\n";
        assert_eq!(matched(md, &run(&ColonContinuation, md)), vec!["とおり："]);
    }

    #[test]
    fn p20_ignores_noun_labels_and_colons_not_followed_by_lists() {
        for md in [
            "使い方：\n\n- 画面を開く\n",
            "ポイント:\n\n- 速い\n",
            "申請書は以下の通りです：\n\n本文の段落が続く。\n",
            "手順を説明します。\n\n- 画面を開く\n",
        ] {
            assert!(run(&ColonContinuation, md).is_empty(), "{md}");
        }
    }

    #[test]
    fn p20_respects_scope() {
        // リストの中の段落は既定では見ない
        let md = "- 項目は以下の通りです：\n  - 子の項目\n";
        assert!(run(&ColonContinuation, md).is_empty());
    }

    fn item(d: &Diagnostic) -> &str {
        match d.metrics.get("item") {
            Some(crate::diagnostic::Metric::Text(s)) => s,
            other => panic!("item がありません: {other:?}"),
        }
    }

    #[test]
    fn items_group_hits_by_predicate_or_kind() {
        // P13 は述語の活用をそろえた形 (校正で述語ごとに比べるため)
        let md =
            "この事実は、関心が移ったことを示している。そのことは、需要の変化を示唆している。\n";
        let d = run(&InanimateSubject, md);
        let items: Vec<&str> = d.iter().map(item).collect();
        assert_eq!(items, vec!["示す", "示唆する"]);

        // P14 は挿入句の対と、対になっていない二倍ダッシュを分ける。二倍ダッシュは測った強さが
        // ルールと同じで、対の挿入句は件数が少なく測れないので、どちらもルールの強さ (弱い) を継ぐ
        let md = "作業は明日——予定どおりに進めば——終わる。準備は万全——だと思う。\n";
        let d = run(&EmDash, md);
        let items: Vec<&str> = d.iter().map(item).collect();
        assert_eq!(items, vec![P14_PAIR_ITEM, P14_DOUBLE_ITEM]);
        let signals: Vec<_> = d.iter().map(|d| d.signal).collect();
        assert_eq!(signals, [Some(Signal::Weak), Some(Signal::Weak)]);

        // P20 は述語の語形
        let md = "申請に必要な書類は以下の通りです：\n\n- 申請書\n";
        let d = run(&ColonContinuation, md);
        assert_eq!(item(&d[0]), "です＋コロン");
    }

    #[test]
    fn every_p13_verb_form_has_an_item() {
        let forms = [
            "もたらす",
            "もたらし",
            "もたらさ",
            "示唆する",
            "示唆し",
            "暗示する",
            "暗示し",
            "示す",
            "示し",
            "示さ",
            "意味する",
            "意味し",
            "証明する",
            "証明し",
            "生み出す",
            "生み出し",
            "生み出さ",
            "反映する",
            "反映し",
            "物語る",
            "物語っ",
            "浮き彫りにする",
            "浮き彫りにし",
            "後押しする",
            "後押しし",
        ];
        let items: Vec<&str> = P13_VERB_ITEMS.iter().map(|(_, item)| *item).collect();
        for form in forms {
            let m = VERB_RE
                .find(form)
                .unwrap_or_else(|| panic!("{form} が一致しない"));
            assert_eq!(m.as_str(), form, "{form} の全体が一致する");
            let item = p13_verb_item(form);
            assert!(items.contains(&item.as_str()), "{form} → {item}");
            assert_eq!(form.chars().next(), item.chars().next(), "{form} → {item}");
        }
        assert_eq!(
            p13_verb_item("示唆し"),
            "示唆する",
            "「示唆」を「示」と取り違えない"
        );
    }
}
