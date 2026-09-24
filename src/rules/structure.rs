//! 構造系ルール (`S` で始まる ID)。
//!
//! 地の文ではなく Markdown の体裁 (太字・箇条書き・見出し・段階表現・絵文字) に現れる、
//! 教科書的な構成の癖を拾う。どれも定量的な校正の前なので実験的 (既定で無効) で、
//! ビジネス文書で正当に使われる体裁 (S01〜S04・S07・S09・S10) はジャンル business では
//! 既定で動かさない。

use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::document::{Block, BlockKind, MarkKind};
use crate::genre::Genre;
use crate::heading::HeadingShape;
use crate::rules::{Fires, Measure, Rule, RuleContext, RuleMeta, option_f64, option_usize, quote};
use crate::text;

/// 構造系の組み込みルール。
pub fn rules(_genre: Genre) -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(BoldDensity::new()),
        Box::new(BulletRatio::new()),
        Box::new(BoilerplateHeading),
        Box::new(NumberedPhases::new()),
        Box::new(EmojiDensity::new()),
        Box::new(BoldLabelList::new()),
        Box::new(HeadingTemplate::new()),
        Box::new(HeadingEmphasis),
        Box::new(StructureDensity::new()),
        Box::new(LabelStyle::new()),
    ]
}

fn unknown_option(meta: &RuleMeta, key: &str) -> String {
    format!("{} に `{key}` という設定項目はありません", meta.id)
}

fn option_count(key: &str, value: &toml::Value) -> Result<usize, String> {
    match option_usize(key, value)? {
        0 => Err(format!("`{key}` には 1 以上の整数を指定してください")),
        n => Ok(n),
    }
}

fn option_positive(key: &str, value: &toml::Value) -> Result<f64, String> {
    let v = option_f64(key, value)?;
    if v < 0.0 || !v.is_finite() {
        return Err(format!("`{key}` には 0 以上の数値を指定してください"));
    }
    Ok(v)
}

/// 設定値を 0 以上 1 以下の比率として読む。
fn option_ratio(key: &str, value: &toml::Value) -> Result<f64, String> {
    let v = option_positive(key, value)?;
    if v > 1.0 {
        return Err(format!(
            "`{key}` には 0 以上 1 以下の数値を指定してください"
        ));
    }
    Ok(v)
}

/// ブロックの本文 (前後の空白を除く) と、その原文上の範囲。
fn trimmed_text(block: &Block) -> (Span, &str) {
    let text = block.text.trim();
    let start = block.text.len() - block.text.trim_start().len();
    (block.to_source(start..start + text.len()), text)
}

/// 原文 1000 字あたりの件数。
fn per_thousand(count: usize, chars: usize) -> f64 {
    count as f64 * 1000.0 / chars.max(1) as f64
}

/// 数えた箇所の数を `min_count` と比べる測定値 (以上で指摘する)。1 つもない文書は、
/// `min_count` (1 以上) をどう変えても指摘しないので値を返さない。
fn count_measure(name: &'static str, count: usize) -> Vec<Measure> {
    if count == 0 {
        return Vec::new();
    }
    vec![Measure::new(
        name,
        count as f64,
        "min_count",
        Fires::AtOrAbove,
    )]
}

/// 文書中の太字 (Strong) の原文上の範囲。
fn bold_spans(ctx: &RuleContext<'_>) -> Vec<Span> {
    ctx.doc
        .blocks
        .iter()
        .flat_map(|b| b.marks.iter())
        .filter(|m| m.kind == MarkKind::Strong)
        .map(|m| m.span)
        .collect()
}

/// 本文のブロック (段落とリスト項目) の数と、そのうちのリスト項目。
fn body_blocks<'a>(ctx: &RuleContext<'a>) -> (usize, Vec<&'a Block>) {
    let blocks: Vec<&Block> = ctx
        .doc
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Paragraph | BlockKind::ListItem))
        .collect();
    let items = blocks
        .iter()
        .copied()
        .filter(|b| b.kind == BlockKind::ListItem)
        .collect();
    (blocks.len(), items)
}

// ---------------------------------------------------------------------------
// S01 BOLD_DENSITY
// ---------------------------------------------------------------------------

static BOLD_META: RuleMeta = RuleMeta {
    id: "S01",
    name: "BOLD_DENSITY",
    title: "太字の多用",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "太字が原文 1000 字あたり 3 箇所以上あり、強調が散らばって効かなくなっている",
    explanation: "\
### 何を見るか

太字 (`**…**`) が 3 箇所以上あり、原文 1000 字あたり 3 箇所以上の密度で使われている文書を指します。

### なぜ問題か

太字は、ほかが装飾されていないから目立ちます。段落ごとに太字を置くと、強調がどこにでもあって\
どこにもない状態になります。チャットの画面で強調を多用する癖が、文書にそのまま持ち込まれた形でも\
あります。

### 直し方

太字は文書の核になる 1〜2 箇所に絞ってください。強調したい内容は、短い一文で言い切るなど、文の\
組み立てで示します。

### 例

- 直す前: 「**大切なのは**、**早めの申請**と**正確な記入**です。」
- 直した後: 「申請は締め日の 3 日前までに出してください。」

### 根拠

実験的です。生成文書の精読で繰り返し見つかった癖ですが、コーパスでの定量的な校正はしていません。\
ビジネス文書では太字の強調が正当な慣習なので、ジャンル business では既定で動かしません。
",
};

/// S01 BOLD_DENSITY。
pub struct BoldDensity {
    min_count: usize,
    per_1000: f64,
}

impl BoldDensity {
    pub fn new() -> Self {
        Self {
            min_count: 3,
            per_1000: 3.0,
        }
    }
}

impl Default for BoldDensity {
    fn default() -> Self {
        Self::new()
    }
}

impl Rule for BoldDensity {
    fn meta(&self) -> &'static RuleMeta {
        &BOLD_META
    }

    fn allowed_in(&self, genre: Genre) -> bool {
        genre != Genre::Business
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_count" => self.min_count = option_count(key, value)?,
            "per_1000" => self.per_1000 = option_positive(key, value)?,
            _ => return Err(unknown_option(&BOLD_META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_count", self.min_count.to_string()),
            ("per_1000", self.per_1000.to_string()),
        ]
    }

    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let n = bold_spans(ctx).len();
        if n < self.min_count {
            return Vec::new();
        }
        vec![Measure::new(
            "bold_per_1000",
            per_thousand(n, ctx.doc.char_count()),
            "per_1000",
            Fires::AtOrAbove,
        )]
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let spans = bold_spans(ctx);
        let density = per_thousand(spans.len(), ctx.doc.char_count());
        if spans.len() < self.min_count || density < self.per_1000 {
            return;
        }
        out.push(
            BOLD_META
                .diagnostic(
                    spans[0],
                    format!(
                        "太字が {} 箇所あります (原文 1000 字あたり {density:.1} 箇所)",
                        spans.len()
                    ),
                )
                .with_hint("太字は文書の核になる 1〜2 箇所に絞り、強調は短い一文で言い切るなど文の組み立てで示してください")
                .with_related(spans.clone())
                .with_metric("count", spans.len())
                .with_metric("per_1000", density),
        );
    }
}

// ---------------------------------------------------------------------------
// S02 BULLET_RATIO
// ---------------------------------------------------------------------------

static BULLET_META: RuleMeta = RuleMeta {
    id: "S02",
    name: "BULLET_RATIO",
    title: "箇条書きへの偏り",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "本文の 35% 以上が箇条書きで、文章で説明すべき因果まで刻まれている疑いがある",
    explanation: "\
### 何を見るか

段落と箇条書きの項目を合わせて 10 以上ある文書で、箇条書きの項目が 35% 以上を占めるときに 1 件\
出します。

### なぜ問題か

箇条書きは、書き手が項目どうしの論理を組み立てずに済む書き方です。その分、つながりを読み手に\
預けることになります。原因と結果の流れまで箇条書きに切り分けると、書き手が筋道を組み立てずに\
済ませたように読まれます。

### 直し方

項目どうしを「だから」「ところが」でつなげられないか試し、つながるなら地の文に戻してください。項目が本当に並列で、あとから探す価値のある一覧 (決定事項・宿題) はそのままで\
かまいません。

### 例

- 直す前: 「- 雨で来場者が減った / - 物販の売上が落ちた / - 次回の出店を見送った」
- 直した後: 「雨で来場者が減り、物販の売上も落ちたため、次回の出店は見送った。」

### 根拠

実験的です。定量的な校正はしていません。議事録や報告書では箇条書きが正当に多いため、ジャンル \
business では既定で動かしません。
",
};

/// S02 BULLET_RATIO。
pub struct BulletRatio {
    min_blocks: usize,
    ratio_threshold: f64,
}

impl BulletRatio {
    pub fn new() -> Self {
        Self {
            min_blocks: 10,
            ratio_threshold: 0.35,
        }
    }
}

impl Default for BulletRatio {
    fn default() -> Self {
        Self::new()
    }
}

impl Rule for BulletRatio {
    fn meta(&self) -> &'static RuleMeta {
        &BULLET_META
    }

    fn allowed_in(&self, genre: Genre) -> bool {
        genre != Genre::Business
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_blocks" => self.min_blocks = option_count(key, value)?,
            "ratio_threshold" => self.ratio_threshold = option_ratio(key, value)?,
            _ => return Err(unknown_option(&BULLET_META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_blocks", self.min_blocks.to_string()),
            ("ratio_threshold", self.ratio_threshold.to_string()),
        ]
    }

    /// 本文のブロックが `min_blocks` 以上ある文書で、箇条書きの項目の割合を返す。項目が 1 つも
    /// ない文書は、閾値をどう変えても指摘しないので値を返さない。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let (blocks, items) = body_blocks(ctx);
        if blocks < self.min_blocks || items.is_empty() {
            return Vec::new();
        }
        vec![Measure::new(
            "list_ratio",
            items.len() as f64 / blocks as f64,
            "ratio_threshold",
            Fires::AtOrAbove,
        )]
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let (blocks, items) = body_blocks(ctx);
        if blocks < self.min_blocks {
            return;
        }
        let ratio = items.len() as f64 / blocks as f64;
        if items.is_empty() || ratio < self.ratio_threshold {
            return;
        }
        out.push(
            BULLET_META
                .diagnostic(
                    items[0].to_source(0..items[0].text.len()),
                    format!(
                        "本文のブロック {} 個のうち {} 個 ({:.0}%) が箇条書きです",
                        blocks,
                        items.len(),
                        ratio * 100.0
                    ),
                )
                .with_hint("項目の間に因果や経緯が隠れていないか確かめ、隠れていれば地の文に戻してください")
                .with_metric("blocks", blocks)
                .with_metric("list_items", items.len())
                .with_metric("ratio", ratio),
        );
    }
}

// ---------------------------------------------------------------------------
// S03 BOILERPLATE_HEADING
// ---------------------------------------------------------------------------

static HEADING_META: RuleMeta = RuleMeta {
    id: "S03",
    name: "BOILERPLATE_HEADING",
    title: "定型見出しでの締め",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "「まとめ」「おわりに」など、中身ではなく構成の型を示す見出しで締めている",
    explanation: "\
### 何を見るか

「まとめ」「おわりに」「終わりに」「さいごに」「最後に」「結論」「総括」「Conclusion」で始まる見出しを\
指します。

### なぜ問題か

見出しが「まとめ」だと、見出しだけを拾い読みしても最後に何が言われるのかが分かりません。中身では\
なく構成の型で文書を締める、教科書的な組み立ての名残でもあります。

### 直し方

見出しに節の結論そのものを入れてください。結びの節が本文の繰り返しにすぎないなら、節ごと削る\
ことも検討します。

### 例

- 直す前: 「## まとめ」
- 直した後: 「## 差し戻しの半分は入力画面の改修で防げる」

### 根拠

実験的です。定量的な校正はしていません。ビジネス文書では定型の見出しが正当な慣習なので、ジャンル \
business では既定で動かしません。
",
};

/// 定型の締めの見出し (前方一致。英字は大文字小文字を無視する)。
const BOILERPLATE_HEADINGS: &[&str] = &[
    "まとめ",
    "おわりに",
    "終わりに",
    "さいごに",
    "最後に",
    "結論",
    "総括",
    "conclusion",
];

/// S03 BOILERPLATE_HEADING。
pub struct BoilerplateHeading;

impl Rule for BoilerplateHeading {
    fn meta(&self) -> &'static RuleMeta {
        &HEADING_META
    }

    fn allowed_in(&self, genre: Genre) -> bool {
        genre != Genre::Business
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for block in &ctx.doc.blocks {
            if !block.is_heading() {
                continue;
            }
            let (span, heading) = trimmed_text(block);
            let lower = heading.to_lowercase();
            let Some(word) = BOILERPLATE_HEADINGS.iter().find(|w| lower.starts_with(**w)) else {
                continue;
            };
            let heading = quote(heading);
            out.push(
                HEADING_META
                    .diagnostic(
                        span,
                        format!("見出し「{heading}」は、中身ではなく構成の型 (「{word}」) を示す見出しです"),
                    )
                    .with_hint("見出しに節の結論そのものを入れてください")
                    .with_metric("heading", heading),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// S04 NUMBERED_PHASES
// ---------------------------------------------------------------------------

static PHASE_META: RuleMeta = RuleMeta {
    id: "S04",
    name: "NUMBERED_PHASES",
    title: "番号付きの段階",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "「フェーズ1」「ステップ2」のような番号付きの段階表現が 3 回以上ある",
    explanation: "\
### 何を見るか

「フェーズ1」「ステップ2」「ステージ3」のような番号付きの段階表現が、見出しや箇条書きを含む文書\
全体で 3 回以上出てくるときに、最初の位置で 1 件出します。

### なぜ問題か

内容がもともと順序を持っていないのに段階を刻み、「まずこれ、次にこれ」と並べるのは、生成された\
文章の癖の一つです。並列の話題に順番を付けると、主張の趣旨まで変わってしまいます。

### 直し方

内容が本当に順序を持つかを確かめてください。持たないなら段階の番号を外し、並列のまま書きます。

### 例

- 直す前: 「フェーズ1: 現状を把握する / フェーズ2: 課題を洗い出す / フェーズ3: 改善策を実行する」
- 直した後: 「問い合わせの記録から多い質問を 10 件選び、手順書で答えられるものから直していく。」

### 根拠

実験的です。定量的な校正はしていません。ビジネス文書では工程の段階表現が正当に使われるため、\
ジャンル business では既定で動かしません。
",
};

static PHASE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:フェーズ|ステップ|段階|ステージ)\s*[0-9０-９一二三四五六七八九]")
        .expect("phase regex")
});

/// 文書全体 (見出し・箇条書きを含む) の番号付きの段階表現の原文上の範囲。
fn phase_spans(ctx: &RuleContext<'_>) -> Vec<Span> {
    ctx.doc
        .blocks
        .iter()
        .flat_map(|b| {
            PHASE_RE
                .find_iter(&b.text)
                .map(move |m| b.to_source(m.range()))
        })
        .collect()
}

/// S04 NUMBERED_PHASES。
pub struct NumberedPhases {
    min_count: usize,
}

impl NumberedPhases {
    pub fn new() -> Self {
        Self { min_count: 3 }
    }
}

impl Default for NumberedPhases {
    fn default() -> Self {
        Self::new()
    }
}

impl Rule for NumberedPhases {
    fn meta(&self) -> &'static RuleMeta {
        &PHASE_META
    }

    fn allowed_in(&self, genre: Genre) -> bool {
        genre != Genre::Business
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_count" => self.min_count = option_count(key, value)?,
            _ => return Err(unknown_option(&PHASE_META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![("min_count", self.min_count.to_string())]
    }

    /// 段階表現の数を `min_count` と比べる値として返す。1 つもない文書は値を返さない。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        count_measure("phase_count", phase_spans(ctx).len())
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let spans = phase_spans(ctx);
        if spans.len() < self.min_count {
            return;
        }
        out.push(
            PHASE_META
                .diagnostic(
                    spans[0],
                    format!("「フェーズ1」のような番号付きの段階表現が {} 回あります", spans.len()),
                )
                .with_hint("内容が本当に順序を持つか確かめ、持たないなら番号を外して並列のまま書いてください")
                .with_related(spans.clone())
                .with_metric("count", spans.len()),
        );
    }
}

// ---------------------------------------------------------------------------
// S05 EMOJI_DENSITY
// ---------------------------------------------------------------------------

static EMOJI_META: RuleMeta = RuleMeta {
    id: "S05",
    name: "EMOJI_DENSITY",
    title: "絵文字・装飾記号の多用",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "絵文字や装飾記号が原文 1000 字あたり 2 個以上あり、チャットの体裁が持ち込まれている",
    explanation: "\
### 何を見るか

絵文字や装飾記号 (✅ ⭐ 📌 🚀 など) が 3 個以上あり、原文 1000 字あたり 2 個以上の密度で使われて\
いる文書を指します。コードブロックの中は数えません。

### なぜ問題か

感情の演出や区切りの飾りとして絵文字を並べるのは、チャットの出力の体裁が文書に持ち込まれた形に\
なりがちです。

### 直し方

絵文字は、社内で意味が決まっている区分のラベルのように、読み手の道案内として働く場合だけ残して\
ください。

### 例

- 直す前: 「✅ 申請はオンラインで完結します 🚀」
- 直した後: 「申請はオンラインで完結します。」

### 根拠

実験的です。定量的な校正はしていません。
",
};

/// 絵文字・装飾記号か (代表的な範囲に限る。厳密な絵文字の判定はしない)。
fn is_emoji_like(c: char) -> bool {
    matches!(
        c,
        '\u{1F300}'..='\u{1FAFF}' | '\u{2600}'..='\u{27BF}' | '\u{2B50}' | '\u{2B55}'
    )
}

/// ブロック内の絵文字・装飾記号の原文上の範囲。
fn block_emoji(block: &Block) -> impl Iterator<Item = Span> + '_ {
    block
        .text
        .char_indices()
        .filter(|&(_, c)| is_emoji_like(c))
        .map(move |(i, c)| block.to_source(i..i + c.len_utf8()))
}

/// 文書中の絵文字・装飾記号の原文上の範囲 (コードブロックは解析用テキストに含まれない)。
fn emoji_spans(ctx: &RuleContext<'_>) -> Vec<Span> {
    ctx.doc.blocks.iter().flat_map(block_emoji).collect()
}

/// S05 EMOJI_DENSITY。
pub struct EmojiDensity {
    min_count: usize,
    per_1000: f64,
}

impl EmojiDensity {
    pub fn new() -> Self {
        Self {
            min_count: 3,
            per_1000: 2.0,
        }
    }
}

impl Default for EmojiDensity {
    fn default() -> Self {
        Self::new()
    }
}

impl Rule for EmojiDensity {
    fn meta(&self) -> &'static RuleMeta {
        &EMOJI_META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_count" => self.min_count = option_count(key, value)?,
            "per_1000" => self.per_1000 = option_positive(key, value)?,
            _ => return Err(unknown_option(&EMOJI_META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_count", self.min_count.to_string()),
            ("per_1000", self.per_1000.to_string()),
        ]
    }

    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let n = emoji_spans(ctx).len();
        if n < self.min_count {
            return Vec::new();
        }
        vec![Measure::new(
            "emoji_per_1000",
            per_thousand(n, ctx.doc.char_count()),
            "per_1000",
            Fires::AtOrAbove,
        )]
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let spans = emoji_spans(ctx);
        let density = per_thousand(spans.len(), ctx.doc.char_count());
        if spans.len() < self.min_count || density < self.per_1000 {
            return;
        }
        out.push(
            EMOJI_META
                .diagnostic(
                    spans[0],
                    format!(
                        "絵文字・装飾記号が {} 個あります (原文 1000 字あたり {density:.1} 個)",
                        spans.len()
                    ),
                )
                .with_hint("絵文字は、読み手の道案内として意味が決まっている場合だけ残してください")
                .with_related(spans.clone())
                .with_metric("count", spans.len())
                .with_metric("per_1000", density),
        );
    }
}

// ---------------------------------------------------------------------------
// S06 BOLD_LABEL_LIST
// ---------------------------------------------------------------------------

static LABEL_META: RuleMeta = RuleMeta {
    id: "S06",
    name: "BOLD_LABEL_LIST",
    title: "「**項目**: 説明」の定型",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "箇条書きが「**ラベル**: 説明」の型で 3 項目以上並んでいる",
    explanation: "\
### 何を見るか

箇条書きの項目が「**ラベル**: 説明」の形 (項目の先頭が太字で、直後にコロンやダッシュが続く) で \
3 つ以上並んでいる箇所を指します。

### なぜ問題か

項目ごとに太字の見出し語と説明を並べる型は、生成された文章に特有の体裁になりがちです。太字が\
項目の数だけ並ぶので、強調としても効きません。

### 直し方

説明が短ければラベルを外して一文にまとめてください。項目が多いなら表にするか、見出しに格上げ\
します。

### 例

- 直す前: 「- **速度**: 起動が速い / - **安全性**: 権限を絞れる / - **拡張性**: プラグインを追加できる」
- 直した後: 「起動が速く、権限を細かく絞れ、プラグインで機能を足せる。」

### 根拠

実験的です。定量的な校正はしていません。
",
};

/// ラベルの直後に続く区切り。
fn starts_with_label_separator(rest: &str) -> bool {
    let rest = rest.trim_start();
    rest.starts_with([':', '：', '-', '—', '―', '－', '|', '｜'])
}

/// 「**ラベル**: 説明」の型の箇条書きの項目 (太字の原文上の範囲と、ラベル)。
fn bold_labels(ctx: &RuleContext<'_>) -> Vec<(Span, String)> {
    ctx.doc
        .blocks
        .iter()
        .filter(|b| b.kind == BlockKind::ListItem)
        .filter_map(|b| {
            let lead = b.text.len() - b.text.trim_start().len();
            let mark = b
                .marks
                .iter()
                .find(|m| m.kind == MarkKind::Strong && m.range.start == lead)?;
            let label = &b.text[mark.range.clone()];
            let colon_inside = label.trim_end().ends_with([':', '：']);
            if !colon_inside && !starts_with_label_separator(&b.text[mark.range.end..]) {
                return None;
            }
            let label = label.trim_end_matches([':', '：']).trim().to_string();
            Some((mark.span, label))
        })
        .collect()
}

/// S06 BOLD_LABEL_LIST。
pub struct BoldLabelList {
    min_count: usize,
}

impl BoldLabelList {
    pub fn new() -> Self {
        Self { min_count: 3 }
    }
}

impl Default for BoldLabelList {
    fn default() -> Self {
        Self::new()
    }
}

impl Rule for BoldLabelList {
    fn meta(&self) -> &'static RuleMeta {
        &LABEL_META
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_count" => self.min_count = option_count(key, value)?,
            _ => return Err(unknown_option(&LABEL_META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![("min_count", self.min_count.to_string())]
    }

    /// 「**ラベル**: 説明」の項目の数を `min_count` と比べる値として返す。1 つもない文書は
    /// 値を返さない。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        count_measure("bold_label_items", bold_labels(ctx).len())
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let labels = bold_labels(ctx);
        if labels.len() < self.min_count {
            return;
        }
        let related: Vec<Span> = labels.iter().map(|(s, _)| *s).collect();
        let n = labels.len();
        for (span, label) in &labels {
            out.push(
                LABEL_META
                    .diagnostic(
                        *span,
                        format!("箇条書きが「**{label}**: 説明」の型で {n} 項目並んでいます"),
                    )
                    .with_hint("説明が短ければラベルを外して一文にまとめ、項目が多ければ表や見出しにしてください")
                    .with_related(related.clone())
                    .with_metric("count", n),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// S07 HEADING_TEMPLATE
// ---------------------------------------------------------------------------

static TEMPLATE_META: RuleMeta = RuleMeta {
    id: "S07",
    name: "HEADING_TEMPLATE",
    title: "見出しの型の反復",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "見出しの 8 割以上が「X: Y」・問い・番号のどれか同じ型で、型に流し込んだ構成になっている",
    explanation: "\
### 何を見るか

レベル 2 以下の見出し (`##` とそれより深いもの。文書の題名の `#` は除く) が 4 つ以上ある文書で、見出しを\
形で分けます。

- コロン型: 「X: Y」「X：Y」
- 問い型: 「？」「?」「とは」「のか」「何か」「でしょうか」で終わる
- 番号型: 「1.」「1.2 」「第1」「第三章」「ステップ1」「①」「(1)」で始まる

どれにも当てはまらない見出しは型に入れずに数だけ数え、同じ型の見出しが全体の 80% 以上を占めるときに \
1 件出します。

### なぜ問題か

どの見出しも同じ型に流し込むと、節ごとの中身の違いが見出しから消えます。「X とは？」や「X: Y」を\
並べた構成は生成された文章によく見られ、見出しを拾い読みした読み手には型の繰り返しだけが残ります。

### 直し方

見出しにはその節で言いたいことを書き、型をそろえること自体を目的にしないでください。手順のように\
本当に順序を持つ節だけ番号を残します。

### 例

- 直す前: 「## 背景: なぜ今なのか / ## 課題: 何が起きているか / ## 対策: どう変えるか / ## 効果: 何が得られるか」
- 直した後: 「## 夜間の問い合わせが 3 倍に増えた / ## 一次回答を自動化する / ## 残業が月 20 時間減る見込み」

### 根拠

実験的です。見出しの数が人の文章と生成された文章で差の出やすい指標だという公開の調査はありますが、\
見出しの型の比率と閾値 0.8 は noslop のコーパスで校正していません。ビジネス文書では見出しの型を\
そろえるのが正当な慣習なので、ジャンル business では既定で動かしません。
",
};

/// S07 が数える見出し (レベル 2 以下、引用の外) の原文上の範囲と本文。
fn section_headings<'a>(ctx: &RuleContext<'a>) -> Vec<(Span, &'a str)> {
    ctx.doc
        .blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Heading(level) if level >= 2) && !b.in_quote)
        .map(trimmed_text)
        .filter(|(_, text)| !text.is_empty())
        .collect()
}

/// いちばん多い型 (「それ以外」を除く) とその数。同数なら先の型。
fn dominant_shape(shapes: &[HeadingShape]) -> (HeadingShape, usize) {
    let mut best = (HeadingShape::Colon, 0);
    for shape in HeadingShape::TEMPLATES {
        let n = shapes.iter().filter(|&&s| s == shape).count();
        if n > best.1 {
            best = (shape, n);
        }
    }
    best
}

/// S07 HEADING_TEMPLATE。
pub struct HeadingTemplate {
    min_headings: usize,
    ratio_threshold: f64,
}

impl HeadingTemplate {
    pub fn new() -> Self {
        Self {
            min_headings: 4,
            ratio_threshold: 0.8,
        }
    }
}

impl Default for HeadingTemplate {
    fn default() -> Self {
        Self::new()
    }
}

impl Rule for HeadingTemplate {
    fn meta(&self) -> &'static RuleMeta {
        &TEMPLATE_META
    }

    fn allowed_in(&self, genre: Genre) -> bool {
        genre != Genre::Business
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_headings" => self.min_headings = option_count(key, value)?,
            "ratio_threshold" => self.ratio_threshold = option_ratio(key, value)?,
            _ => return Err(unknown_option(&TEMPLATE_META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_headings", self.min_headings.to_string()),
            ("ratio_threshold", self.ratio_threshold.to_string()),
        ]
    }

    /// 見出しが `min_headings` 以上ある文書で、いちばん多い型の割合を返す。どの見出しも型に
    /// 当てはまらない文書は、閾値をどう変えても指摘しないので値を返さない。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let shapes: Vec<HeadingShape> = section_headings(ctx)
            .iter()
            .map(|(_, t)| HeadingShape::of(t))
            .collect();
        if shapes.len() < self.min_headings {
            return Vec::new();
        }
        let (_, n) = dominant_shape(&shapes);
        if n == 0 {
            return Vec::new();
        }
        vec![Measure::new(
            "template_ratio",
            n as f64 / shapes.len() as f64,
            "ratio_threshold",
            Fires::AtOrAbove,
        )]
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let headings = section_headings(ctx);
        if headings.len() < self.min_headings {
            return;
        }
        let shapes: Vec<HeadingShape> = headings.iter().map(|(_, t)| HeadingShape::of(t)).collect();
        let (shape, n) = dominant_shape(&shapes);
        let ratio = n as f64 / headings.len() as f64;
        if n == 0 || ratio < self.ratio_threshold {
            return;
        }
        let Some(first) = shapes.iter().position(|&s| s == shape) else {
            return;
        };
        out.push(
            TEMPLATE_META
                .diagnostic(
                    headings[first].0,
                    format!(
                        "見出し {} 個のうち {n} 個 ({:.0}%) が{}です",
                        headings.len(),
                        ratio * 100.0,
                        shape.label()
                    ),
                )
                .with_hint("見出しには節で言いたいことを書き、型をそろえること自体を目的にしないでください")
                .with_related(headings.iter().map(|(s, _)| *s).collect())
                .with_metric("headings", headings.len())
                .with_metric("count", n)
                .with_metric("ratio", ratio)
                .with_metric("shape", shape.as_str()),
        );
    }
}

// ---------------------------------------------------------------------------
// S08 HEADING_EMPHASIS
// ---------------------------------------------------------------------------

static HEADING_EMPHASIS_META: RuleMeta = RuleMeta {
    id: "S08",
    name: "HEADING_EMPHASIS",
    title: "見出し内の太字・絵文字",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "見出しの中に太字や絵文字があり、もともと目立つ見出しをさらに飾っている",
    explanation: "\
### 何を見るか

見出しの中の太字 (`**…**`) と、絵文字・装飾記号 (✅ 🚀 など) を指します。見出し 1 つにつき 1 件\
出します。

### なぜ問題か

見出しは文字の大きさと位置で、すでに本文から浮いています。そこに太字や絵文字を重ねても目立ち方は\
ほとんど変わらず、飾りだけが増えます。チャットの画面で見出しまで飾る体裁を、文書に持ち込んだ形でも\
あります。

### 直し方

太字と絵文字を外してください。目立たせたい理由があるなら、見出しの言葉そのものを具体的にします。

### 例

- 直す前: 「## 🚀 **導入の手順**」
- 直した後: 「## 導入の手順 (15 分で終わります)」

### 根拠

実験的です。既存の文章校正ツールにある生成文章向けのルール集でも、見出しの中の太字は生成された\
文章の特徴として扱われていますが、noslop のコーパスでは校正していません。
",
};

/// S08 HEADING_EMPHASIS。
pub struct HeadingEmphasis;

impl Rule for HeadingEmphasis {
    fn meta(&self) -> &'static RuleMeta {
        &HEADING_EMPHASIS_META
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for block in ctx.doc.blocks.iter().filter(|b| b.is_heading()) {
            let strong: Vec<Span> = block
                .marks
                .iter()
                .filter(|m| m.kind == MarkKind::Strong)
                .map(|m| m.span)
                .collect();
            let emoji: Vec<Span> = block_emoji(block).collect();
            let what = match (strong.is_empty(), emoji.is_empty()) {
                (true, true) => continue,
                (false, false) => "太字と絵文字",
                (false, true) => "太字",
                (true, false) => "絵文字",
            };
            let mut spans: Vec<Span> = strong.iter().chain(&emoji).copied().collect();
            spans.sort_by_key(|s| s.start);
            let heading = quote(trimmed_text(block).1);
            out.push(
                HEADING_EMPHASIS_META
                    .diagnostic(
                        spans[0],
                        format!("見出し「{heading}」の中に{what}があります"),
                    )
                    .with_hint("見出しはそれだけで目立つので、太字や絵文字を外してください")
                    .with_related(spans.clone())
                    .with_metric("strong", strong.len())
                    .with_metric("emoji", emoji.len()),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// S09 STRUCTURE_DENSITY
// ---------------------------------------------------------------------------

static DENSITY_META: RuleMeta = RuleMeta {
    id: "S09",
    name: "STRUCTURE_DENSITY",
    title: "見出しと箇条書きの密度",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "見出しか箇条書きの項目が原文 1000 字あたりの暫定の目安より多く、文章が細切れになっている",
    explanation: "\
### 何を見るか

原文が 1000 字以上ある文書で、原文 1000 字あたりの見出しの数と箇条書きの項目の数を数えます。見出しが \
4 個以上か、箇条書きの項目が 10 個以上なら 1 件出します。1 つの項目の中に段落が複数あれば、段落ごとに \
1 項目と数えます。

### なぜ問題か

見出しと箇条書きで細かく区切った文章は、節と節、項目と項目のあいだの論理を読み手に預けます。短い\
文書に見出しが何本も立ち、どの節も数行の箇条書きで終わる構成は、生成された文章に目立つ体裁です。

### 直し方

中身が数行しかない節は前後の節にまとめ、見出しを減らしてください。つながりのある項目は地の文に戻し、\
「だから」「ところが」で因果を書きます。

### 例

- 直す前: 「## 背景 / - 問い合わせが増えた / ## 課題 / - 回答が遅れる / ## 対策 / - 一次回答を自動化する」
- 直した後: 「問い合わせが増えて回答が遅れているので、一次回答を自動化する。」

### 根拠

実験的です。公開の調査で、見出しの数と箇条書きの記号の数は、個々の語よりも人の文章と生成された\
文章の差が大きい指標だと報告されています。ただし既定の閾値 (見出し 4 個、項目 10 個) は校正前の\
暫定値で、noslop のコーパスでは校正していません。ビジネス文書では見出しと箇条書きで区切るのが正当な\
慣習なので、ジャンル business では既定で動かしません。
",
};

/// S09 が数えた見出しと箇条書きの項目。
struct Density<'a> {
    chars: usize,
    headings: Vec<&'a Block>,
    items: Vec<&'a Block>,
}

impl Density<'_> {
    fn headings_per_1000(&self) -> f64 {
        per_thousand(self.headings.len(), self.chars)
    }

    fn items_per_1000(&self) -> f64 {
        per_thousand(self.items.len(), self.chars)
    }
}

/// S09 STRUCTURE_DENSITY。
pub struct StructureDensity {
    min_chars: usize,
    headings_per_1000: f64,
    list_items_per_1000: f64,
}

impl StructureDensity {
    pub fn new() -> Self {
        Self {
            min_chars: 1000,
            // どちらも校正前の暫定値
            headings_per_1000: 4.0,
            list_items_per_1000: 10.0,
        }
    }

    /// 判定の前提 (原文が `min_chars` 字以上) を満たす文書で数える。
    fn density<'a>(&self, ctx: &RuleContext<'a>) -> Option<Density<'a>> {
        let chars = ctx.doc.char_count();
        if chars < self.min_chars {
            return None;
        }
        let blocks = &ctx.doc.blocks;
        Some(Density {
            chars,
            headings: blocks.iter().filter(|b| b.is_heading()).collect(),
            items: blocks
                .iter()
                .filter(|b| b.kind == BlockKind::ListItem)
                .collect(),
        })
    }
}

impl Default for StructureDensity {
    fn default() -> Self {
        Self::new()
    }
}

impl Rule for StructureDensity {
    fn meta(&self) -> &'static RuleMeta {
        &DENSITY_META
    }

    fn allowed_in(&self, genre: Genre) -> bool {
        genre != Genre::Business
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_chars" => self.min_chars = option_count(key, value)?,
            "headings_per_1000" => self.headings_per_1000 = option_positive(key, value)?,
            "list_items_per_1000" => self.list_items_per_1000 = option_positive(key, value)?,
            _ => return Err(unknown_option(&DENSITY_META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![
            ("min_chars", self.min_chars.to_string()),
            ("headings_per_1000", self.headings_per_1000.to_string()),
            ("list_items_per_1000", self.list_items_per_1000.to_string()),
        ]
    }

    /// 原文が `min_chars` 字以上ある文書で、見出しと箇条書きの項目の密度をそれぞれの閾値と
    /// 比べる値として返す。見出し (項目) が 1 つもなければ、その密度は閾値をどう変えても
    /// 指摘しないので返さない。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let Some(d) = self.density(ctx) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if !d.headings.is_empty() {
            out.push(Measure::new(
                "headings_per_1000",
                d.headings_per_1000(),
                "headings_per_1000",
                Fires::AtOrAbove,
            ));
        }
        if !d.items.is_empty() {
            out.push(Measure::new(
                "list_items_per_1000",
                d.items_per_1000(),
                "list_items_per_1000",
                Fires::AtOrAbove,
            ));
        }
        out
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let Some(d) = self.density(ctx) else {
            return;
        };
        let (headings, items) = (d.headings_per_1000(), d.items_per_1000());
        let many_headings = !d.headings.is_empty() && headings >= self.headings_per_1000;
        let many_items = !d.items.is_empty() && items >= self.list_items_per_1000;
        let first = match (many_headings, many_items) {
            (true, _) => d.headings[0],
            (false, true) => d.items[0],
            (false, false) => return,
        };
        let mut parts = Vec::new();
        if many_headings {
            parts.push(format!("見出しが {headings:.1} 個"));
        }
        if many_items {
            parts.push(format!("箇条書きの項目が {items:.1} 個"));
        }
        out.push(
            DENSITY_META
                .diagnostic(
                    first.to_source(0..first.text.len()),
                    format!("原文 1000 字あたり{}あります", parts.join("、")),
                )
                .with_hint("中身の短い節は前後にまとめて見出しを減らし、つながりのある項目は地の文に戻してください")
                .with_metric("chars", d.chars)
                .with_metric("headings", d.headings.len())
                .with_metric("list_items", d.items.len())
                .with_metric("headings_per_1000", headings)
                .with_metric("list_items_per_1000", items),
        );
    }
}

// ---------------------------------------------------------------------------
// S10 LABEL_STYLE
// ---------------------------------------------------------------------------

static LABEL_STYLE_META: RuleMeta = RuleMeta {
    id: "S10",
    name: "LABEL_STYLE",
    title: "絵文字やラベルで始める書き方",
    lane: Lane::Slop,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "段落や項目を絵文字か「ラベル：」で書き出す箇所が 3 か所以上あり、文ではなく札を並べた体裁になっている",
    explanation: "\
### 何を見るか

次の 2 つが文書の中で合わせて 3 か所以上あるとき、1 か所ずつ指します。

- 絵文字・装飾記号で始まる段落や箇条書きの項目 (「✅ 完了」「💡 ポイント: …」)
- 本文の段落の冒頭に置いた、2〜12 字の短いラベルとコロン (「重要な要素： …」)

数字だけのラベル (「10:30」など)、時刻や比 (「午前10:30」「1:2」)、URL、日本語を含まない段落は\
拾いません。箇条書きの項目の「**ラベル**: 説明」は S06 が見ます。「注意：」を 1 か所だけ使うような\
書き方は人の文書にもよくあるので、数が少ない文書では指摘しません。

### なぜ問題か

段落の頭に札を立てると、文がどこから始まるのかが見えにくくなります。絵文字や「ポイント：」で\
区切る書き方はチャットの画面を読みやすくするための体裁で、文書に持ち込むと、文でつなぐべき論理が\
札の並びに置き換わります。

### 直し方

ラベルを外し、その中身を文の主語や前置きとして書き込んでください。絵文字は、社内で意味が決まって\
いる区分の目印のように、読み手の道案内として働く場合だけ残します。

### 例

- 直す前: 「注意点：申請の前に上長の承認を取ってください。」
- 直した後: 「申請の前に、上長の承認を取っておいてください。」

### 根拠

実験的です。既存の文章校正ツールにある生成文章向けのルール集でも、絵文字やラベルで始める体裁は\
扱われていますが、noslop のコーパスでは校正していません。対談の書き起こしや Q&A の「話者名：発言」は\
正当な書き方なので、抑制コメントで理由を添えて外してください。ビジネス文書ではラベルで項目を立てる\
のが正当な慣習なので、ジャンル business では既定で動かしません。
",
};

/// 段落の冒頭のラベル: 2〜12 字 + コロン + 本文。
static LEADING_LABEL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([^\s:：、。，．「」『』（）()\[\]]{2,12})[:：]\s*(\S)")
        .expect("leading label regex")
});

fn is_digit(c: char) -> bool {
    c.is_ascii_digit() || ('０'..='９').contains(&c)
}

/// 段落の冒頭のラベル (「注意点：」など) の本体と、コロンまでの長さ (解析用テキスト上のバイト数)。
fn leading_label(text: &str) -> Option<(&str, usize)> {
    let caps = LEADING_LABEL_RE.captures(text)?;
    let label = caps.get(1)?;
    let next = caps.get(2)?.as_str().chars().next()?;
    let body = label.as_str();
    // 「10:30」「午前10:30」「1:2」のような時刻や比、URL の `://`
    let numeric = body.chars().all(is_digit) || (body.ends_with(is_digit) && is_digit(next));
    if numeric || next == '/' {
        return None;
    }
    let colon = text[label.end()..].chars().next()?;
    Some((body, label.end() + colon.len_utf8()))
}

/// 絵文字かラベルで始まるブロック 1 つ分の指摘 (日本語を含む段落・項目だけを見る)。
fn label_lead(block: &Block) -> Option<Diagnostic> {
    let place = match block.kind {
        BlockKind::Paragraph => "段落",
        BlockKind::ListItem => "箇条書きの項目",
        _ => return None,
    };
    if block.in_quote || block.in_footnote || !text::contains_japanese(&block.text) {
        return None;
    }
    let lead = block.text.len() - block.text.trim_start().len();
    let rest = &block.text[lead..];
    if let Some(c) = rest.chars().next().filter(|&c| is_emoji_like(c)) {
        return Some(
            LABEL_STYLE_META
                .diagnostic(
                    block.to_source(lead..lead + c.len_utf8()),
                    format!("{place}を絵文字「{c}」で書き出しています"),
                )
                .with_hint(
                    "絵文字を外し、読み手の道案内として意味が決まっている場合だけ残してください",
                ),
        );
    }
    if block.kind != BlockKind::Paragraph {
        return None;
    }
    let (label, end) = leading_label(rest)?;
    let mut span = block.to_source(lead..lead + end);
    // 「**重要**:」のようにラベルが太字なら、太字の記号ごと指す
    if let Some(m) = block
        .marks
        .iter()
        .find(|m| m.kind == MarkKind::Strong && m.range.start == lead)
    {
        span = Span::new(m.span.start.min(span.start), m.span.end.max(span.end));
    }
    Some(
        LABEL_STYLE_META
            .diagnostic(
                span,
                format!(
                    "段落を「{}」というラベルで書き出しています",
                    quote(&rest[..end])
                ),
            )
            .with_hint("ラベルを外し、その中身を文の主語や前置きとして書き込んでください")
            .with_metric("label", quote(label)),
    )
}

/// S10 LABEL_STYLE。
pub struct LabelStyle {
    min_count: usize,
}

impl LabelStyle {
    pub fn new() -> Self {
        Self { min_count: 3 }
    }
}

impl Default for LabelStyle {
    fn default() -> Self {
        Self::new()
    }
}

impl Rule for LabelStyle {
    fn meta(&self) -> &'static RuleMeta {
        &LABEL_STYLE_META
    }

    fn allowed_in(&self, genre: Genre) -> bool {
        genre != Genre::Business
    }

    fn configure(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "min_count" => self.min_count = option_count(key, value)?,
            _ => return Err(unknown_option(&LABEL_STYLE_META, key)),
        }
        Ok(())
    }

    fn options(&self) -> Vec<(&'static str, String)> {
        vec![("min_count", self.min_count.to_string())]
    }

    /// 絵文字かラベルで書き出す箇所の数を `min_count` と比べる値として返す。1 つもない文書は
    /// 値を返さない。
    fn measure(&self, ctx: &RuleContext<'_>) -> Vec<Measure> {
        let n = ctx.doc.blocks.iter().filter_map(label_lead).count();
        count_measure("label_leads", n)
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        let found: Vec<Diagnostic> = ctx.doc.blocks.iter().filter_map(label_lead).collect();
        let n = found.len();
        if n < self.min_count {
            return;
        }
        out.extend(found.into_iter().map(|mut d| {
            d.message.push_str(&format!(" (文書全体で {n} か所)"));
            d.with_metric("count", n)
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::rules::testing::{assert_measures_agree, matched, measure_md, run};

    /// 既定の条件でルールの測定値を取る。
    /// measure を実装しているルール。S03・S08 は閾値を持たない (該当する見出しごとに指摘する)。
    const MEASURED: [&str; 8] = ["S01", "S02", "S04", "S05", "S06", "S07", "S09", "S10"];

    fn rule_by_id(id: &str) -> Box<dyn Rule> {
        rules(Genre::General)
            .into_iter()
            .find(|r| r.meta().id == id)
            .unwrap_or_else(|| panic!("{id} がありません"))
    }

    /// 指摘する例・しない例をひととおり含む文書。
    fn samples() -> Vec<String> {
        let filler = "本文の説明を続ける。".repeat(150);
        let paragraphs = |n: usize| {
            (0..n)
                .map(|i| format!("段落{i}の本文。\n\n"))
                .collect::<String>()
        };
        let items = |n: usize| (0..n).map(|i| format!("- 項目{i}\n")).collect::<String>();
        let sections = |n: usize, body: usize| {
            (0..n)
                .map(|i| format!("## 節{i}\n\n{}。\n\n", "本文".repeat(body)))
                .collect::<String>()
        };
        vec![
            String::new(),
            "本文だけの短い段落。\n".to_string(),
            // S01・S05
            "**大切なのは**、**早めの申請**と**正確な記入**です。\n".to_string(),
            format!("**要点**だけを太字にした。\n\n{filler}\n\n**二つ目**と**三つ目**。\n"),
            "**一つ**だけ。\n".to_string(),
            "✅ 申請はオンラインで完結します 🚀 📌\n".to_string(),
            format!("✅ 最初の段落。\n\n{filler}\n\n🚀 と 📌 を最後に置いた。\n"),
            // S02
            paragraphs(6) + &items(4),
            paragraphs(8) + &items(2),
            paragraphs(10),
            // S04
            "## フェーズ1: 設計\n\n- ステップ2で試す\n- ステージ 3 で広げる\n".to_string(),
            "フェーズ1とフェーズ2。\n".to_string(),
            // S06
            "- **速度**: 起動が速い\n- **安全性**: 権限を絞れる\n- **拡張性:** プラグインを足せる\n"
                .to_string(),
            "- **速度**: 起動が速い\n- **安全性**は高い\n- 拡張性\n".to_string(),
            // S07
            "# 手引き\n\n## 背景: なぜ今なのか\n\n本文。\n\n## 課題: 何が起きているか\n\n本文。\n\n## 対策: どう変えるか\n\n本文。\n\n## 効果: 何が得られるか\n\n本文。\n"
                .to_string(),
            "## 導入\n\n## 設定: 環境変数\n\n## 使い方\n\n## よくある質問\n".to_string(),
            "## 導入\n\n## 使い方\n\n## 設定\n\n## 参考資料\n".to_string(),
            // S09
            sections(6, 95),
            sections(2, 295),
            "手順を並べる。\n\n".to_string()
                + &(0..15)
                    .map(|i| format!("- 項目{i:02}の説明{}\n", "文".repeat(60)))
                    .collect::<String>(),
            // S10
            "✅ 申請が完了しました。\n\n- 💡 ポイント: 早めに出す\n- 通常の項目\n\n注意点：申請の前に上長の承認を取ってください。\n\n**重要**: 締め日は月末です。\n\n普通の段落です。\n"
                .to_string(),
            "注意点：申請の前に上長の承認を取ってください。\n\n✅ 申請が完了しました。\n".to_string(),
        ]
    }

    #[test]
    fn measures_agree_with_check() {
        let docs: Vec<Document> = samples().into_iter().map(Document::markdown).collect();
        for rule in rules(Genre::General) {
            let id = rule.meta().id;
            if !MEASURED.contains(&id) {
                assert!(
                    docs.iter()
                        .all(|d| rule.measure(&RuleContext::new(d)).is_empty()),
                    "{id}: measure を実装したら MEASURED に足してください"
                );
                continue;
            }
            let agreement = assert_measures_agree(|| rule_by_id(id), &docs);
            assert!(agreement.fired > 0, "{id}: 指摘する例がありません");
            assert!(
                agreement.quiet > 0,
                "{id}: 値を測れて指摘しない例がありません"
            );
            assert!(agreement.switches.is_empty(), "{id}");
        }
    }

    #[test]
    fn documents_without_the_counted_marks_are_not_measured() {
        // 閾値を 0 にしても指摘しない文書は、値を測れない文書として扱う
        let plain = "段落0の本文。\n\n".repeat(10);
        assert!(measure_md(&BulletRatio::new(), &plain).is_empty());
        let mut zero = BulletRatio::new();
        zero.configure("ratio_threshold", &toml::Value::Float(0.0))
            .unwrap();
        assert!(run(&zero, &plain).is_empty());

        let others = "## 導入\n\n## 使い方\n\n## 設定\n\n## 参考資料\n";
        assert!(measure_md(&HeadingTemplate::new(), others).is_empty());

        let no_items = format!("## 節\n\n{}。\n", "本文".repeat(600));
        let m = measure_md(&StructureDensity::new(), &no_items);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].threshold_key, "headings_per_1000");

        for rule in [
            Box::new(NumberedPhases::new()) as Box<dyn Rule>,
            Box::new(BoldLabelList::new()),
            Box::new(LabelStyle::new()),
        ] {
            assert!(
                measure_md(rule.as_ref(), &plain).is_empty(),
                "{}",
                rule.meta().id
            );
        }
        let m = measure_md(&NumberedPhases::new(), "フェーズ1とフェーズ2。\n");
        assert_eq!((m[0].threshold_key, m[0].value), ("min_count", 2.0));
    }

    #[test]
    fn ids_and_business_genre() {
        let rules = rules(Genre::General);
        let ids: Vec<_> = rules.iter().map(|r| r.meta().id).collect();
        let expected: Vec<String> = (1..=10).map(|n| format!("S{n:02}")).collect();
        assert_eq!(ids, expected);
        for r in &rules {
            assert_eq!(r.meta().status, RuleStatus::Experimental);
            assert_eq!(r.meta().lane, Lane::Slop);
            let blocked = matches!(
                r.meta().id,
                "S01" | "S02" | "S03" | "S04" | "S07" | "S09" | "S10"
            );
            assert_eq!(r.allowed_in(Genre::Business), !blocked, "{}", r.meta().id);
            assert!(r.allowed_in(Genre::Tech));
        }
    }

    #[test]
    fn metas_follow_the_explanation_template() {
        for rule in rules(Genre::General) {
            let m = rule.meta();
            for section in [
                "### 何を見るか",
                "### なぜ問題か",
                "### 直し方",
                "### 例",
                "### 根拠",
            ] {
                assert!(
                    m.explanation.contains(section),
                    "{} の explanation に {section} がない",
                    m.id
                );
            }
            assert!(m.explanation.contains("- 直す前: "), "{}", m.id);
            assert!(m.explanation.contains("- 直した後: "), "{}", m.id);
            assert!(!m.summary.contains('\n'), "{}", m.id);
        }
    }

    #[test]
    fn s01_s02_s05_measure_the_compared_values() {
        let bold = "**大切なのは**、**早めの申請**と**正確な記入**です。\n";
        let m = measure_md(&BoldDensity::new(), bold);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].threshold_key, "per_1000");
        assert!((m[0].value - 3000.0 / bold.chars().count() as f64).abs() < 1e-9);
        assert!(m[0].fires_at(3.0));
        assert!(measure_md(&BoldDensity::new(), "**一つ**だけ。\n").is_empty());

        let mut list = String::new();
        for i in 0..6 {
            list.push_str(&format!("段落{i}の本文。\n\n"));
        }
        for i in 0..4 {
            list.push_str(&format!("- 項目{i}\n"));
        }
        let m = measure_md(&BulletRatio::new(), &list);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].threshold_key, "ratio_threshold");
        assert!((m[0].value - 0.4).abs() < 1e-9);
        assert!(measure_md(&BulletRatio::new(), "段落。\n\n- 項目\n").is_empty());

        let emoji = "✅ 申請はオンラインで完結します 🚀 📌\n";
        let m = measure_md(&EmojiDensity::new(), emoji);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].threshold_key, "per_1000");
        assert!((m[0].value - 3000.0 / emoji.chars().count() as f64).abs() < 1e-9);
        assert!(measure_md(&EmojiDensity::new(), "✅ だけ\n").is_empty());
    }

    #[test]
    fn s07_flags_headings_poured_into_one_template() {
        let md = "# 手引き\n\n## 背景: なぜ今なのか\n\n本文。\n\n## 課題: 何が起きているか\n\n本文。\n\n### 対策: どう変えるか\n\n本文。\n\n## 効果: 何が得られるか\n\n本文。\n";
        let d = run(&HeadingTemplate::new(), md);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, "S07");
        assert_eq!(matched(md, &d), vec!["背景: なぜ今なのか"]);
        assert_eq!(d[0].related.len(), 4);
        assert!(d[0].message.contains("4 個のうち 4 個 (100%)"));
        assert!(d[0].message.contains("コロン型"));
        let numbered = "## 1. 概要\n\n## 2. 手順\n\n## 3. 注意\n\n## 4. 参考\n";
        assert!(
            run(&HeadingTemplate::new(), numbered)[0]
                .message
                .contains("番号型")
        );
    }

    #[test]
    fn s07_ignores_mixed_few_or_top_level_headings() {
        let mixed = "## 導入\n\n## 設定: 環境変数\n\n## 使い方\n\n## よくある質問\n";
        assert!(run(&HeadingTemplate::new(), mixed).is_empty());
        let few = "## 1. 概要\n\n## 2. 手順\n\n## 3. 注意\n";
        assert!(run(&HeadingTemplate::new(), few).is_empty());
        let titles = "# A: 一\n\n# B: 二\n\n# C: 三\n\n# D: 四\n";
        assert!(run(&HeadingTemplate::new(), titles).is_empty());
    }

    #[test]
    fn s07_measures_the_largest_template_ratio() {
        let md = "## 1. 概要\n\n## 2. 手順\n\n## 3. 注意\n\n## 参考資料\n";
        let m = measure_md(&HeadingTemplate::new(), md);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].threshold_key, "ratio_threshold");
        assert!((m[0].value - 0.75).abs() < 1e-9);
        assert!(!m[0].fires_at(0.8));
        assert!(run(&HeadingTemplate::new(), md).is_empty());
        let mut r = HeadingTemplate::new();
        r.configure("ratio_threshold", &toml::Value::Float(0.7))
            .unwrap();
        assert_eq!(run(&r, md).len(), 1);
        assert!(measure_md(&HeadingTemplate::new(), "## 1. 概要\n").is_empty());
    }

    #[test]
    fn s08_flags_bold_and_emoji_in_headings() {
        let md = "# 🚀 **導入の手順**\n\n本文の**太字**は見ない。\n\n## 設定\n\n## ✅ 確認\n\n## **注意**点\n";
        let d = run(&HeadingEmphasis, md);
        assert_eq!(matched(md, &d), vec!["🚀", "✅", "**注意**"]);
        assert!(d[0].message.contains("太字と絵文字"));
        assert_eq!(d[0].related.len(), 2);
        assert!(d[1].message.ends_with("の中に絵文字があります"));
        assert!(d[2].message.ends_with("の中に太字があります"));
        assert!(run(&HeadingEmphasis, "## 設定\n\n**太字**の本文 ✅\n").is_empty());
        let code = run(&HeadingEmphasis, "## ✅ `config` を確認\n");
        assert_eq!(
            code[0].message,
            "見出し「✅ … を確認」の中に絵文字があります"
        );
    }

    #[test]
    fn s09_flags_dense_headings_and_lists() {
        // 1 節 200 字 (見出し 7 字 + 本文 193 字) を 6 節並べた 1200 字
        let mut dense = String::new();
        for i in 0..6 {
            dense.push_str(&format!("## 節{i}\n\n{}。\n\n", "本文".repeat(95)));
        }
        let d = run(&StructureDensity::new(), &dense);
        assert_eq!(d.len(), 1);
        assert_eq!(matched(&dense, &d), vec!["節0"]);
        assert!(d[0].message.contains("見出しが 5.0 個"));
        assert!(!d[0].message.contains("箇条書き"));

        let mut sparse = String::new();
        for i in 0..2 {
            sparse.push_str(&format!("## 節{i}\n\n{}。\n\n", "本文".repeat(295)));
        }
        assert!(run(&StructureDensity::new(), &sparse).is_empty());

        let mut list = String::from("手順を並べる。\n\n");
        for i in 0..15 {
            list.push_str(&format!("- 項目{i:02}の説明{}\n", "文".repeat(60)));
        }
        let d = run(&StructureDensity::new(), &list);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("箇条書きの項目が 14.2 個"));
    }

    #[test]
    fn s09_needs_enough_text_and_measures_two_densities() {
        let short = "## 一\n\n## 二\n\n## 三\n\n## 四\n\n## 五\n";
        assert!(run(&StructureDensity::new(), short).is_empty());
        assert!(measure_md(&StructureDensity::new(), short).is_empty());

        let mut md = String::new();
        for i in 0..2 {
            md.push_str(&format!(
                "## 節{i}\n\n{}。\n\n- 項目\n\n",
                "本文".repeat(245)
            ));
        }
        let per_1000 = 2000.0 / md.chars().count() as f64;
        let m = measure_md(&StructureDensity::new(), &md);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].threshold_key, "headings_per_1000");
        assert!((m[0].value - per_1000).abs() < 1e-9);
        assert_eq!(m[1].threshold_key, "list_items_per_1000");
        assert!((m[1].value - per_1000).abs() < 1e-9);
        assert!(run(&StructureDensity::new(), &md).is_empty());
    }

    #[test]
    fn s10_flags_emoji_and_label_leads() {
        let md = "✅ 申請が完了しました。\n\n- 💡 ポイント: 早めに出す\n- 通常の項目\n\n注意点：申請の前に上長の承認を取ってください。\n\n**重要**: 締め日は月末です。\n\n普通の段落です。\n";
        let d = run(&LabelStyle::new(), md);
        assert_eq!(matched(md, &d), vec!["✅", "💡", "注意点：", "**重要**:"]);
        assert!(d[0].message.starts_with("段落を絵文字"));
        assert!(d[1].message.starts_with("箇条書きの項目を絵文字"));
        assert!(d[2].message.contains("「注意点：」"));
        assert!(d[3].message.contains("「重要:」"));
        assert!(d[3].message.ends_with("(文書全体で 4 か所)"));
    }

    #[test]
    fn s10_needs_several_leads_in_the_document() {
        let md = "注意点：申請の前に上長の承認を取ってください。\n\n✅ 申請が完了しました。\n";
        assert!(run(&LabelStyle::new(), md).is_empty());
        let mut r = LabelStyle::new();
        r.configure("min_count", &toml::Value::Integer(1)).unwrap();
        assert_eq!(run(&r, md).len(), 2);
        assert_eq!(r.options(), vec![("min_count", "1".to_string())]);
        assert!(r.configure("min_count", &toml::Value::Integer(0)).is_err());
        assert!(r.configure("nope", &toml::Value::Integer(1)).is_err());
    }

    #[test]
    fn s10_ignores_times_urls_short_labels_lists_quotes_and_english() {
        let md = "10:30 に会議室へ集合します。\n\n午前10:30に始めます。\n\n比率は 1:2 です。\n\nhttps://example.com で確認できます。\n\n例：申請書の書き方を示す。\n\n- 手順: 申請書を出す\n\n> 注意点：引用の中は見ない。\n\nNote: English paragraphs are skipped.\n\n✅ Done.\n";
        let mut r = LabelStyle::new();
        r.configure("min_count", &toml::Value::Integer(1)).unwrap();
        assert!(run(&r, md).is_empty());
    }

    #[test]
    fn configure_new_structure_rules() {
        let mut t = HeadingTemplate::new();
        t.configure("min_headings", &toml::Value::Integer(2))
            .unwrap();
        assert!(
            t.configure("min_headings", &toml::Value::Integer(0))
                .is_err()
        );
        assert!(
            t.configure("ratio_threshold", &toml::Value::Float(1.2))
                .is_err()
        );
        assert!(t.configure("nope", &toml::Value::Integer(1)).is_err());
        assert_eq!(t.options()[0], ("min_headings", "2".to_string()));

        let mut s = StructureDensity::new();
        s.configure("min_chars", &toml::Value::Integer(1)).unwrap();
        s.configure("headings_per_1000", &toml::Value::Float(50.0))
            .unwrap();
        assert_eq!(run(&s, "## 一\n\n本文。\n").len(), 1);
        assert!(
            s.configure("list_items_per_1000", &toml::Value::Float(-1.0))
                .is_err()
        );
        assert!(s.configure("nope", &toml::Value::Integer(1)).is_err());
        assert_eq!(s.options().len(), 3);

        let mut e = HeadingEmphasis;
        assert!(e.configure("x", &toml::Value::Integer(1)).is_err());
        assert!(e.options().is_empty());
    }

    #[test]
    fn s01_flags_dense_bold() {
        let md = "**大切なのは**、**早めの申請**と**正確な記入**です。\n";
        let d = run(&BoldDensity::new(), md);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].rule_id, "S01");
        assert_eq!(matched(md, &d), vec!["**大切なのは**"]);
        assert_eq!(d[0].related.len(), 3);
    }

    #[test]
    fn s01_ignores_sparse_bold() {
        let mut md = String::from("**要点**だけを太字にした。\n\n");
        md.push_str(&"本文の説明を続ける。".repeat(150));
        md.push_str("\n\n**二つ目**と**三つ目**。\n");
        assert!(run(&BoldDensity::new(), &md).is_empty());
    }

    #[test]
    fn s02_flags_list_heavy_documents() {
        let mut md = String::new();
        for i in 0..6 {
            md.push_str(&format!("段落{i}の本文。\n\n"));
        }
        for i in 0..4 {
            md.push_str(&format!("- 項目{i}\n"));
        }
        let d = run(&BulletRatio::new(), &md);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("10 個のうち 4 個"));
        let few = "段落。\n\n- 項目\n- 項目\n";
        assert!(run(&BulletRatio::new(), few).is_empty());
    }

    #[test]
    fn s03_flags_boilerplate_headings() {
        let md = "# 申請の手引き\n\n本文。\n\n## まとめ\n\n本文。\n\n## Conclusion\n";
        let d = run(&BoilerplateHeading, md);
        assert_eq!(matched(md, &d), vec!["まとめ", "Conclusion"]);
    }

    #[test]
    fn s04_counts_numbered_phases_everywhere() {
        let md = "## フェーズ1: 設計\n\n- ステップ2で試す\n- ステージ 3 で広げる\n";
        let d = run(&NumberedPhases::new(), md);
        assert_eq!(d.len(), 1);
        assert_eq!(matched(md, &d), vec!["フェーズ1"]);
        assert_eq!(d[0].related.len(), 3);
        assert!(run(&NumberedPhases::new(), "フェーズ1とフェーズ2。\n").is_empty());
    }

    #[test]
    fn s05_flags_emoji_but_not_in_code() {
        let md = "✅ 申請はオンラインで完結します 🚀 📌\n";
        assert_eq!(run(&EmojiDensity::new(), md).len(), 1);
        let code = "```\n✅ 🚀 📌 ⭐\n```\n\n本文です。\n";
        assert!(run(&EmojiDensity::new(), code).is_empty());
    }

    #[test]
    fn s06_flags_bold_label_lists() {
        let md = "- **速度**: 起動が速い\n- **安全性**: 権限を絞れる\n- **拡張性:** プラグインを足せる\n";
        let d = run(&BoldLabelList::new(), md);
        assert_eq!(d.len(), 3);
        assert!(d[2].message.contains("「**拡張性**: 説明」"));
        let plain = "- 速度: 起動が速い\n- **安全性**は高い\n- 拡張性\n";
        assert!(run(&BoldLabelList::new(), plain).is_empty());
    }

    #[test]
    fn configure_structure_rules() {
        let mut r = BoldDensity::new();
        r.configure("min_count", &toml::Value::Integer(1)).unwrap();
        r.configure("per_1000", &toml::Value::Float(0.0)).unwrap();
        assert_eq!(run(&r, "**一つ**だけ。\n").len(), 1);
        assert!(r.configure("per_1000", &toml::Value::Float(-1.0)).is_err());
        assert!(r.configure("nope", &toml::Value::Integer(1)).is_err());
        let mut b = BulletRatio::new();
        assert!(
            b.configure("ratio_threshold", &toml::Value::Float(1.5))
                .is_err()
        );
        let mut h = BoilerplateHeading;
        assert!(h.configure("x", &toml::Value::Integer(1)).is_err());
    }
}
