//! 改稿の偏り: 同じ変換を文書全体に一律に当てた形跡を探す。
//!
//! 個々の直しが正しくても、全部の見出しを同じ書式にそろえる・箇条書きを一斉に地の文へ
//! 開く・体言止めを機械的に足す・読点を一律に削る、といった掃引は新しい均一さを生む。
//! ここで出すのは「一律に当てていないか見直してください」という確認事項で、改稿を
//! 失敗扱いにはしない。

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::diagnostic::Span;
use crate::document::{BlockKind, Document, Sentence};
use crate::heading::HeadingShape;
use crate::text::{self, is_nominal_ending};

/// 見出しの形をそろえたとみなす、最も多い形の割合。
const HEADING_SHARE: f64 = 0.8;
/// 改稿前と比べて、その形の割合がこれ以上増えたときだけ「そろえた」とみなす。
const HEADING_SHARE_GAIN: f64 = 0.3;
/// 見出しの形を比べるのに要る見出しの数。
const MIN_HEADINGS: usize = 3;
/// 見出しの書き換えを一律とみなす割合。
const REWRITTEN_SHARE: f64 = 0.8;
/// 箇条書きの増減を比べるのに要る項目の数。
const MIN_LIST_ITEMS: usize = 4;
/// 文の統計を比べるのに要る地の文の文数。
const MIN_SENTENCES: usize = 8;
/// 文長の burstiness と交互の並びを比べるのに要る文数。
const MIN_SENTENCES_FOR_RHYTHM: usize = 10;
/// 名詞らしい終止の比率の変化をこれ以上なら指す。
const NOMINAL_DELTA: f64 = 0.15;
/// 1 文あたりの読点数の相対変化をこれ以上なら指す。
const COMMA_RELATIVE_DELTA: f64 = 0.4;
/// 改稿前の読点が少ないときに見る、1 文あたりの読点数の絶対増加。
const COMMA_ABSOLUTE_DELTA: f64 = 0.8;
/// burstiness の変化をこれ以上なら指す。
const BURSTINESS_DELTA: f64 = 0.15;
/// 隣り合う文の長さの相関がこれ以下なら「長短が機械的に交互」とみなす。
const ALTERNATION_THRESHOLD: f64 = -0.5;
/// 同じ直しがこれ以上の文に当たっていたら指す。
const REPEATED_EDIT_MIN: usize = 3;
/// 同じ直しとして数える、削った・足した文字列の長さの上限 (文字数)。
const REPEATED_EDIT_MAX_CHARS: usize = 16;
/// 改稿前後の文を対応づけるときの、文字 bigram の Jaccard 係数の下限。
const PAIR_SIMILARITY: f64 = 0.5;
/// 文の対応づけで比べる組の数の上限 (これを超える長い文書では同じ直しの検出を省く)。
const MAX_PAIR_COMPARISONS: usize = 4_000_000;

/// 偏りの種類。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ShiftKind {
    /// 見出しの形が 1 つにそろった。
    HeadingShape,
    /// 見出しのほとんどを書き換えた。
    HeadingsRewritten,
    /// 箇条書きの多くを地の文に開いた。
    ListsToProse,
    /// 地の文の多くを箇条書きにした。
    ProseToLists,
    /// 名詞らしい終止 (体言止め) の比率が大きく動いた。
    NominalEndings,
    /// 1 文あたりの読点の数が大きく動いた。
    Commas,
    /// 文長のばらつき (burstiness) が大きく動いた。
    Burstiness,
    /// 長い文と短い文が機械的に交互に並んだ。
    Alternation,
    /// 同じ直しを多数の文に当てた。
    RepeatedEdit,
}

/// 偏り 1 件。
#[derive(Debug, Clone, PartialEq)]
pub struct Shift {
    pub kind: ShiftKind,
    /// 何が起きたか (具体的に)。
    pub message: String,
    pub before: Option<f64>,
    pub after: Option<f64>,
    /// 該当した件数 (見出しの数・文の数など)。
    pub count: Option<usize>,
    /// 改稿後の文書での該当箇所。
    pub spans: Vec<Span>,
}

/// 前後の文書から偏りを探す。
pub fn detect(before: &Document, after: &Document) -> Vec<Shift> {
    let mut shifts = Vec::new();
    headings(before, after, &mut shifts);
    lists(before, after, &mut shifts);
    sentence_stats(before, after, &mut shifts);
    repeated_edits(before, after, &mut shifts);
    shifts
}

/// 比べる見出し (文書の題名になる最初の H1 は除く)。
fn heading_blocks(doc: &Document) -> Vec<(String, Span)> {
    doc.blocks
        .iter()
        .filter_map(|b| match b.kind {
            BlockKind::Heading(level) if level != 1 => Some((b.text.trim().to_string(), b.span)),
            _ => None,
        })
        .collect()
}

fn headings(before: &Document, after: &Document, out: &mut Vec<Shift>) {
    let hb = heading_blocks(before);
    let ha = heading_blocks(after);
    if ha.len() < MIN_HEADINGS {
        return;
    }
    let share = |hs: &[(String, Span)], shape: HeadingShape| {
        if hs.is_empty() {
            0.0
        } else {
            hs.iter()
                .filter(|(t, _)| HeadingShape::of(t) == shape)
                .count() as f64
                / hs.len() as f64
        }
    };
    for shape in [
        HeadingShape::Colon,
        HeadingShape::Question,
        HeadingShape::Numbered,
    ] {
        let (sa, sb) = (share(&ha, shape), share(&hb, shape));
        if sa >= HEADING_SHARE && sa - sb >= HEADING_SHARE_GAIN {
            let matching: Vec<Span> = ha
                .iter()
                .filter(|(t, _)| HeadingShape::of(t) == shape)
                .map(|(_, s)| *s)
                .collect();
            let before_count = hb
                .iter()
                .filter(|(t, _)| HeadingShape::of(t) == shape)
                .count();
            out.push(Shift {
                kind: ShiftKind::HeadingShape,
                message: format!(
                    "見出し {} 件のうち {} 件が{}にそろいました (改稿前は {} 件中 {} 件)。全部の見出しを同じ書式にすると、見出しを拾い読みしたときに単調な並びになります。形を内容に合わせて混ぜられないか見直してください",
                    ha.len(),
                    matching.len(),
                    shape.label(),
                    hb.len(),
                    before_count
                ),
                before: Some(sb),
                after: Some(sa),
                count: Some(matching.len()),
                spans: matching,
            });
        }
    }

    // 見出しの数がほぼ同じまま、ほとんどを書き換えた
    if hb.len() >= MIN_HEADINGS && hb.len().abs_diff(ha.len()) <= 1 {
        let old: HashSet<&str> = hb.iter().map(|(t, _)| t.as_str()).collect();
        let rewritten: Vec<Span> = ha
            .iter()
            .filter(|(t, _)| !old.contains(t.as_str()))
            .map(|(_, s)| *s)
            .collect();
        let ratio = rewritten.len() as f64 / ha.len() as f64;
        if ratio >= REWRITTEN_SHARE {
            out.push(Shift {
                kind: ShiftKind::HeadingsRewritten,
                message: format!(
                    "見出し {} 件のうち {} 件を書き換えています。結論を見出しに入れる直しは、開いて中身を探す手間が大きい節だけに絞れないか見直してください",
                    ha.len(),
                    rewritten.len()
                ),
                before: None,
                after: Some(ratio),
                count: Some(rewritten.len()),
                spans: rewritten,
            });
        }
    }
}

fn count_kind(doc: &Document, kind: BlockKind) -> usize {
    doc.blocks.iter().filter(|b| b.kind == kind).count()
}

fn lists(before: &Document, after: &Document, out: &mut Vec<Shift>) {
    let (lb, la) = (
        count_kind(before, BlockKind::ListItem),
        count_kind(after, BlockKind::ListItem),
    );
    let (pb, pa) = (
        count_kind(before, BlockKind::Paragraph),
        count_kind(after, BlockKind::Paragraph),
    );
    if lb >= MIN_LIST_ITEMS && la * 2 <= lb && pa > pb {
        out.push(Shift {
            kind: ShiftKind::ListsToProse,
            message: format!(
                "箇条書きの項目が {lb} → {la} に減り、段落が {pb} → {pa} に増えました。因果や経緯が隠れた箇条書きだけを地の文に戻し、本当に並列で後から探す一覧 (決定事項・宿題など) は箇条書きのまま残してください"
            ),
            before: Some(lb as f64),
            after: Some(la as f64),
            count: Some(lb - la),
            spans: Vec::new(),
        });
    } else if la >= lb + MIN_LIST_ITEMS && la >= 2 * lb.max(1) && pa < pb {
        let spans = after
            .blocks
            .iter()
            .filter(|b| b.kind == BlockKind::ListItem)
            .map(|b| b.span)
            .collect();
        out.push(Shift {
            kind: ShiftKind::ProseToLists,
            message: format!(
                "箇条書きの項目が {lb} → {la} に増え、段落が {pb} → {pa} に減りました。つながりのある説明まで箇条書きに刻んでいないか見直してください"
            ),
            before: Some(lb as f64),
            after: Some(la as f64),
            count: Some(la - lb),
            spans,
        });
    }
}

/// 地の文の統計。
struct Stats {
    count: usize,
    nominal_ratio: f64,
    commas_per_sentence: f64,
    burstiness: Option<f64>,
    lag1: Option<f64>,
}

fn stats(doc: &Document) -> Stats {
    let sentences: Vec<&Sentence> = doc.prose_sentences().collect();
    let count = sentences.len();
    if count == 0 {
        return Stats {
            count,
            nominal_ratio: 0.0,
            commas_per_sentence: 0.0,
            burstiness: None,
            lag1: None,
        };
    }
    let nominal = sentences
        .iter()
        .filter(|s| is_nominal_ending(doc.sentence_text(s)))
        .count();
    let commas: usize = sentences
        .iter()
        .map(|s| {
            doc.sentence_text(s)
                .chars()
                .filter(|&c| text::is_comma(c))
                .count()
        })
        .sum();
    let lengths: Vec<f64> = sentences.iter().map(|s| s.length as f64).collect();
    let (mean, sd) = mean_sd(&lengths);
    let burstiness = (count >= 2 && mean + sd > 0.0).then(|| (sd - mean) / (sd + mean));
    Stats {
        count,
        nominal_ratio: nominal as f64 / count as f64,
        commas_per_sentence: commas as f64 / count as f64,
        burstiness,
        lag1: lag1_autocorrelation(&lengths),
    }
}

fn mean_sd(values: &[f64]) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    (mean, var.sqrt())
}

/// 隣り合う文の長さの相関 (lag-1 自己相関)。ばらつきがなければ `None`。
fn lag1_autocorrelation(values: &[f64]) -> Option<f64> {
    if values.len() < 3 {
        return None;
    }
    let xs = &values[..values.len() - 1];
    let ys = &values[1..];
    let (mx, sx) = mean_sd(xs);
    let (my, sy) = mean_sd(ys);
    if sx == 0.0 || sy == 0.0 {
        return None;
    }
    let cov = xs
        .iter()
        .zip(ys)
        .map(|(a, b)| (a - mx) * (b - my))
        .sum::<f64>()
        / xs.len() as f64;
    Some(cov / (sx * sy))
}

fn sentence_stats(before: &Document, after: &Document, out: &mut Vec<Shift>) {
    let (sb, sa) = (stats(before), stats(after));
    if sb.count < MIN_SENTENCES || sa.count < MIN_SENTENCES {
        return;
    }
    let pct = |v: f64| format!("{:.0}%", v * 100.0);

    let delta = sa.nominal_ratio - sb.nominal_ratio;
    if delta.abs() >= NOMINAL_DELTA {
        let direction = if delta > 0.0 {
            "増えました"
        } else {
            "減りました"
        };
        out.push(Shift {
            kind: ShiftKind::NominalEndings,
            message: format!(
                "名詞らしい終止 (体言止め) の比率が {} → {} に{direction}。体言止めを数合わせで足したり削ったりしていないか見直してください",
                pct(sb.nominal_ratio),
                pct(sa.nominal_ratio)
            ),
            before: Some(sb.nominal_ratio),
            after: Some(sa.nominal_ratio),
            count: None,
            spans: Vec::new(),
        });
    }

    let (cb, ca) = (sb.commas_per_sentence, sa.commas_per_sentence);
    let comma_shift = if cb >= 0.5 {
        (ca - cb).abs() / cb >= COMMA_RELATIVE_DELTA
    } else {
        ca - cb >= COMMA_ABSOLUTE_DELTA
    };
    if comma_shift {
        let direction = if ca > cb {
            "増えました"
        } else {
            "減りました"
        };
        out.push(Shift {
            kind: ShiftKind::Commas,
            message: format!(
                "読点が 1 文あたり {cb:.1} → {ca:.1} 個に{direction}。読点を一律に削ったり足したりしていないか、文の切れ目の読みやすさで見直してください"
            ),
            before: Some(cb),
            after: Some(ca),
            count: None,
            spans: Vec::new(),
        });
    }

    if sb.count >= MIN_SENTENCES_FOR_RHYTHM && sa.count >= MIN_SENTENCES_FOR_RHYTHM {
        if let (Some(bb), Some(ba)) = (sb.burstiness, sa.burstiness)
            && (ba - bb).abs() >= BURSTINESS_DELTA
        {
            out.push(Shift {
                kind: ShiftKind::Burstiness,
                message: format!(
                    "文長のばらつき (burstiness) が {bb:.2} → {ba:.2} に変わりました。長い文と短い文を形だけで付け足していないか、情報の重さに合った長さか見直してください"
                ),
                before: Some(bb),
                after: Some(ba),
                count: None,
                spans: Vec::new(),
            });
        }
        if let Some(la) = sa.lag1
            && la <= ALTERNATION_THRESHOLD
            && sb.lag1.is_none_or(|lb| lb > ALTERNATION_THRESHOLD)
        {
            out.push(Shift {
                kind: ShiftKind::Alternation,
                message: format!(
                    "隣り合う文の長さの相関が {la:.2} で、長い文と短い文が機械的に交互に並んでいます。リズムを付けるための交互の並びは、それ自体が新しい型として読まれます"
                ),
                before: sb.lag1,
                after: Some(la),
                count: None,
                spans: Vec::new(),
            });
        }
    }
}

/// 改稿前後の文 (日本語を含む文すべて。空白を除いた本文と原文の位置)。
fn sentence_texts(doc: &Document) -> Vec<(String, Span)> {
    doc.sentences
        .iter()
        .filter(|s| s.japanese)
        .map(|s| {
            let t: String = doc
                .sentence_text(s)
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            (t, s.span)
        })
        .collect()
}

fn bigrams(s: &str) -> HashSet<(char, char)> {
    let chars: Vec<char> = s.chars().collect();
    chars.windows(2).map(|w| (w[0], w[1])).collect()
}

fn jaccard(a: &HashSet<(char, char)>, b: &HashSet<(char, char)>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let inter = a.intersection(b).count();
    inter as f64 / (a.len() + b.len() - inter) as f64
}

/// 1 組の文の違いを「削った部分 → 足した部分」の 1 か所にまとめる (前後の共通部分を除く)。
fn local_edit(before: &str, after: &str) -> (String, String) {
    let b: Vec<char> = before.chars().collect();
    let a: Vec<char> = after.chars().collect();
    let prefix = b.iter().zip(&a).take_while(|(x, y)| x == y).count();
    let max_suffix = b.len().min(a.len()) - prefix;
    let suffix = b
        .iter()
        .rev()
        .zip(a.iter().rev())
        .take(max_suffix)
        .take_while(|(x, y)| x == y)
        .count();
    (
        b[prefix..b.len() - suffix].iter().collect(),
        a[prefix..a.len() - suffix].iter().collect(),
    )
}

fn repeated_edits(before: &Document, after: &Document, out: &mut Vec<Shift>) {
    let tb = sentence_texts(before);
    let ta = sentence_texts(after);
    let unchanged_before: HashSet<&str> = tb.iter().map(|(t, _)| t.as_str()).collect();
    let unchanged_after: HashSet<&str> = ta.iter().map(|(t, _)| t.as_str()).collect();
    let changed_before: Vec<&(String, Span)> = tb
        .iter()
        .filter(|(t, _)| !unchanged_after.contains(t.as_str()))
        .collect();
    let changed_after: Vec<&(String, Span)> = ta
        .iter()
        .filter(|(t, _)| !unchanged_before.contains(t.as_str()))
        .collect();
    if changed_before.is_empty()
        || changed_after.is_empty()
        || changed_before.len() * changed_after.len() > MAX_PAIR_COMPARISONS
    {
        return;
    }

    // 改稿後の変わった文ごとに、いちばん似ている改稿前の文を対応づける (一度使った文は使わない)
    let before_grams: Vec<HashSet<(char, char)>> =
        changed_before.iter().map(|(t, _)| bigrams(t)).collect();
    let mut used = vec![false; changed_before.len()];
    let mut edits: HashMap<(String, String), Vec<Span>> = HashMap::new();
    for (text_after, span_after) in &changed_after {
        let grams = bigrams(text_after);
        let best = before_grams
            .iter()
            .enumerate()
            .filter(|(i, _)| !used[*i])
            .map(|(i, g)| (i, jaccard(g, &grams)))
            .max_by(|x, y| x.1.total_cmp(&y.1));
        let Some((i, sim)) = best else {
            continue;
        };
        if sim < PAIR_SIMILARITY {
            continue;
        }
        used[i] = true;
        let (deleted, inserted) = local_edit(&changed_before[i].0, text_after);
        let size = deleted.chars().count() + inserted.chars().count();
        if size == 0
            || deleted.chars().count() > REPEATED_EDIT_MAX_CHARS
            || inserted.chars().count() > REPEATED_EDIT_MAX_CHARS
        {
            continue;
        }
        edits
            .entry((deleted, inserted))
            .or_default()
            .push(*span_after);
    }

    let mut repeated: Vec<((String, String), Vec<Span>)> = edits
        .into_iter()
        .filter(|(_, spans)| spans.len() >= REPEATED_EDIT_MIN)
        .collect();
    repeated.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
    for ((deleted, inserted), spans) in repeated {
        let what = match (deleted.is_empty(), inserted.is_empty()) {
            (true, false) => format!("「{inserted}」の追加"),
            (false, true) => format!("「{deleted}」の削除"),
            _ => format!("「{deleted}」→「{inserted}」の置き換え"),
        };
        out.push(Shift {
            kind: ShiftKind::RepeatedEdit,
            message: format!(
                "同じ直し ({what}) を {} 文に当てています。効果の大きい箇所だけに絞れないか、一律に当てていないか見直してください",
                spans.len()
            ),
            before: None,
            after: None,
            count: Some(spans.len()),
            spans,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(before: &str, after: &str) -> Vec<ShiftKind> {
        detect(&Document::markdown(before), &Document::markdown(after))
            .into_iter()
            .map(|s| s.kind)
            .collect()
    }

    #[test]
    fn detects_headings_converging_to_one_shape() {
        let before = "# 題\n\n## 背景\n\n本文。\n\n## 課題\n\n本文。\n\n## 提案\n\n本文。\n\n## 今後\n\n本文。\n";
        let after = "# 題\n\n## 背景: 電話のほうが信頼されている\n\n本文。\n\n## 課題: 入力の手間が大きい\n\n本文。\n\n## 提案: 入力をなくす\n\n本文。\n\n## 今後: 月末に見直す\n\n本文。\n";
        let k = kinds(before, after);
        assert!(k.contains(&ShiftKind::HeadingShape), "{k:?}");
        assert!(k.contains(&ShiftKind::HeadingsRewritten), "{k:?}");
    }

    #[test]
    fn keeps_quiet_when_headings_already_had_the_shape() {
        let before = "## A: あ\n\n本文。\n\n## B: い\n\n本文。\n\n## C: う\n\n本文。\n";
        let after = "## A: あ\n\n本文です。\n\n## B: い\n\n本文です。\n\n## C: う\n\n本文です。\n";
        assert!(!kinds(before, after).contains(&ShiftKind::HeadingShape));
    }

    #[test]
    fn detects_lists_opened_into_prose() {
        let before =
            "- 在庫が減った\n- 担当が気づかなかった\n- 欠品した\n- 客が離れた\n- 売上が落ちた\n";
        let after = "在庫が減っても担当が気づかず、そのまま欠品した。客が離れ、売上も落ちた。\n";
        assert!(kinds(before, after).contains(&ShiftKind::ListsToProse));
        // 逆向き (段落 1 つ → 項目 5 つ) は地の文の箇条書き化
        assert!(kinds(after, before).contains(&ShiftKind::ProseToLists));
    }

    #[test]
    fn detects_the_same_edit_applied_to_many_sentences() {
        let before = "設定を変更することができます。記録を削除することができます。一覧を表示することができます。画面を閉じることができます。\n";
        let after =
            "設定を変更できます。記録を削除できます。一覧を表示できます。画面を閉じられます。\n";
        let shifts = detect(&Document::markdown(before), &Document::markdown(after));
        let edit = shifts
            .iter()
            .find(|s| s.kind == ShiftKind::RepeatedEdit)
            .expect("repeated edit");
        // 「することが」を削った 3 文が同じ直しで、「閉じられます」は別の直しとして数える
        assert_eq!(edit.count, Some(3));
        assert!(
            edit.message.contains("「することが」の削除"),
            "{}",
            edit.message
        );
    }

    #[test]
    fn detects_prefix_added_to_many_sentences() {
        let before = "在庫を数えた。棚を片づけた。伝票を整理した。今日は晴れた。\n";
        let after =
            "実際に在庫を数えた。実際に棚を片づけた。実際に伝票を整理した。今日は晴れた。\n";
        let shifts = detect(&Document::markdown(before), &Document::markdown(after));
        let edit = shifts
            .iter()
            .find(|s| s.kind == ShiftKind::RepeatedEdit)
            .expect("repeated edit");
        assert_eq!(edit.count, Some(3));
        assert!(
            edit.message.contains("「実際に」の追加"),
            "{}",
            edit.message
        );
    }

    #[test]
    fn detects_nominal_ending_and_comma_shifts() {
        let before: String = (0..10)
            .map(|i| format!("手順{i}では、設定を、確認して、保存します。"))
            .collect::<String>()
            + "\n";
        let after: String = (0..10)
            .map(|i| format!("手順{i}で設定を確認し保存。"))
            .collect::<String>()
            + "\n";
        let k = kinds(&before, &after);
        assert!(k.contains(&ShiftKind::NominalEndings), "{k:?}");
        assert!(k.contains(&ShiftKind::Commas), "{k:?}");
    }

    #[test]
    fn detects_mechanical_alternation_of_lengths() {
        let long = "朝のうちに倉庫の棚を端から順に確かめて、足りない部品を一覧にまとめた。";
        let short = "終わった。";
        let before: String = (0..12)
            .map(|i| format!("{}日目は倉庫を{}回確かめた。", i + 1, i % 3 + 1))
            .collect::<String>()
            + "\n";
        let after: String = (0..12)
            .map(|i| if i % 2 == 0 { long } else { short })
            .collect::<String>()
            + "\n";
        assert!(kinds(&before, &after).contains(&ShiftKind::Alternation));
    }
}
