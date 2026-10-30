//! 人・組織・担当の語 (担当者・管理者・私たちなど) を、主語や連絡・依頼の相手らしい位置から
//! 取り出す。
//!
//! 改稿で原文にない人や担当を主語・依頼先に足していないかを確かめる手掛かりにする。誤った通知を
//! 出さないことを優先し、語・位置・述語を閉じた一覧に絞る (取り逃しは許す)。
//!
//! - 語の直前: ブロックか文の先頭・句読点。間の半角・全角の空白と、文の頭の「・」は飛ばす。
//!   複合語の末尾 (システム管理者) と修飾句の中 (営業の担当者) からは取らない。改行は、文分割が
//!   文の区切りとしたときだけ文の頭になる (折り返しただけの改行は境界にしない)
//! - 主体・話題: 語の直後が「は」「が」「も」「では」
//! - 連絡・依頼の相手: 語に「に」「へ」(と任意の「は」「も」) が続き、その直後か、短い目的語
//!   (漢字・カタカナ・英数字の 12 字以内と「を」) の直後に、知らせる・通知する・連絡する・
//!   報告する・依頼する・頼む・共有する・伝える・相談する・確認する の活用形が来る
//! - 語から述語までが 1 文に収まり、引用符の内側とコード・画像・数式・取り消し線に重ならない。
//!   引用ブロックの中からは取らない
//!
//! 文書のどこかに同じ語 (表記の揺れと言い換えを含む) が出るかは、位置を問わずに調べる
//! ([`presence`])。増減は、相手の文書のどこにも同じ語がないときだけ出す。

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::LazyLock;

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};

use super::{Fact, FactKind};
use crate::document::{Block, Document, MarkKind};
use crate::text::{self, quotes::QuoteRanges};

/// 照合キーと表記。同じキーの表記は同じ語として照合する。
///
/// 同じ人と認めるためではなく、新しく足したという通知を控えるための照合。そのため表記の揺れに
/// 加えて、同じものを指す言い換え (私たち／我々、当社／弊社、上司／上長) もまとめる。改稿では、
/// 文体に合わせてこれらを入れ替えることが多い。
const ACTORS: &[(&str, &[&str])] = &[
    ("担当", &["担当者", "担当"]),
    ("管理者", &["管理者", "アドミン"]),
    ("責任者", &["責任者"]),
    ("事務局", &["事務局"]),
    ("上司", &["上司", "上長"]),
    (
        "私たち",
        &["わたしたち", "われわれ", "私たち", "私達", "我々"],
    ),
    ("当社", &["当社", "弊社"]),
    ("皆さん", &["皆さん", "みなさん"]),
];

/// 語の直前の境界を確かめるときに飛ばす空白。
const SPACES: [char; 2] = [' ', '\u{3000}'];

/// 語の直前に来てよい句読点。
const PUNCTUATION: [char; 8] = ['、', '。', '，', '．', '！', '？', '!', '?'];

/// 主体・話題の助詞 (長いものを先に並べる)。
const TOPIC_PARTICLES: [&str; 4] = ["では", "は", "が", "も"];

/// 助詞から始まるが助詞ではない語の頭。
const NOT_PARTICLES: [&str; 3] = ["もしくは", "もどき", "がらみ"];

/// 短い目的語の字数の上限 (「を」を除く)。
const MAX_OBJECT_CHARS: usize = 12;

/// 述語の語幹の後に続いてよい形と、その直後に来てよい字 (空なら問わない。文が終わってもよい)。
///
/// 「し」「す」のような短い形は、直後の字で活用の続きだと確かめる (「通知しきい値」「確認しか」
/// を述語にしない)。
type Forms = &'static [(&'static str, &'static str)];

/// サ変の述語 (通知する・報告するなど) の形。
const SURU_FORMS: Forms = &[
    ("し", "てたまなよつ、，"),
    ("す", "るれべ"),
    ("させ", "るてたまなよれら"),
    ("され", "るてたまなよず"),
    ("せず", ""),
    ("せよ", ""),
    ("いたし", "てたまな"),
    ("致し", "てたまな"),
    ("いただ", "くきけい"),
    ("頂", "くきけい"),
    ("ください", ""),
    ("下さい", ""),
    ("願", "いうえ"),
    ("お願い", ""),
    ("をし", "てたまなよ"),
    ("をす", "るれべ"),
    ("をお願い", ""),
    ("をいただ", "くきけい"),
    ("を頂", "くきけい"),
    ("を依頼", ""),
    ("を頼", "むんまみめ"),
];

/// 一段の述語 (知らせる・伝える) の形。
const ICHIDAN_FORMS: Forms = &[
    ("る", ""),
    ("れ", ""),
    ("ろ", ""),
    ("よ", ""),
    ("た", ""),
    ("て", ""),
    ("ま", ""),
    ("な", ""),
    ("ず", ""),
    ("ら", ""),
    ("させ", ""),
];

/// 「お知らせ」「お伝え」の形。
const HONORIFIC_FORMS: Forms = &[
    ("ください", ""),
    ("下さい", ""),
    ("し", "てたまなよ"),
    ("す", "るれべ"),
    ("いたし", "てたまな"),
    ("致し", "てたまな"),
    ("いただ", "くきけい"),
    ("頂", "くきけい"),
    ("願", "いうえ"),
];

/// 「頼む」の形 (「頼る」「頼みごと」「頼もしい」を取らない)。
const TANOMU_FORMS: Forms = &[
    ("む", ""),
    ("んで", ""),
    ("んだ", ""),
    ("まな", ""),
    ("まず", ""),
    ("まれ", ""),
    ("ませ", ""),
    ("みま", ""),
    ("みた", ""),
    ("みな", ""),
    ("めば", ""),
    ("める", ""),
    ("めま", ""),
    ("もう", ""),
];

/// 連絡・依頼の述語の語幹と、続いてよい形。語幹だけの名詞 (報告書・連絡先・確認画面) は取らない。
const PREDICATES: &[(&str, Forms)] = &[
    ("通知", SURU_FORMS),
    ("連絡", SURU_FORMS),
    ("報告", SURU_FORMS),
    ("依頼", SURU_FORMS),
    ("共有", SURU_FORMS),
    ("相談", SURU_FORMS),
    ("確認", SURU_FORMS),
    ("知らせ", ICHIDAN_FORMS),
    ("伝え", ICHIDAN_FORMS),
    ("頼", TANOMU_FORMS),
];

/// 表記の照合器と、表記ごとの照合キー。同じ位置からは長い表記 (担当者) を先に取る。
static SURFACES: LazyLock<(AhoCorasick, Vec<&'static str>)> = LazyLock::new(|| {
    let (keys, surfaces): (Vec<&str>, Vec<&str>) = ACTORS
        .iter()
        .flat_map(|&(key, surfaces)| surfaces.iter().map(move |&s| (key, s)))
        .unzip();
    let matcher = AhoCorasickBuilder::new()
        .match_kind(MatchKind::LeftmostLongest)
        .build(&surfaces)
        .expect("actor surfaces");
    (matcher, keys)
});

/// 文書のどこか (原文と、各ブロックの解析用テキスト) に出る語の照合キー。位置は問わない。
///
/// 原文では太字の記号で分かれる語 (「**管理**者」) も表示上は 1 語なので、ブロックの解析用テキストも
/// 調べる。ブロックどうしはつながない (別のセルの「管理」と「者」を 1 語にしない)。
pub(super) fn presence(doc: &Document) -> BTreeSet<&'static str> {
    let texts: Vec<&str> = std::iter::once(doc.source.as_str())
        .chain(doc.blocks.iter().map(|b| b.text.as_str()))
        .collect();
    ACTORS
        .iter()
        .filter(|(_, surfaces)| {
            surfaces
                .iter()
                .any(|s| texts.iter().any(|text| text.contains(s)))
        })
        .map(|&(key, _)| key)
        .collect()
}

/// ブロックから、主語や連絡・依頼の相手の位置にある語を取り出す。
pub(super) fn extract_block(doc: &Document, block: &Block, out: &mut Vec<Fact>) {
    if block.in_quote {
        return;
    }
    let text = block.text.as_str();
    let sentences = &doc.sentences[block.sentences.clone()];
    let mut quotes = None;
    let (matcher, keys) = &*SURFACES;
    for m in matcher.find_iter(text) {
        let word = m.range();
        let key = keys[m.pattern().as_usize()];
        let Some(sentence) = sentences
            .iter()
            .find(|s| s.range.start <= word.start && word.end <= s.range.end)
        else {
            continue;
        };
        if !starts_clause(text, sentence.range.start, word.start) {
            continue;
        }
        let Some(len) = role(&text[word.end..sentence.range.end], key) else {
            continue;
        };
        let evidence = word.start..word.end + len;
        if quotes
            .get_or_insert_with(|| QuoteRanges::new(text))
            .overlaps(&evidence)
            || hidden(block, &evidence)
        {
            continue;
        }
        out.push(Fact {
            kind: FactKind::Actor,
            text: text[word.clone()].to_string(),
            key: key.to_string(),
            span: block.to_source(word),
        });
    }
}

/// 語が節の頭から始まるか: ブロックか文の先頭、句読点の直後。間の半角・全角の空白と、文の頭に
/// 置いた「・」は飛ばす。
///
/// 改行は、文分割が文の区切りとしたとき (1 行 1 項目の行など) だけ文の頭になる。文の途中で
/// 折り返しただけの改行 (「営業の」の後の改行) は境界にしない。折り返す位置で結果が変わらないように。
fn starts_clause(text: &str, sentence_start: usize, start: usize) -> bool {
    let head = |at: usize| at == 0 || at == sentence_start;
    let before = text[..start].trim_end_matches(SPACES);
    if start == sentence_start || head(before.len()) || before.ends_with(PUNCTUATION) {
        return true;
    }
    before
        .strip_suffix('・')
        .is_some_and(|rest| head(rest.trim_end_matches(SPACES).len()))
}

/// 語の後ろが主体・話題か連絡・依頼の相手の形なら、その形の長さ (バイト) を返す。
fn role(tail: &str, key: &str) -> Option<usize> {
    topic(tail, key).or_else(|| addressee(tail))
}

/// 主体・話題の助詞の長さ。助詞から始まる別の語 (もしくは・もどき・がらみ・皆さんがた) は除く。
fn topic(tail: &str, key: &str) -> Option<usize> {
    if NOT_PARTICLES.iter().any(|p| tail.starts_with(p))
        || (key == "皆さん" && tail.starts_with("がた"))
    {
        return None;
    }
    TOPIC_PARTICLES
        .iter()
        .find(|p| tail.starts_with(*p))
        .map(|p| p.len())
}

/// 語を連絡・依頼の相手とする形の長さ: 「に」「へ」(と「は」「も」)、短い目的語と「を」(任意)、述語。
fn addressee(tail: &str) -> Option<usize> {
    let mut at = ["に", "へ"].iter().find(|p| tail.starts_with(*p))?.len();
    at += ["は", "も"]
        .iter()
        .find(|p| tail[at..].starts_with(*p))
        .map_or(0, |p| p.len());
    if let Some(len) = predicate(&tail[at..]) {
        return Some(at + len);
    }
    at += object(&tail[at..])?;
    Some(at + predicate(&tail[at..])?)
}

/// 短い目的語と「を」の長さ。目的語は漢字・カタカナ・英数字だけの 1〜12 字 (句読点・空白・
/// ひらがなを含む句や、別の節は目的語にしない)。
fn object(tail: &str) -> Option<usize> {
    for (count, (at, c)) in tail.char_indices().enumerate() {
        if c == 'を' && count > 0 {
            return Some(at + c.len_utf8());
        }
        let allowed = text::is_kanji(c)
            || text::is_katakana(c)
            || c.is_ascii_alphanumeric()
            || matches!(c, '０'..='９' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ');
        if count == MAX_OBJECT_CHARS || !allowed {
            return None;
        }
    }
    None
}

/// 連絡・依頼の述語の長さ。前に付いた「ご」「お」と、形に続く活用の残り (ひらがな) も含める
/// (引用・取り消し線などとの重なりを、述語の終わりまでで確かめるため)。
fn predicate(tail: &str) -> Option<usize> {
    let (prefix, rest) = ["ご", "お"]
        .iter()
        .find_map(|p| tail.strip_prefix(p).map(|rest| (p.len(), rest)))
        .unwrap_or((0, tail));
    PREDICATES.iter().find_map(|&(stem, forms)| {
        let after = rest.strip_prefix(stem)?;
        let honorific: Forms = if prefix > 0 && forms == ICHIDAN_FORMS {
            HONORIFIC_FORMS
        } else {
            &[]
        };
        let form = forms.iter().chain(honorific).find_map(|&(form, next)| {
            let following = after.strip_prefix(form)?.chars().next();
            (next.is_empty() || following.is_none_or(|c| next.contains(c))).then_some(form.len())
        })?;
        let inflection: usize = after[form..]
            .chars()
            .take_while(|&c| text::is_hiragana(c))
            .map(char::len_utf8)
            .sum();
        Some(prefix + stem.len() + form + inflection)
    })
}

/// コード・画像・数式・取り消し線に重なるか (表示されない、または消した文言)。
fn hidden(block: &Block, range: &Range<usize>) -> bool {
    block.marks.iter().any(|m| {
        matches!(
            m.kind,
            MarkKind::Code | MarkKind::Image | MarkKind::Math | MarkKind::Strikethrough
        ) && m.range.start < range.end
            && range.start < m.range.end
    })
}
