//! 見出しの形 (コロン型・問い型・番号型) の分類。
//!
//! 生成された文書は、見出しを 1 つの型に流し込みがちになる (「背景: 〜」「課題: 〜」、
//! 「〜とは？」の連続など)。S07 は文書の中での型の偏りを、`noslop diff` は改稿で型が
//! 1 つにそろったかを見るので、分類はここに 1 つだけ置く。

use std::sync::LazyLock;

use regex::Regex;

use crate::text;

/// 番号型の見出し: 「1.」「1.2 」「第1」「第三章」「ステップ1」「①」「(1)」で始まる。
///
/// 「第三者」のような語を拾わないよう、漢数字の「第〜」は章・節などが続くときだけにする。
static NUMBERED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)^(?:",
        r"[0-9０-９]+(?:[.．\-－][0-9０-９]+)*[.．)）、:：]",
        r"|[0-9０-９]+(?:[.．\-－][0-9０-９]+)+(?:\s|$)",
        r"|第\s*[0-9０-９]+",
        r"|第\s*[一二三四五六七八九十百]+\s*[章節部回条項段歩]",
        r"|(?:step|phase|ステップ|フェーズ|ステージ)\s*[0-9０-９一二三四五六七八九十]",
        r"|[①-⑳]",
        r"|[(（][0-9０-９]+[)）]",
        r")"
    ))
    .expect("numbered heading regex")
});

/// コロン型の見出し: 「X: Y」「X：Y」。URL の `://` は除く。
static COLON_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^:：]{1,30}[:：]\s*[^\s/]").expect("colon heading regex"));

/// 問い型の見出しの終わり方。
static QUESTION_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:[？?]|とは|のか|何か|でしょうか)$").expect("question heading regex")
});

/// 見出しの形。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeadingShape {
    /// 「背景: 電話が〜」のようにコロンでラベルと本文を分ける。
    Colon,
    /// 「〜とは？」「なぜ〜のか」のような問い。
    Question,
    /// 「1. 〜」「第 2 章」「ステップ 3」のような番号つき。
    Numbered,
    /// どの型でもない見出し。
    Plain,
}

impl HeadingShape {
    /// 偏りを数える型 (どの型でもない見出しは除く)。
    pub const TEMPLATES: [HeadingShape; 3] = [Self::Colon, Self::Question, Self::Numbered];

    /// 見出しの本文を分類する。番号型・コロン型・問い型の順に当てる。
    pub fn of(heading: &str) -> Self {
        let heading = heading.trim();
        if NUMBERED_RE.is_match(heading) {
            return Self::Numbered;
        }
        if COLON_RE.is_match(heading) {
            return Self::Colon;
        }
        let body =
            heading.trim_end_matches(|c: char| c.is_whitespace() || text::is_closing_bracket(c));
        if QUESTION_RE.is_match(body) {
            return Self::Question;
        }
        Self::Plain
    }

    /// JSON などに出す識別子。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Colon => "colon",
            Self::Question => "question",
            Self::Numbered => "numbered",
            Self::Plain => "plain",
        }
    }

    /// 利用者向けの名前。
    pub fn label(self) -> &'static str {
        match self {
            Self::Colon => "「X: Y」のコロン型",
            Self::Question => "「〜とは」「〜？」の問い型",
            Self::Numbered => "「1.」「ステップ1」の番号型",
            Self::Plain => "型のない見出し",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_heading_shapes() {
        use HeadingShape as H;
        for (heading, shape) in [
            ("1. 概要", H::Numbered),
            ("1.2 詳しい手順", H::Numbered),
            ("第1章 背景", H::Numbered),
            ("第 2 章 設計", H::Numbered),
            ("第三章 課題", H::Numbered),
            ("ステップ2: 設定", H::Numbered),
            ("Phase 1 準備", H::Numbered),
            ("①準備", H::Numbered),
            ("(3) 注意", H::Numbered),
            ("背景: なぜ今なのか", H::Colon),
            ("設定：環境変数", H::Colon),
            ("C#: 入門", H::Colon),
            ("RAGとは", H::Question),
            ("なぜ遅いのか", H::Question),
            ("何が変わる？", H::Question),
            ("「なぜ手入力が残るのか」", H::Question),
            ("2024年の振り返り", H::Plain),
            ("3 つの理由", H::Plain),
            ("第三者の視点", H::Plain),
            ("https://example.com の使い方", H::Plain),
            ("導入", H::Plain),
        ] {
            assert_eq!(HeadingShape::of(heading), shape, "{heading}");
        }
    }
}
