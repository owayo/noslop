//! 文字種の判定と、文長の数え方。
//!
//! 形態素解析を使わない (Zero-Dictionary) ため、判定はすべて Unicode の文字範囲で行う。

/// インラインコード・数式・画像など、解析テキストで 1 文字ぶんの「語」に置き換えた箇所。
///
/// 原文の文字列は文長にも語句マッチにも寄与させたくないが、文の中に何かが
/// あったこと自体は残したい (「`foo` を実行する」の主語が消えないように) ため、
/// オブジェクト置換文字 1 字に畳む。
pub const PLACEHOLDER: char = '\u{FFFC}';

/// ひらがな (長音符・踊り字を含む)。
pub fn is_hiragana(c: char) -> bool {
    matches!(c, '\u{3041}'..='\u{309F}') || c == 'ー'
}

/// カタカナ (半角カタカナ・長音符・中黒を除く記号は含めない)。
pub fn is_katakana(c: char) -> bool {
    matches!(c, '\u{30A1}'..='\u{30FA}' | '\u{30FD}'..='\u{30FF}' | '\u{31F0}'..='\u{31FF}' | '\u{FF66}'..='\u{FF9D}')
        || c == 'ー'
}

/// 漢字 (CJK 統合漢字・拡張 A・互換漢字・々〆ヶ)。
pub fn is_kanji(c: char) -> bool {
    matches!(
        c,
        '\u{4E00}'..='\u{9FFF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2FA1F}'
            | '々'
            | '〆'
            | 'ヶ'
    )
}

/// 日本語の文字 (ひらがな・カタカナ・漢字) か。
pub fn is_japanese(c: char) -> bool {
    is_hiragana(c) || is_katakana(c) || is_kanji(c)
}

/// 改行をまたいで連結するとき、間に空白を入れずにつなげる文字か
/// (日本語の文字・全角の記号・プレースホルダ)。
pub fn is_cjk_like(c: char) -> bool {
    is_japanese(c)
        || c == PLACEHOLDER
        || matches!(c, '\u{3000}'..='\u{303F}' | '\u{FF00}'..='\u{FFEF}')
}

/// ASCII または全角の英数字か。
pub fn is_alnum(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '０'..='９' | 'Ａ'..='Ｚ' | 'ａ'..='ｚ')
}

/// 文字列に日本語の文字が 1 字でも含まれるか。
pub fn contains_japanese(s: &str) -> bool {
    s.chars().any(is_japanese)
}

/// 全角・半角の読点類。
pub fn is_comma(c: char) -> bool {
    matches!(c, '、' | '，' | '､')
}

/// 文末記号 (文分割に使う `hasami::sentence` と同じ集合)。
///
/// 字だけで判定する。ASCII の `!` `?` は URL やコード片にも現れるため、文分割
/// ([`crate::segment`]) は直後の文字も見て文末かどうかを決める。
pub fn is_sentence_ender(c: char) -> bool {
    matches!(
        c,
        '。' | '！' | '？' | '!' | '?' | '‼' | '⁇' | '⁈' | '⁉' | '．' | '｡'
    )
}

/// 開き括弧に対応する閉じ括弧を返す (文分割に使う `hasami::sentence` と同じ括弧類)。
///
/// ASCII の `"` `'` は開閉が同じ字で、アポストロフィや寸法表記にも使われるため含めない。
pub fn closing_bracket(open: char) -> Option<char> {
    Some(match open {
        '「' => '」',
        '『' => '』',
        '（' => '）',
        '(' => ')',
        '〔' => '〕',
        '［' => '］',
        '[' => ']',
        '｛' => '｝',
        '{' => '}',
        '〈' => '〉',
        '《' => '》',
        '【' => '】',
        '〖' => '〗',
        '〘' => '〙',
        '〚' => '〛',
        '｟' => '｠',
        '“' => '”',
        '‘' => '’',
        '«' => '»',
        '‹' => '›',
        '｢' => '｣',
        '〝' => '〟',
        _ => return None,
    })
}

/// 閉じ括弧か。
pub fn is_closing_bracket(c: char) -> bool {
    matches!(
        c,
        '」' | '』'
            | '）'
            | ')'
            | '〕'
            | '］'
            | ']'
            | '｝'
            | '}'
            | '〉'
            | '》'
            | '】'
            | '〗'
            | '〙'
            | '〛'
            | '｠'
            | '”'
            | '’'
            | '»'
            | '›'
            | '｣'
            | '〟'
            | '〞'
    )
}

/// 読み手が実際に読む文字数の近似。
///
/// 空白類は数えず、プレースホルダ (インラインコード等) は 1 字として数える。
/// 文末記号の連続は数えない (校正に使った文長は文末記号を含まない数え方だったため)。
pub fn reading_length(s: &str) -> usize {
    let trimmed = s.trim_end_matches(|c: char| is_sentence_ender(c) || c.is_whitespace());
    trimmed.chars().filter(|c| !c.is_whitespace()).count()
}

/// 文末の記号 (文末記号・閉じ括弧・空白) を取り除いた本体。
pub fn strip_sentence_end(s: &str) -> &str {
    s.trim_end_matches(|c: char| is_sentence_ender(c) || is_closing_bracket(c) || c.is_whitespace())
}

/// 名詞らしい語の終わり方をする文字か (漢字・カタカナ・英数字・プレースホルダ)。
///
/// 辞書を使わないため、品詞の代わりに文字種で名詞を推定する。
pub fn is_nounish(c: char) -> bool {
    is_kanji(c) || is_katakana(c) || is_alnum(c) || c == PLACEHOLDER
}

/// 名詞で終わる文とみなす形式名詞 (ひらがなで書かれる名詞)。
const FORMAL_NOUNS: [&str; 12] = [
    "こと",
    "もの",
    "ため",
    "とき",
    "ところ",
    "はず",
    "わけ",
    "ほう",
    "うち",
    "まま",
    "ほど",
    "つもり",
];

/// 名詞らしい終止 (体言止め) か (辞書を使わない推定)。
///
/// 文末の記号を除いた最後の文字が名詞らしい文字か、形式名詞で終わる文を名詞の終止とみなす。
/// R06・R12 と `noslop diff` の改稿の偏りが、同じ近似で数えるためにここに置く。
pub fn is_nominal_ending(sentence: &str) -> bool {
    let core = strip_sentence_end(sentence);
    match core.chars().next_back() {
        Some(c) if is_nounish(c) => true,
        Some(_) => FORMAL_NOUNS.iter().any(|n| core.ends_with(n)),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_scripts() {
        assert!(is_hiragana('あ'));
        assert!(is_katakana('ア'));
        assert!(is_kanji('漢'));
        assert!(is_kanji('々'));
        assert!(!is_japanese('a'));
        assert!(contains_japanese("abc日本"));
        assert!(!contains_japanese("abc 123"));
    }

    #[test]
    fn reading_length_ignores_whitespace_and_final_enders() {
        assert_eq!(reading_length("今日は 晴れ。"), 5);
        assert_eq!(reading_length("本当！？"), 2);
        assert_eq!(reading_length("\u{FFFC}を実行する。"), 6);
    }

    #[test]
    fn brackets_pair_up() {
        assert_eq!(closing_bracket('「'), Some('」'));
        assert_eq!(closing_bracket('a'), None);
        assert!(is_closing_bracket('）'));
    }

    #[test]
    fn strips_sentence_end_marks() {
        assert_eq!(strip_sentence_end("言った。」 "), "言った");
        assert_eq!(strip_sentence_end("本当！？"), "本当");
    }

    #[test]
    fn nounish_characters() {
        assert!(is_nounish('評'));
        assert!(is_nounish('ト'));
        assert!(is_nounish('A'));
        assert!(is_nounish(PLACEHOLDER));
        assert!(!is_nounish('す'));
    }

    #[test]
    fn nominal_endings_are_guessed_from_the_last_character() {
        assert!(is_nominal_ending("次の課題は夜間の対応。"));
        assert!(is_nominal_ending("大切なのは続けること。"));
        assert!(is_nominal_ending("結果は「良好」。"));
        assert!(is_nominal_ending("明日から始めるつもり。"));
        assert!(!is_nominal_ending("半分に減りました。"));
        assert!(!is_nominal_ending("。"));
    }
}
