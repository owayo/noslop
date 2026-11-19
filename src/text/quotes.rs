//! 同じブロック内の引用範囲。
//!
//! 語句ルールが語句への言及を除くときと、`noslop diff` が引用の中の人・担当の語を取らないときに使う。

use std::collections::HashMap;
use std::ops::Range;

/// まだ閉じていない開き括弧 (引用符) の並び。
///
/// 閉じは、対応する最も内側の開きまで戻って閉じる (内側に閉じない括弧が残っていても外側を閉じる)。
/// 閉じの字ごとに、まだ閉じていない開きの数を持つので、対応する開きのない字では並びを探さない
/// (閉じない括弧が積み上がった長い段落で、字ごとに並びを探すと二乗になる)。並びを探して閉じた
/// ときに飛ばした開きは取り除くので、探す手間は全体で入力の長さに比例する。
#[derive(Debug, Default)]
pub(crate) struct OpenStack {
    /// (開きの位置, 対応する閉じの字)。位置は呼び出し側が決める (開きの字の位置か、その直後)。
    open: Vec<(usize, char)>,
    waiting: HashMap<char, usize>,
}

impl OpenStack {
    pub(crate) fn push(&mut self, at: usize, close: char) {
        self.open.push((at, close));
        *self.waiting.entry(close).or_default() += 1;
    }

    /// `c` で閉じる開きがあれば、それより内側の開きごと閉じて、その開きの位置を返す。
    pub(crate) fn close(&mut self, c: char) -> Option<usize> {
        if self.waiting.get(&c).is_none_or(|&n| n == 0) {
            return None;
        }
        let position = self.open.iter().rposition(|&(_, close)| close == c)?;
        let start = self.open[position].0;
        for &(_, close) in &self.open[position..] {
            if let Some(n) = self.waiting.get_mut(&close) {
                *n -= 1;
            }
        }
        self.open.truncate(position);
        Some(start)
    }

    /// 閉じないまま残った開きの位置 (並びの順)。
    pub(crate) fn unclosed(&self) -> impl Iterator<Item = usize> + '_ {
        self.open.iter().map(|&(at, _)| at)
    }
}

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
        let mut stack = OpenStack::default();
        for (offset, c) in text.char_indices() {
            // 非対称の引用符は、内側に別種の未対応の引用符があっても閉じる
            if let Some(start) = stack.close(c) {
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
                stack.push(start, close);
                openings.push(start);
            }
        }
        Self {
            interiors,
            openings,
            unclosed: stack.unclosed().collect(),
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

    /// 開きの並びを素朴に探す閉じ方 (`rposition`) と、閉じた範囲・残った開きが同じになる。
    /// 5 種の字の長さ 7 までの並びをすべて試す。
    #[test]
    fn open_stack_closes_like_a_nearest_search() {
        let alphabet = ['「', '」', '（', '）', 'a'];
        let close_of = |c: char| match c {
            '「' => Some('」'),
            '（' => Some('）'),
            _ => None,
        };
        let mut words = vec![Vec::new()];
        for _ in 0..7 {
            let mut longer = Vec::new();
            for w in &words {
                for &c in &alphabet {
                    let mut v: Vec<char> = w.clone();
                    v.push(c);
                    longer.push(v);
                }
            }
            for w in &longer {
                let (mut naive, mut naive_open) = (Vec::new(), Vec::<(usize, char)>::new());
                let (mut fast, mut stack) = (Vec::new(), OpenStack::default());
                for (at, &c) in w.iter().enumerate() {
                    if let Some(close) = close_of(c) {
                        naive_open.push((at, close));
                        stack.push(at, close);
                    } else {
                        if let Some(p) = naive_open.iter().rposition(|&(_, close)| close == c) {
                            naive.push(naive_open[p].0..at);
                            naive_open.truncate(p);
                        }
                        if let Some(start) = stack.close(c) {
                            fast.push(start..at);
                        }
                    }
                }
                assert_eq!(fast, naive, "{w:?}");
                let rest: Vec<usize> = stack.unclosed().collect();
                let naive_rest: Vec<usize> = naive_open.iter().map(|&(at, _)| at).collect();
                assert_eq!(rest, naive_rest, "{w:?}");
            }
            words = longer;
        }
    }

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
