//! 語句への言及を除くための、同じブロック内の引用範囲。

use std::ops::Range;

pub(super) struct QuoteRanges {
    interiors: Vec<Range<usize>>,
    openings: Vec<usize>,
}

impl QuoteRanges {
    pub(super) fn new(text: &str) -> Self {
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
        }
    }

    pub(super) fn excludes(&self, hit: &Range<usize>) -> bool {
        self.openings.binary_search(&hit.start).is_ok()
            || self
                .interiors
                .iter()
                .any(|q| q.start <= hit.start && hit.end <= q.end)
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
}
