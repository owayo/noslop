//! 同じブロック内の引用範囲。
//!
//! 語句ルールが語句への言及を除くときと、`noslop diff` が引用の中の人・担当の語を取らないときに使う。

use std::ops::Range;

pub(crate) struct QuoteRanges {
    interiors: Vec<Range<usize>>,
    openings: Vec<usize>,
    /// 閉じないままブロックが終わった引用の内側の始まり。
    unclosed: Vec<usize>,
}

impl QuoteRanges {
    pub(crate) fn new(text: &str) -> Self {
        let mut interiors = Vec::new();
        let mut openings = Vec::new();
        let mut stack = Vec::new();
        for (offset, c) in text.char_indices() {
            // 非対称の引用符は、内側に別種の未対応の引用符があっても閉じる。
            if let Some(position) = stack.iter().rposition(|&(_, close)| close == c) {
                let (start, _) = stack[position];
                stack.truncate(position);
                interiors.push(start..offset);
            } else if let Some(close) = match c {
                '「' => Some('」'),
                '『' => Some('』'),
                '“' => Some('”'),
                '"' if !text[..offset]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_ascii_digit()) =>
                {
                    Some('"')
                }
                _ => None,
            } {
                let start = offset + c.len_utf8();
                stack.push((start, close));
                openings.push(start);
            }
        }
        Self {
            interiors,
            openings,
            unclosed: stack.into_iter().map(|(start, _)| start).collect(),
        }
    }

    /// 語句への言及として除くか: 閉じた引用の内側に収まるか、開き引用符の直後から始まる。
    pub(crate) fn excludes(&self, hit: &Range<usize>) -> bool {
        self.openings.binary_search(&hit.start).is_ok()
            || self
                .interiors
                .iter()
                .any(|q| q.start <= hit.start && hit.end <= q.end)
    }

    /// 言及として除いた一致 `hit` ([`Self::excludes`]) の後ろで、次の一致を探し始める位置。
    ///
    /// 一致の始まりを含む閉じた引用があれば、その閉じの位置 (入れ子なら一番内側の引用)。引用の
    /// 外へはみ出した一致 (「単なる作業」と、これは単なる道具ではなく) の、引用の外の部分から
    /// 探し直すため。閉じない引用の直後から始まる一致は、引用の範囲が分からないので、一致の
    /// 全体を言及とみなして一致の末尾から探す。
    pub(crate) fn resume_after(&self, hit: &Range<usize>) -> usize {
        self.interiors
            .iter()
            .filter(|q| q.start <= hit.start && hit.start < q.end)
            .map(|q| q.end)
            .min()
            .unwrap_or(hit.end)
    }

    /// 引用の内側に一部でも重なるか。閉じない引用は、ブロックの終わりまでを内側とみなす。
    pub(crate) fn overlaps(&self, range: &Range<usize>) -> bool {
        self.interiors
            .iter()
            .any(|q| q.start < range.end && range.start < q.end)
            || self.unclosed.iter().any(|&start| start < range.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_nested_quotes_and_recovers_from_unclosed_inner_quotes() {
        for text in ["「前『対象』後」外", "「前5\"対象」外", "“前「対象」後”外"]
        {
            let quotes = QuoteRanges::new(text);
            let start = text.find("対象").unwrap();
            assert!(quotes.excludes(&(start..start + "対象".len())));
            let start = text.find("外").unwrap();
            assert!(!quotes.excludes(&(start..text.len())));
            assert!(!quotes.excludes(&(0..text.len())));
        }
    }

    #[test]
    fn only_opening_quotes_exclude_the_next_word() {
        for text in ["「対象", "『対象", "“対象", "\"対象"] {
            let start = text.find("対象").unwrap();
            assert!(QuoteRanges::new(text).excludes(&(start..text.len())));
        }
        for text in ["「 対象", "」対象", "\"前\"対象", "5\"対象", "「前対象"] {
            let start = text.find("対象").unwrap();
            assert!(
                !QuoteRanges::new(text).excludes(&(start..text.len())),
                "{text}"
            );
        }
        let text = "\"対象5\"外";
        let start = text.find("外").unwrap();
        assert!(!QuoteRanges::new(text).excludes(&(start..text.len())));
    }

    #[test]
    fn resumes_after_the_closing_quote_or_the_whole_unclosed_mention() {
        let range = |text: &str, word: &str| {
            let start = text.find(word).unwrap();
            start..start + word.len()
        };
        // 閉じた引用の直後から始まり、外へはみ出した一致: 閉じの引用符から探し直す
        let text = "「単なる作業」と、これは単なる道具ではなく";
        let hit = range(text, "単なる作業」と、これは単なる道具ではなく");
        assert_eq!(
            QuoteRanges::new(text).resume_after(&hit),
            text.find('」').unwrap()
        );
        // 入れ子の引用の中: 一番内側の引用の閉じ
        let text = "「前『対象』後」外";
        let hit = range(text, "対象");
        assert_eq!(
            QuoteRanges::new(text).resume_after(&hit),
            text.find('』').unwrap()
        );
        // 閉じない引用の直後: 一致の末尾
        let text = "「することができるという語";
        let hit = range(text, "することができる");
        assert_eq!(QuoteRanges::new(text).resume_after(&hit), hit.end);
    }

    #[test]
    fn overlaps_counts_partial_overlaps_and_unclosed_quotes_to_the_end() {
        let range = |text: &str, word: &str| {
            let start = text.find(word).unwrap();
            start..start + word.len()
        };
        // 閉じた引用: 一部でも重なれば重なる。外にあれば重ならない
        let text = "前「対象」後";
        let quotes = QuoteRanges::new(text);
        assert!(quotes.overlaps(&range(text, "象」後")));
        assert!(quotes.overlaps(&range(text, "対象")));
        assert!(!quotes.overlaps(&range(text, "後")));
        assert!(!quotes.overlaps(&range(text, "前")));
        // 閉じない引用は、開いた位置からブロックの終わりまで
        let text = "前「説明、対象が続く";
        let quotes = QuoteRanges::new(text);
        assert!(quotes.overlaps(&range(text, "対象が")));
        assert!(!quotes.overlaps(&range(text, "前")));
    }
}
