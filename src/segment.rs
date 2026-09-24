//! 日本語の文分割。
//!
//! 分割は [hasami](https://github.com/owayo/hasami) の辞書を使わない文分割
//! (`hasami::sentence`) で行い、ここでは noslop の文書モデルとの橋渡しだけをする。
//! 規則の要点は次のとおり (詳細は hasami の文書にある)。
//!
//! - 文末記号 (`。！？!?‼⁇⁈⁉．｡`) の連続を 1 つの文末とみなす
//! - 括弧類 (「」『』（）【】など) の内側の文末記号では分割しない。括弧は先に対応を取り、
//!   対応の取れた組だけを「分割しない範囲」にするので、閉じ忘れた括弧が後続の文を巻き込まない
//! - ASCII の `!` `?` は、直後が英数字・ASCII 記号なら文末にしない (URL の `?id=1`)
//! - 数字に挟まれた全角ピリオドは文末にしない (`３．１４`)
//! - 組み込みの例外表に載っている語の内側では分割しない (`Yahoo!ニュース`)
//!
//! 段落内の改行は、既定では文の区切りにしない (Markdown の折り返しは見た目上の改行で、
//! 一文一行の文書は改行の直前に句点がある)。句点を打たずに一文一行で書く文書向けに
//! [`LineBreakMode::Sentence`] を用意している。解析用テキストには改行の字が残らず、位置だけを
//! [`crate::document::Block::line_breaks`] に持つので、このモードではその位置を改行とみなして
//! 分割する (`Splitter::split_with_breaks`)。

use std::ops::Range;
use std::sync::LazyLock;

use hasami::sentence::Splitter;
use serde::Deserialize;

/// 段落内の改行の扱い。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineBreakMode {
    /// 改行は文の区切りにしない (既定)。
    #[default]
    Space,
    /// 改行を文の区切りにする (句点を打たない一文一行の文書向け)。
    Sentence,
}

/// 分割した 1 文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    /// テキスト上の範囲 (前後の空白を除く)。
    pub range: Range<usize>,
    /// 括弧の内側に文末記号を含むか。
    pub embedded_enders: bool,
}

/// 分割器 (改行の字では区切らず、組み込みの例外表を使う既定の設定)。
static SPLITTER: LazyLock<Splitter> = LazyLock::new(Splitter::default);

/// テキストを文に分割する。
///
/// `line_breaks` は原文の改行があった位置 (バイトオフセット)。`mode` が
/// [`LineBreakMode::Sentence`] のときだけ、括弧の外側にある改行を文の区切りにする。
pub fn split(text: &str, line_breaks: &[usize], mode: LineBreakMode) -> Vec<Piece> {
    let sentences = match mode {
        LineBreakMode::Space => SPLITTER.split(text),
        LineBreakMode::Sentence => {
            // 文書モデルが作る位置は文字の境界にあるが、hasami は境界でない位置で panic するので、
            // 崩れていても分割を止めないよう読み飛ばす
            let breaks: Vec<usize> = line_breaks
                .iter()
                .copied()
                .filter(|&at| text.is_char_boundary(at))
                .collect();
            SPLITTER.split_with_breaks(text, &breaks)
        }
    };
    sentences
        .into_iter()
        .map(|s| Piece {
            range: s.range,
            embedded_enders: s.embedded_enders,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(text: &str) -> Vec<&str> {
        split(text, &[], LineBreakMode::Space)
            .into_iter()
            .map(|p| &text[p.range])
            .collect()
    }

    fn texts_with<'a>(text: &'a str, line_breaks: &[usize], mode: LineBreakMode) -> Vec<&'a str> {
        split(text, line_breaks, mode)
            .into_iter()
            .map(|p| &text[p.range])
            .collect()
    }

    #[test]
    fn splits_on_enders() {
        assert_eq!(
            texts("これが最初の文。これは二番目の文。これが最後の文。"),
            vec!["これが最初の文。", "これは二番目の文。", "これが最後の文。"]
        );
    }

    #[test]
    fn keeps_runs_of_enders_together() {
        assert_eq!(
            texts("え、本当…！？嘘だろ…"),
            vec!["え、本当…！？", "嘘だろ…"]
        );
    }

    #[test]
    fn does_not_split_inside_brackets() {
        assert_eq!(
            texts("「うまく行くかな？」と思った。次の文。"),
            vec!["「うまく行くかな？」と思った。", "次の文。"]
        );
        assert_eq!(
            texts("注意（詳細は後述。）を読む。"),
            vec!["注意（詳細は後述。）を読む。"]
        );
    }

    #[test]
    fn marks_embedded_enders() {
        let pieces = split(
            "彼は「行こう。」と言った。そうだ。",
            &[],
            LineBreakMode::Space,
        );
        assert!(pieces[0].embedded_enders);
        assert!(!pieces[1].embedded_enders);
    }

    #[test]
    fn question_mark_of_a_url_in_brackets_is_not_an_embedded_ender() {
        let pieces = split(
            "資料（https://example.com/?q=1）を読む。",
            &[],
            LineBreakMode::Space,
        );
        assert_eq!(pieces.len(), 1);
        assert!(!pieces[0].embedded_enders);
    }

    #[test]
    fn unmatched_open_bracket_does_not_swallow_the_rest() {
        assert_eq!(
            texts("1) 手順を読む。「閉じ忘れ。次の文。"),
            vec!["1) 手順を読む。", "「閉じ忘れ。", "次の文。"]
        );
    }

    #[test]
    fn stray_closing_bracket_after_ender_stays_with_sentence() {
        assert_eq!(texts("終わりだ。」次だ。"), vec!["終わりだ。」", "次だ。"]);
    }

    #[test]
    fn ascii_question_mark_in_url_is_not_an_ender() {
        assert_eq!(
            texts("https://example.com/?q=1 を開く。本当?すごい。"),
            vec!["https://example.com/?q=1 を開く。", "本当?", "すごい。"]
        );
    }

    #[test]
    fn english_sentences_split_on_ascii_enders() {
        assert_eq!(texts("Really? Yes!"), vec!["Really?", "Yes!"]);
    }

    #[test]
    fn words_in_the_exception_table_keep_their_enders() {
        assert_eq!(
            texts("Yahoo!ニュースで読んだ。次の文。"),
            vec!["Yahoo!ニュースで読んだ。", "次の文。"]
        );
        // 語の末尾の文末記号は、直後が助詞のときだけ分割しない
        assert_eq!(
            texts("モーニング娘。のライブに行った。好きなのはモーニング娘。次の話。"),
            vec![
                "モーニング娘。のライブに行った。",
                "好きなのはモーニング娘。",
                "次の話。"
            ]
        );
    }

    #[test]
    fn line_breaks_are_ignored_by_default_and_split_in_sentence_mode() {
        // 解析用テキストには改行の字が残らない。日本語どうしは詰め、英数字の間は空白でつなぐ
        let text = "一行目の途中で折り返して続く文。二文目 is here";
        let lb = [text.find("続く").unwrap(), text.find(" is").unwrap()];
        assert_eq!(
            texts_with(text, &lb, LineBreakMode::Space),
            vec!["一行目の途中で折り返して続く文。", "二文目 is here"]
        );
        assert_eq!(
            texts_with(text, &lb, LineBreakMode::Sentence),
            vec!["一行目の途中で折り返して", "続く文。", "二文目", "is here"]
        );
    }

    #[test]
    fn line_breaks_inside_brackets_do_not_split() {
        let text = "「前半を書いて後半を書く」と言った";
        let lb = [text.find("後半").unwrap()];
        assert_eq!(
            texts_with(text, &lb, LineBreakMode::Sentence),
            vec!["「前半を書いて後半を書く」と言った"]
        );
    }

    #[test]
    fn line_breaks_at_the_edges_repeated_or_unordered_are_accepted() {
        let text = "一文目。二文目";
        let mid = text.find("二").unwrap();
        assert_eq!(
            texts_with(text, &[text.len(), mid, 0, mid], LineBreakMode::Sentence),
            vec!["一文目。", "二文目"]
        );
    }

    #[test]
    fn broken_line_break_positions_do_not_panic() {
        let text = "一文目。二文目";
        // 文字の途中・範囲外の位置は読み飛ばす
        let lb = [1, text.find("二").unwrap(), 100];
        assert_eq!(
            texts_with(text, &lb, LineBreakMode::Sentence),
            vec!["一文目。", "二文目"]
        );
    }

    #[test]
    fn many_unmatched_brackets_are_handled_in_linear_time() {
        // 開き括弧が大量に残ったまま、対応しない閉じ括弧が大量に続く
        let text = format!("{}{}。次の文。", "(".repeat(50_000), "]".repeat(50_000));
        let pieces = split(&text, &[], LineBreakMode::Space);
        assert_eq!(pieces.len(), 2);
        assert!(!pieces[0].embedded_enders);

        // 崩れた入れ子でも、同じ種類の開き括弧まで遡って対応を取る
        assert_eq!(
            texts("「前（中。」後。次の文。"),
            vec!["「前（中。」後。", "次の文。"]
        );
    }

    #[test]
    fn full_width_period_is_an_ender() {
        assert_eq!(texts("第一文．第二文．"), vec!["第一文．", "第二文．"]);
    }

    #[test]
    fn empty_and_whitespace_only_text_yields_nothing() {
        assert!(texts("").is_empty());
        assert!(texts("   ").is_empty());
    }

    #[test]
    fn short_exception_words_do_not_merge_ordinary_sentences() {
        // 例外表の短い語 (「べる。」「すぎ。」) と同じ形の文末でも、次の文とつなげない
        assert_eq!(
            texts("高すぎ。でも買った。"),
            vec!["高すぎ。", "でも買った。"]
        );
        assert_eq!(
            texts("意見を述べる。では次に進む。"),
            vec!["意見を述べる。", "では次に進む。"]
        );
    }

    #[test]
    fn decimal_points_and_full_width_exception_words_are_kept() {
        assert_eq!(
            texts("円周率は３．１４です。次の文。"),
            vec!["円周率は３．１４です。", "次の文。"]
        );
        assert_eq!(
            texts("Ｙａｈｏｏ！ニュースを見た。次の文。"),
            vec!["Ｙａｈｏｏ！ニュースを見た。", "次の文。"]
        );
    }

    /// 組み込みの例外表が変わると文の数が変わり、文長などの統計 (校正の前提) が変わる。
    ///
    /// hasami を上げてここが落ちたら、手元の文書で分割の差分 (区切りが増えた・消えた箇所) を
    /// 確かめてから値を書き換える。例外表の出典の表示が変わっていないかも確かめ、変わっていれば
    /// THIRD_PARTY_NOTICES.md の写しを直す。
    #[test]
    fn builtin_exception_table_version_is_pinned() {
        assert_eq!(
            hasami::sentence::BUILTIN_EXCEPTIONS_VERSION,
            "18918-1da7a834bf345567"
        );
    }
}
