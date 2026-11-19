//! R22: 随筆によく出る表現が、近い範囲の地の文に系統を変えて重なる箇所。

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, Severity, Signal, Span};
use crate::document::{Block, Document, MarkKind, Sentence};
use crate::genre::Genre;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta};
use crate::text;
use crate::text::quotes::OpenStack;

use super::{option_count, unknown_option};

static META: RuleMeta = RuleMeta {
    id: "R22",
    name: "ESSAY_PHRASE_CLUSTER",
    title: "随筆の表現の重なり",
    lane: Lane::Slop,
    signal: Some(Signal::Strong),
    default_severity: Severity::Info,
    summary: "随筆によく出る「立ち止まる」「ふと」「静かに」などの表現が、近い範囲の地の文に系統を変えて重なる箇所を示す",
    explanation: r"### 何を見るか

既定ではジャンル essay (別名 blog) のときだけ動きます (設定や `--only-rules` で明示的に有効にすれば、ほかのジャンルでも動きます)。地の文を原文の順に約 1,300 字ずつの範囲 (窓) にまとめ、次の 8 系統のうち 2 系統以上 (min_families) が同じ窓に出る箇所を指します。「立ち止ま」「という行為」「た瞬間」(直後が「、」か「に」)、「少しだけ」「静かに」「ささやかな」「ふと」(直後が「、」「気」「思」「見」「目」)、「私たち」(直後が「は」「の」「が」) です。同じ系統が何度出ても 1 と数えます。

窓は、段落を順に詰め、次の段落を入れると 1,300 字 (空白と文末記号を除いた字数) を超えるところで区切ります。長い段落は文の境目で分け、1,300 字を超える一文はその文だけで 1 つの窓にします。見出し・リスト・表・引用・脚注・コード・区切り線、日本語を含まない文、「ラベル: 値」の行でも窓を区切り、その中の表現は数えません。括弧 (会話の「」や補足の（）など) の内側と、Markdown の記法で書いたリンク・コード・画像・数式・取り消し線に重なる表現も数えません (HTML のタグで書いたリンクなどの中は数えます)。

条件を満たした窓ごとに 1 件を出し、最初の表現を指して、ほかの系統の表現を関連箇所に添えます。

### なぜ問題か

生成した随筆では、足を止める・ふと気づく・静かに眺める・ささやかな喜び、といった内省や情景の決まった言い回しが、近い範囲にまとめて出ることがあります。どれも人の随筆にもある普通の表現で、重なっていても文章の欠点とは限りません。このルールは見直しのきっかけを示すだけで、書き手や執筆方法、文章の良し悪しは判定しません。

### 直し方

それぞれの表現が、その場面で意図した情景や考えを伝えているか確かめてください。場面や語り口に合っていれば、そのまま残してください。指摘を消すために別の語へ置き換えたり、事実や感情を書き足したりする必要はありません。

### 例

- 直す前: 駅の階段でふと立ち止まり、静かに暮れていく空を見上げた。ささやかな幸せとは、こういう時間のことなのかもしれない。
- 直した後: 駅の階段の途中で足を止め、暮れていく空を見上げた。幸せとは、こういう時間のことなのかもしれない。

### 根拠

系統は、2026 年のモデルで生成した随筆と人の随筆を比べて選びました。検証には、系統・窓・閾値を固定した後に同じ手順で生成した随筆 272 本と、2022 年 11 月 30 日より前に公開された人の随筆 102 本を使いました。人の 102 本は、系統を選ぶ段階でも反応を見ています。

2 系統以上で指摘した文書は、人 3 本 (2.9%、片側 95% 上限 7.1%)・生成 140 本 (51.5%) でした。3 系統以上にすると人 1 本・生成 50 本です。窓の単位では、人 482 窓のうち 3 窓 (0.6%)、生成 961 窓のうち 206 窓 (21.4%) でした。窓に入れた地の文の字数でも分けて比べました。人と生成の順に、1,000 字以下は 12 本中 0 本と 139 本中 52 本、2,000 字以下は 37 本中 0 本と 71 本中 40 本です。4,000 字以下は 31 本中 2 本と 38 本中 29 本、それより長いものは 22 本中 1 本と 24 本中 19 本でした。系統を 1 つずつ外しても、生成の反応は 36.4% 以上に残り、人の反応は 3 本を超えませんでした。見出しや箇条書きで区切った随筆は窓が短くなり、反応が下がります (文体を指定して体裁を整えさせた生成 63 本では 18 本)。

人に当たった 3 本は、出来事の説明や体験の描写として自然な箇所でした。検証に使った生成の随筆は、お題 18 題とモデルの系統が系統の選定に使ったものと同じで、新しいお題での検証はしていません。

手掛かりの強さは「強い」です (人の随筆 102 本と、系統を固定した後に生成した随筆 272 本で測った値)。",
};

/// 系統ごとの照合。前後の文字を条件にだけ使う系統は、指す範囲を名前付きグループ `m` で囲む。
const PATTERNS: [&str; 8] = [
    "立ち止ま",
    "という行為",
    "(?P<m>た瞬間)[、に]",
    "少しだけ",
    "静かに",
    "ささやかな",
    "(?P<m>ふと)[、気思見目]",
    "(?P<m>私たち)(?:は|の|が)",
];

static FAMILIES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    PATTERNS
        .iter()
        .map(|p| Regex::new(p).expect("R22 の系統の正規表現"))
        .collect()
});

/// 窓に詰める字数の上限 (文の [`Sentence::length`] の和)。
const WINDOW_CHARS: usize = 1300;

/// 窓の中の、ある系統の表現の一致。
#[derive(Debug, Clone)]
struct Hit {
    family: usize,
    /// 原文上の指す範囲。
    span: Span,
    /// 解析用テキストで一致した語。強調の記号が語の途中にあっても、原文の範囲と違い記号を含まない。
    text: String,
}

/// 近い範囲の地の文 (窓)。
#[derive(Debug, Default)]
struct Window {
    /// 窓に入れた文の字数の和。
    chars: usize,
    /// 窓の最初の文の始まりから最後の文の終わりまで (原文)。
    span: Option<Span>,
    /// 原文の順の一致。
    hits: Vec<Hit>,
}

impl Window {
    /// 系統ごとの最初の一致 (原文の順)。数が窓の系統の数になる。
    fn firsts(&self) -> Vec<Hit> {
        let mut firsts: Vec<Hit> = Vec::new();
        for hit in &self.hits {
            if !firsts.iter().any(|h| h.family == hit.family) {
                firsts.push(hit.clone());
            }
        }
        firsts
    }
}

/// 地の文を窓に分ける。
struct Windows<'a> {
    doc: &'a Document,
    done: Vec<Window>,
    current: Window,
}

impl<'a> Windows<'a> {
    fn close(&mut self) {
        let window = std::mem::take(&mut self.current);
        if window.span.is_some() {
            self.done.push(window);
        }
    }

    fn push(&mut self, block: &Block, decorations: &Disjoint, sentence: &Sentence) {
        let current = &mut self.current;
        current.chars += sentence.length;
        current.span = Some(match current.span {
            Some(span) => Span::new(span.start, sentence.span.end),
            None => sentence.span,
        });
        current
            .hits
            .extend(hits(self.doc, block, decorations, sentence));
    }

    /// 同じ段落の中で続く対象の文のまとまりを詰める。入りきらなければ窓を閉じてから入れ、
    /// まとまりが上限を超えるなら文の境界で分ける。上限を超える一文は、その文だけで窓にする。
    fn push_run(&mut self, block: &Block, decorations: &Disjoint, run: &[Sentence]) {
        let chars: usize = run.iter().map(|s| s.length).sum();
        if self.current.chars + chars > WINDOW_CHARS {
            self.close();
        }
        for sentence in run {
            if sentence.length > WINDOW_CHARS {
                self.close();
                self.push(block, decorations, sentence);
                self.close();
                continue;
            }
            if self.current.chars + sentence.length > WINDOW_CHARS {
                self.close();
            }
            self.push(block, decorations, sentence);
        }
    }
}

/// 窓に入れる文か: 日本語を含み、「ラベル: 値」の行でない。
fn eligible(sentence: &Sentence) -> bool {
    sentence.japanese && !sentence.label_value
}

/// 地の文を、原文の順に窓へ分ける。
///
/// 地の文でないブロック (見出し・リスト・表・引用・脚注)、ブロックの間に空白以外の原文が
/// ある箇所 (コード・HTML・区切り線)、日本語を含まない文、「ラベル: 値」の行で窓を区切る。
fn windows(doc: &Document) -> Vec<Window> {
    let mut windows = Windows {
        doc,
        done: Vec::new(),
        current: Window::default(),
    };
    let mut previous_end = 0;
    for (index, block) in doc.blocks.iter().enumerate() {
        let gap = doc.source.get(previous_end..block.span.start).unwrap_or("");
        if !block.is_prose() || !gap.trim().is_empty() {
            windows.close();
        }
        previous_end = previous_end.max(block.span.end);
        if !block.is_prose() {
            continue;
        }
        // 強調以外の装飾 (リンク・コード・画像・数式・取り消し線) の原文の範囲。ブロックごとに 1 度だけ作る
        let decorations = Disjoint::new(
            block
                .marks
                .iter()
                .filter(|mark| !matches!(mark.kind, MarkKind::Strong | MarkKind::Emphasis))
                .map(|mark| mark.span.range())
                .collect(),
        );
        // 対象外の文の位置で窓を閉じ、その後の文は次の窓に入れる
        for (i, run) in doc
            .block_sentences(index)
            .split(|s| !eligible(s))
            .enumerate()
        {
            if i > 0 {
                windows.close();
            }
            windows.push_run(block, &decorations, run);
        }
    }
    windows.close();
    windows.done
}

/// 重なりを調べる範囲の集まり。始まりの順に並べ、重なる範囲をまとめておき、二分探索で引く
/// (一致ごとにすべての範囲を調べると、括弧やリンクの多い長い段落で二乗になる)。
struct Disjoint(Vec<Range<usize>>);

impl Disjoint {
    fn new(mut ranges: Vec<Range<usize>>) -> Self {
        ranges.sort_by_key(|r| r.start);
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for r in ranges {
            match merged.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => merged.push(r),
            }
        }
        Self(merged)
    }

    /// `range` と一部でも重なる範囲があるか。
    fn overlaps(&self, range: &Range<usize>) -> bool {
        // まとめた範囲は終わりも昇順なので、終わりが `range` の始まりより後の最初の範囲だけを見ればよい
        let i = self.0.partition_point(|r| r.end <= range.start);
        self.0.get(i).is_some_and(|r| r.start < range.end)
    }
}

/// 文の中の括弧の内側 (解析用テキストの範囲)。閉じない括弧は文末までを内側とみなす。
///
/// 閉じ括弧は、対応する最も内側の開き括弧まで戻って閉じる (内側に閉じない括弧が残っていても、
/// 外側の引用は閉じる。語句ルールの引用の範囲 `QuoteRanges` と同じ)。
fn bracket_interiors(text: &str) -> Disjoint {
    let mut interiors = Vec::new();
    let mut open = OpenStack::default();
    for (at, c) in text.char_indices() {
        // 〝 は 〟 と 〞 のどちらでも閉じる (文分割とラベルの行の判定と同じ)
        let c = if c == '〞' { '〟' } else { c };
        if let Some(close) = text::closing_bracket(c) {
            open.push(at, close);
        } else if let Some(start) = open.close(c) {
            interiors.push(start..at + c.len_utf8());
        }
    }
    interiors.extend(open.unclosed().map(|start| start..text.len()));
    Disjoint::new(interiors)
}

/// 文の中の系統の一致 (原文の順)。
///
/// 括弧 (会話の「」・補足の（）など) の内側と、強調以外の装飾 (リンク・コード・画像・数式・
/// 取り消し線) に重なる一致は数えない。重なりは正規表現の一致全体で見る。
fn hits(doc: &Document, block: &Block, decorations: &Disjoint, sentence: &Sentence) -> Vec<Hit> {
    let text = doc.sentence_text(sentence);
    let interiors = bracket_interiors(text);
    let to_source = |range: Range<usize>| {
        block.to_source(sentence.range.start + range.start..sentence.range.start + range.end)
    };
    let mut hits = Vec::new();
    for (family, pattern) in FAMILIES.iter().enumerate() {
        for captures in pattern.captures_iter(text) {
            let whole = captures.get(0).expect("一致の全体");
            if interiors.overlaps(&whole.range())
                || decorations.overlaps(&to_source(whole.range()).range())
            {
                continue;
            }
            let shown = captures.name("m").unwrap_or(whole);
            hits.push(Hit {
                family,
                span: to_source(shown.range()),
                text: shown.as_str().to_string(),
            });
        }
    }
    hits.sort_by_key(|h| h.span.start);
    hits
}

pub struct EssayPhraseCluster {
    min_families: usize,
}

impl Default for EssayPhraseCluster {
    fn default() -> Self {
        Self { min_families: 2 }
    }
}

impl Rule for EssayPhraseCluster {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn allowed_in(&self, genre: Genre) -> bool {
        genre == Genre::Essay
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        if key != "min_families" {
            return Err(unknown_option(&META, key));
        }
        let n = option_count(key, value)?;
        if !(2..=PATTERNS.len()).contains(&n) {
            return Err(format!(
                "`min_families` には 2 以上 {} 以下の整数を指定してください",
                PATTERNS.len()
            ));
        }
        self.min_families = n;
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![("min_families", self.min_families.to_string())]
    }

    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let most = windows(ctx.doc)
            .iter()
            .map(|w| w.firsts().len())
            .max()
            .unwrap_or(0);
        if most == 0 {
            return Vec::new();
        }
        vec![Measure::new(
            "max_families",
            most as f64,
            "min_families",
            Fires::AtOrAbove,
        )]
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for window in windows(ctx.doc) {
            let firsts = window.firsts();
            if firsts.len() < self.min_families {
                continue;
            }
            let (Some(context), Some(first)) = (window.span, window.hits.first()) else {
                continue;
            };
            let shown: Vec<String> = firsts.iter().map(|h| format!("「{}」", h.text)).collect();
            out.push(
                META.diagnostic(
                    first.span,
                    format!(
                        "随筆によく出る表現が、{} 字の範囲に {} 系統重なっています ({})",
                        window.chars,
                        firsts.len(),
                        shown.join("")
                    ),
                )
                .with_hint("人の随筆にもある普通の表現です。場面や語り口に合っていれば、そのまま残してください。別の語に置き換えたり、事実や感情を書き足したりする必要はありません")
                .with_context(context)
                .with_related(firsts.iter().skip(1).map(|h| h.span).collect())
                .with_metric("window_chars", window.chars)
                .with_metric("families", firsts.len()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::rules::RuleUnit;
    use crate::rules::testing::{matched, measure_md, run};

    /// 2 系統 (「立ち止ま」「静かに」) が同じ窓に入る文書。
    const TWO: &str = "駅の階段の途中で立ち止まった。\n\n空は静かに暮れていった。\n";

    fn configured(min_families: i64) -> EssayPhraseCluster {
        let mut rule = EssayPhraseCluster::default();
        rule.configure("min_families", &toml::Value::Integer(min_families))
            .unwrap();
        rule
    }

    fn related<'a>(source: &'a str, d: &Diagnostic) -> Vec<&'a str> {
        d.related.iter().map(|s| &source[s.range()]).collect()
    }

    #[test]
    fn points_to_the_first_phrase_of_a_window_with_two_families() {
        let d = run(&EssayPhraseCluster::default(), TWO);
        assert_eq!(matched(TWO, &d), ["立ち止ま"]);
        assert_eq!(related(TWO, &d[0]), ["静かに"]);
        assert_eq!(d[0].severity, Severity::Info);
        assert_eq!(d[0].signal, Some(Signal::Strong));
        assert_eq!(d[0].lane, Lane::Slop);
        // 文脈は窓の最初の文の始まりから最後の文の終わりまで (部分の検査で窓の後ろの編集にも残る)
        let context = d[0].context.unwrap();
        assert_eq!(
            &TWO[context.range()],
            "駅の階段の途中で立ち止まった。\n\n空は静かに暮れていった。"
        );
        assert!(d[0].message.contains("2 系統"), "{}", d[0].message);
        assert!(
            d[0].message.contains("「立ち止ま」「静かに」"),
            "{}",
            d[0].message
        );
        // 1 系統だけなら指摘しない
        assert!(
            run(
                &EssayPhraseCluster::default(),
                "駅の階段の途中で立ち止まった。\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn counts_each_family_once_and_one_diagnostic_per_window() {
        let repeated = "静かに歩いた。静かに座った。静かに眠った。\n";
        assert!(run(&EssayPhraseCluster::default(), repeated).is_empty());
        assert_eq!(
            measure_md(&EssayPhraseCluster::default(), repeated)[0].value,
            1.0
        );
        let three =
            "ふと、窓の外を見た。静かに雨が降っていた。少しだけ眠った。静かに目を閉じた。\n";
        let d = run(&EssayPhraseCluster::default(), three);
        assert_eq!(d.len(), 1);
        assert_eq!(matched(three, &d), ["ふと"]);
        assert_eq!(related(three, &d[0]), ["静かに", "少しだけ"]);
        assert!(d[0].message.contains("3 系統"), "{}", d[0].message);
    }

    #[test]
    fn min_families_raises_the_threshold_and_measure_reports_the_maximum() {
        let three = "ふと、窓の外を見た。静かに雨が降っていた。少しだけ眠った。\n";
        assert_eq!(run(&configured(3), three).len(), 1);
        assert!(run(&configured(3), TWO).is_empty());
        let m = measure_md(&configured(3), TWO);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].name, "max_families");
        assert_eq!(m[0].value, 2.0);
        assert_eq!(m[0].threshold_key, "min_families");
        assert!(measure_md(&EssayPhraseCluster::default(), "記録を読んだ。\n").is_empty());
    }

    #[test]
    fn configure_accepts_only_two_to_eight_families() {
        let mut rule = EssayPhraseCluster::default();
        for bad in [0, 1, 9] {
            assert!(
                rule.configure("min_families", &toml::Value::Integer(bad))
                    .is_err()
            );
        }
        assert!(
            rule.configure("min_count", &toml::Value::Integer(2))
                .is_err()
        );
        for good in 2..=8 {
            assert!(
                rule.configure("min_families", &toml::Value::Integer(good))
                    .is_ok()
            );
        }
        assert_eq!(rule.options(), [("min_families", "8".to_string())]);
    }

    #[test]
    fn windows_close_after_about_1300_chars() {
        // 7 字の文を並べた長い段落を挟むと、前後の表現は別の窓になる
        let filler = "記録を読み返した。".repeat(200);
        let apart =
            format!("駅の階段の途中で立ち止まった。\n\n{filler}\n\n空は静かに暮れていった。\n");
        assert!(run(&EssayPhraseCluster::default(), &apart).is_empty());
        let near = format!(
            "{}\n\n駅の階段の途中で立ち止まった。\n\n空は静かに暮れていった。\n",
            "記録を読み返した。".repeat(10)
        );
        assert_eq!(run(&EssayPhraseCluster::default(), &near).len(), 1);
        // 次の段落を入れると上限を超えるなら、前の窓を閉じてから入れる
        let full = format!(
            "駅の階段の途中で立ち止まった。{}\n\n空は静かに暮れていった。\n",
            "記録を読み返した。".repeat(160)
        );
        assert!(run(&EssayPhraseCluster::default(), &full).is_empty());
    }

    #[test]
    fn a_sentence_over_the_limit_is_a_window_of_its_own() {
        let long = format!("{}立ち止まって静かに", "長".repeat(1400));
        let alone = format!("{long}眺めた。\n");
        assert_eq!(run(&EssayPhraseCluster::default(), &alone).len(), 1);
        // 長い一文の後ろの文は次の窓に入る
        let split = format!("{}立ち止まった。空は静かに暮れた。\n", "長".repeat(1400));
        assert!(run(&EssayPhraseCluster::default(), &split).is_empty());
    }

    #[test]
    fn structure_closes_the_window_and_is_not_counted() {
        for doc in [
            "駅の階段の途中で立ち止まった。\n\n## 帰り道\n\n空は静かに暮れていった。\n",
            "駅の階段の途中で立ち止まった。\n\n> 空は静かに暮れていった。\n",
            "駅の階段の途中で立ち止まった。\n\n- 空は静かに暮れていった。\n",
            "駅の階段の途中で立ち止まった。\n\n| 空 |\n|---|\n| 静かに暮れた |\n",
            "駅の階段の途中で立ち止まった。\n\n```\n静かに\n```\n\n空は暮れていった。\n",
            "駅の階段の途中で立ち止まった。\n\n```\nlog\n```\n\n空は静かに暮れていった。\n",
            "駅の階段の途中で立ち止まった。\n\n---\n\n空は静かに暮れていった。\n",
            "駅の階段の途中で立ち止まった。\n\n**場所**: 駅前\n\n空は静かに暮れていった。\n",
            "駅の階段の途中で立ち止まった。\n**場所**: 駅前  \n空は静かに暮れていった。\n",
            "駅の階段の途中で立ち止まった。[^1]\n\n[^1]: 空は静かに暮れていった。\n",
        ] {
            assert!(run(&EssayPhraseCluster::default(), doc).is_empty(), "{doc}");
        }
    }

    #[test]
    fn phrases_in_brackets_and_decorations_are_not_counted() {
        for doc in [
            "「少しだけ待って」と言われ、駅の階段の途中で立ち止まった。\n",
            "駅の階段の途中で立ち止まった（静かに暮れる空を見ていた）。\n",
            "駅の階段の途中で立ち止まった。（静かに暮れる空を見ていた\n",
            "駅の階段の途中で立ち止まった。[静かに](https://example.com)暮れた。\n",
            "駅の階段の途中で立ち止まった。`静かに`と書いた。\n",
            "駅の階段の途中で立ち止まった。~~静かに~~暮れた。\n",
        ] {
            assert!(run(&EssayPhraseCluster::default(), doc).is_empty(), "{doc}");
        }
        // 強調の中は数え、原文上の位置は記号の内側を指す
        let strong = "駅の階段の途中で立ち止まった。空は**静かに**暮れていった。\n";
        let d = run(&EssayPhraseCluster::default(), strong);
        assert_eq!(related(strong, &d[0]), ["静かに"]);
    }

    #[test]
    fn near_misses_of_the_families_do_not_match() {
        for doc in [
            "ふとした拍子に、私たちも駅で待った。\n",
            "目が合った瞬間。行為を見た。\n",
            "立ちどまった。静か な夜だった。\n",
        ] {
            assert!(
                measure_md(&EssayPhraseCluster::default(), doc).is_empty(),
                "{doc}"
            );
        }
        // 前後の文字を条件にする系統は、語だけを指す
        let doc = "目が合った瞬間、私たちは笑った。\n";
        let d = run(&EssayPhraseCluster::default(), doc);
        assert_eq!(matched(doc, &d), ["た瞬間"]);
        assert_eq!(related(doc, &d[0]), ["私たち"]);
    }

    /// 窓の上限ちょうど (1,300 字) までは同じ窓に入れ、1 字でも超えれば次の窓に送る。
    /// 段落を足して超えるときは、後ろの段落を丸ごと次の窓に送る。
    #[test]
    fn the_window_limit_is_inclusive_and_moves_whole_paragraphs() {
        let rule = EssayPhraseCluster::default();
        let at = |n: usize, tail: &str| {
            format!("{}立ち止まった。{tail}\n\n静かに歩いた。\n", "文".repeat(n))
        };
        assert_eq!(run(&rule, &at(1288, "")).len(), 1);
        assert!(run(&rule, &at(1289, "")).is_empty());
        assert!(run(&rule, &at(1288, "雨だ。")).is_empty());
    }

    #[test]
    fn non_japanese_sentences_split_windows_and_measure_takes_the_largest_window() {
        let rule = EssayPhraseCluster::default();
        let english = "駅の階段の途中で立ち止まった。Really? 空は静かに暮れていった。\n";
        assert!(run(&rule, english).is_empty());
        assert_eq!(measure_md(&rule, english)[0].value, 1.0);
        // 窓をまたいで系統を合算しない
        let two_windows = "立ち止まった。静かに歩いた。\n\n---\n\nふと、空を見た。少しだけ眠った。私たちは笑った。\n";
        assert_eq!(run(&rule, two_windows).len(), 2);
        assert_eq!(measure_md(&rule, two_windows)[0].value, 3.0);
    }

    /// 内側に閉じない括弧が残っていても、外側の引用の閉じで引用を閉じ、その後ろは数える。
    #[test]
    fn an_outer_quote_closes_over_an_unclosed_inner_bracket() {
        let doc = "「画面には（と出た」と聞き、立ち止まって静かに待った。\n";
        let d = run(&EssayPhraseCluster::default(), doc);
        assert_eq!(matched(doc, &d), ["立ち止ま"]);
        // 閉じない外側の括弧は、これまでどおり文末までを内側とみなす
        assert!(
            run(
                &EssayPhraseCluster::default(),
                "（立ち止まって静かに待った。\n"
            )
            .is_empty()
        );
    }

    #[test]
    fn disjoint_ranges_answer_overlaps_like_a_linear_scan() {
        let ranges = vec![10..14, 2..5, 4..8, 20..20, 30..40];
        let disjoint = Disjoint::new(ranges.clone());
        for start in 0..45 {
            for end in start..46 {
                let linear = ranges.iter().any(|r| r.start < end && start < r.end);
                assert_eq!(disjoint.overlaps(&(start..end)), linear, "{start}..{end}");
            }
        }
    }

    #[test]
    fn typographic_quotes_closed_by_either_mark_are_excluded() {
        for close in ['〟', '〞'] {
            let doc = format!("〝見送り{close}を思い出し、立ち止まって静かに待った。\n");
            assert_eq!(
                run(&EssayPhraseCluster::default(), &doc).len(),
                1,
                "{close}"
            );
            let inside = format!("〝静かに{close}を思い出し、立ち止まった。\n");
            assert!(
                run(&EssayPhraseCluster::default(), &inside).is_empty(),
                "{close}"
            );
        }
    }

    /// 強調の記号が語の途中にあっても、メッセージには記号を含めない (位置は原文の範囲のまま)。
    #[test]
    fn messages_quote_the_matched_words_without_markup() {
        let doc = "席で立ち止まった。窓の外を**静か**に眺めた。\n";
        let d = run(&EssayPhraseCluster::default(), doc);
        assert!(
            d[0].message.contains("「立ち止ま」「静かに」"),
            "{}",
            d[0].message
        );
        assert_eq!(related(doc, &d[0]), ["静か**に"]);
    }

    fn engine(genre: Genre) -> crate::engine::Engine {
        crate::engine::Engine::new(crate::engine::EngineOptions {
            genre,
            ..Default::default()
        })
        .unwrap()
    }

    fn r22_count(report: crate::engine::FileReport) -> usize {
        report
            .diagnostics
            .iter()
            .filter(|d| d.rule_id == "R22")
            .count()
    }

    #[test]
    fn the_engine_runs_it_on_essays_by_default() {
        assert_eq!(
            r22_count(engine(Genre::Essay).lint(Document::markdown(TWO))),
            1
        );
        assert_eq!(
            r22_count(engine(Genre::General).lint(Document::markdown(TWO))),
            0
        );
    }

    /// コードのコメントのような断片の集まりには当てない。
    #[cfg(feature = "code")]
    #[test]
    fn the_engine_does_not_run_it_on_fragments() {
        let comments = Document::parse(
            "sample.rs",
            "// 駅の階段の途中で立ち止まった。\n// 空は静かに暮れていった。\n",
            crate::document::SourceFormat::Code(crate::language::CodeLanguage::Rust),
            &crate::document::ParseOptions::default(),
        );
        assert_eq!(comments.kind, crate::document::DocumentKind::Fragments);
        assert_eq!(r22_count(engine(Genre::Essay).lint(comments)), 0);
    }

    #[test]
    fn only_essays_and_whole_documents_are_checked() {
        let rule = EssayPhraseCluster::default();
        assert!(rule.allowed_in(Genre::Essay));
        for genre in [Genre::General, Genre::Tech, Genre::Business] {
            assert!(!rule.allowed_in(genre), "{genre}");
        }
        assert_eq!(rule.unit(), RuleUnit::Document);
        assert!(!rule.requires_full_document());
        // テキストの文書でも同じ窓で数える
        let doc =
            Document::plain_text("駅の階段の途中で立ち止まった。\n\n空は静かに暮れていった。\n");
        let ctx = RuleContext::new(&doc);
        let mut out = Vec::new();
        rule.check(&ctx, &mut out);
        assert_eq!(out.len(), 1);
    }
}
