//! 端末向けの出力。
//!
//! ファイルごとに、指摘をレーンの節 (AI 臭さ → 独自ルール → 読みやすさ) に分けて並べ、
//! 各指摘の見出し行にもレーン名を付ける。末尾の要約も同じ呼び名でレーンごとに数えるので、
//! 要約の件数が一覧のどの指摘に当たるかを見て取れる。
//!
//! ```text
//! 📄 docs/meeting.md  自然度 64/100 (要修正)
//!   AI 臭さの指摘 1 件
//!   3:35  警告  [AI 臭さ]  P01 AI_CONCLUSION
//!     「と言えるだろう」は結論を定型句で押し付ける締めです
//!     │ このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。
//!     │                                                                     ^^^^^^^^^^^^^^
//!     💡 定型句を外して言い切るか、結論を支える事実や数値を書いてください
//!
//!   読みやすさの指摘 1 件 (自然度には入りません)
//!   5:1  情報  [読みやすさ]  P15 KANJI_RUN
//!     漢字が 8 字続いています (「全社業務改善計画」)
//!     │ 全社業務改善計画に組み込む。
//!     │ ^^^^^^^^^^^^^^^^
//!     💡 語の切れ目が読み取れるか確かめ、助詞や動詞を補って開いてください
//!
//! ✖ AI 臭さの指摘 1 件 (警告 1)、読みやすさの指摘 1 件 (情報 1) — 1 ファイルを検査
//! ```
//!
//! 色は ANSI エスケープで書き、端末でなければ呼び出し側 (anstream) が取り除く。

use std::io::{self, Write};

use anstyle::{AnsiColor, Style};
use unicode_width::UnicodeWidthStr;

use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
use crate::engine::{FileReport, RunReport};
use crate::output::RenderOptions;

/// 文脈 (該当する文) の表示幅の上限。超えたら指摘箇所の周りだけを切り出す。
const MAX_CONTEXT_WIDTH: usize = 100;
/// 切り出すとき、指摘箇所の前に残す表示幅。
const LEAD_WIDTH: usize = 24;

/// 節と要約に並べるレーンの順。主目的の AI 臭さを先に、優先度の低い読みやすさを最後に置く。
const LANES: [Lane; 3] = [Lane::Slop, Lane::Custom, Lane::Readability];

fn severity_style(s: Severity) -> Style {
    match s {
        Severity::Error => AnsiColor::Red.on_default().bold(),
        Severity::Warning => AnsiColor::Yellow.on_default().bold(),
        Severity::Info => AnsiColor::Cyan.on_default(),
    }
}

fn dim() -> Style {
    Style::new().dimmed()
}

fn bold() -> Style {
    Style::new().bold()
}

/// 実行結果をすべて書き出す。
pub fn render(report: &RunReport, opts: &RenderOptions, out: &mut dyn Write) -> io::Result<()> {
    for file in &report.files {
        render_file(file, opts, out)?;
    }
    if !opts.quiet {
        render_summary(report, opts, out)?;
    }
    Ok(())
}

fn render_file(file: &FileReport, opts: &RenderOptions, out: &mut dyn Write) -> io::Result<()> {
    let shown: Vec<&Diagnostic> = file
        .diagnostics
        .iter()
        .filter(|d| opts.show_suppressed || !d.is_suppressed())
        .collect();
    if opts.quiet && shown.is_empty() && file.warnings.is_empty() {
        return Ok(());
    }
    let b = bold();
    write!(out, "📄 {b}{}{b:#}", file.path())?;
    if let Some(score) = file.score {
        write!(
            out,
            "  自然度 {}/100 ({})",
            score.value,
            score.band.label_ja()
        )?;
    }
    if shown.is_empty() {
        let d = dim();
        write!(out, "  {d}指摘なし{d:#}")?;
    }
    writeln!(out)?;
    let mut first = true;
    for lane in LANES {
        // 節の中は文書の順 (`diagnostics` は位置とルール ID の順に並んでいる)
        let section: Vec<&Diagnostic> = shown.iter().copied().filter(|d| d.lane == lane).collect();
        if section.is_empty() {
            continue;
        }
        if !first {
            writeln!(out)?;
        }
        first = false;
        render_section_heading(lane, &section, out)?;
        for d in section {
            render_diagnostic(file, d, out)?;
        }
    }
    for w in &file.warnings {
        let s = AnsiColor::Yellow.on_default();
        writeln!(out, "  {s}⚠ {w}{s:#}")?;
    }
    writeln!(out)
}

/// 節の見出し (「AI 臭さの指摘 1 件」)。要約と同じ呼び名で、抑制していない指摘を数える。
/// 抑制した指摘を出しているときは、その件数を別に添える。
fn render_section_heading(
    lane: Lane,
    section: &[&Diagnostic],
    out: &mut dyn Write,
) -> io::Result<()> {
    let suppressed = section.iter().filter(|d| d.is_suppressed()).count();
    let (b, dm) = (bold(), dim());
    write!(
        out,
        "  {b}{}の指摘 {} 件{b:#}",
        lane.label_ja(),
        section.len() - suppressed
    )?;
    if suppressed > 0 {
        write!(out, "、抑制 {suppressed} 件")?;
    }
    if lane != Lane::Slop {
        write!(out, " {dm}(自然度には入りません){dm:#}")?;
    }
    writeln!(out)
}

fn render_diagnostic(file: &FileReport, d: &Diagnostic, out: &mut dyn Write) -> io::Result<()> {
    let doc = &file.doc;
    let (line, col) = doc.line_col(d.span.start);
    let sev = severity_style(d.severity);
    let dm = dim();
    let b = bold();
    // 節の見出しが画面の外に流れても、どのレーンの指摘かが分かるよう、各行にもレーン名を出す
    write!(
        out,
        "  {dm}{line}:{col}{dm:#}  {sev}{}{sev:#}  [{}]  {b}{}{b:#} {}",
        d.severity.label_ja(),
        d.lane.label_ja(),
        d.rule_id,
        d.rule_name
    )?;
    if d.status == RuleStatus::Experimental {
        write!(out, " {dm}[実験的]{dm:#}")?;
    }
    writeln!(out)?;
    writeln!(out, "    {}", d.message)?;
    if let Some(context) = d.context {
        let (text, caret) = excerpt(&doc.source, context, d.span);
        writeln!(out, "    {dm}│{dm:#} {text}")?;
        if let Some((col, width)) = caret {
            writeln!(
                out,
                "    {dm}│{dm:#} {}{sev}{}{sev:#}",
                " ".repeat(col),
                "^".repeat(width)
            )?;
        }
    }
    if let Some(hint) = &d.hint {
        let g = AnsiColor::Green.on_default();
        writeln!(out, "    {g}💡 {hint}{g:#}")?;
    }
    if let Some(s) = &d.suppressed {
        match &s.reason {
            Some(reason) => writeln!(out, "    {dm}↳ 抑制済み (L{}: {reason}){dm:#}", s.line)?,
            None => writeln!(out, "    {dm}↳ 抑制済み (L{}){dm:#}", s.line)?,
        }
    }
    Ok(())
}

/// 文脈の表示用文字列と、キャレットの (開始列, 幅) を返す。
///
/// 改行・タブは空白 1 つに置き換える (バイト長が同じなので位置はずれない)。
/// 表示幅が上限を超えるときは、指摘箇所の少し前から上限までを切り出し、両端に `…` を付ける。
pub(crate) fn excerpt(
    source: &str,
    context: Span,
    highlight: Span,
) -> (String, Option<(usize, usize)>) {
    let raw = &source[context.range()];
    let text: String = raw
        .chars()
        .map(|c| {
            if matches!(c, '\n' | '\r' | '\t') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let within = highlight.start >= context.start && highlight.end <= context.end;
    if !within {
        return (truncate(&text, MAX_CONTEXT_WIDTH), None);
    }
    let rel_start = highlight.start - context.start;
    let rel_end = highlight.end - context.start;

    if text.width() <= MAX_CONTEXT_WIDTH {
        let col = text[..rel_start].width();
        let width = text[rel_start..rel_end].width().max(1);
        return (text, Some((col, width)));
    }

    // 指摘箇所の LEAD_WIDTH ぶん手前から切り出す
    let mut start = rel_start;
    let mut lead = 0;
    for (i, c) in text[..rel_start].char_indices().rev() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if lead + w > LEAD_WIDTH {
            break;
        }
        lead += w;
        start = i;
    }
    let prefix = if start > 0 { "…" } else { "" };
    let mut end = start;
    let mut used = prefix.width();
    for (i, c) in text[start..].char_indices() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > MAX_CONTEXT_WIDTH - 1 {
            break;
        }
        used += w;
        end = start + i + c.len_utf8();
    }
    let suffix = if end < text.len() { "…" } else { "" };
    let shown = format!("{prefix}{}{suffix}", &text[start..end]);
    let col = prefix.width() + text[start..rel_start].width();
    let hl_end = rel_end.min(end);
    let width = text[rel_start..hl_end.max(rel_start)].width().max(1);
    (shown, Some((col, width)))
}

fn truncate(text: &str, max: usize) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    let mut used = 0;
    let mut out = String::new();
    for c in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > max - 1 {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

/// 1 つのレーンの、抑制していない指摘の重大度ごとの件数。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SeverityCounts {
    pub error: usize,
    pub warning: usize,
    pub info: usize,
}

impl SeverityCounts {
    fn add(&mut self, severity: Severity) {
        match severity {
            Severity::Error => self.error += 1,
            Severity::Warning => self.warning += 1,
            Severity::Info => self.info += 1,
        }
    }

    pub(crate) fn total(&self) -> usize {
        self.error + self.warning + self.info
    }

    /// 0 件でない重大度の内訳 (`警告 2・情報 1`)。重い順に並べる。
    fn breakdown(&self) -> String {
        [
            (Severity::Error, self.error),
            (Severity::Warning, self.warning),
            (Severity::Info, self.info),
        ]
        .into_iter()
        .filter(|&(_, n)| n > 0)
        .map(|(s, n)| format!("{} {n}", s.label_ja()))
        .collect::<Vec<_>>()
        .join("・")
    }
}

/// 件数の集計。抑制していない指摘はレーンごとに、抑制した指摘はまとめて数える。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Counts {
    pub slop: SeverityCounts,
    pub custom: SeverityCounts,
    pub readability: SeverityCounts,
    pub suppressed: usize,
}

impl Counts {
    pub(crate) fn of(report: &RunReport) -> Self {
        let mut c = Counts::default();
        for d in report.files.iter().flat_map(|f| f.diagnostics.iter()) {
            if d.is_suppressed() {
                c.suppressed += 1;
                continue;
            }
            match d.lane {
                Lane::Slop => c.slop.add(d.severity),
                Lane::Custom => c.custom.add(d.severity),
                Lane::Readability => c.readability.add(d.severity),
            }
        }
        c
    }

    pub(crate) fn lane(&self, lane: Lane) -> SeverityCounts {
        match lane {
            Lane::Slop => self.slop,
            Lane::Custom => self.custom,
            Lane::Readability => self.readability,
        }
    }
}

/// 要約の行。節の見出しと同じ呼び名で、レーンごとに件数と重大度の内訳を並べる。
///
/// AI 臭さは主目的なので 0 件でも出し、独自ルールと読みやすさは 1 件以上のときだけ出す。
/// ✖ は、AI 臭さか独自ルールの指摘があるとき (読みやすさの指摘だけなら ✔) か、`--fail-on` に
/// 当たったとき。既定の `--fail-on never` では指摘があっても終了コードは 0 なので、印を
/// 終了コードだけで決めると AI 臭さの指摘があっても ✔ になってしまう。
fn render_summary(report: &RunReport, opts: &RenderOptions, out: &mut dyn Write) -> io::Result<()> {
    let c = Counts::of(report);
    let tripped = report.trips(opts.fail_on);
    let mark = if c.slop.total() + c.custom.total() > 0 || tripped {
        let r = AnsiColor::Red.on_default().bold();
        format!("{r}✖{r:#}")
    } else {
        let g = AnsiColor::Green.on_default().bold();
        format!("{g}✔{g:#}")
    };
    let mut parts = Vec::new();
    for lane in LANES {
        let n = c.lane(lane);
        if lane == Lane::Slop || n.total() > 0 {
            parts.push(lane_count(lane, n));
        }
    }
    if c.suppressed > 0 {
        parts.push(format!("抑制 {} 件", c.suppressed));
    }
    if tripped {
        parts.push(format!("--fail-on {} に該当", opts.fail_on));
    }
    let mut line = format!("{mark} {}", parts.join("、"));
    line.push_str(&format!(" — {} ファイルを検査", report.files.len()));
    if !report.errors.is_empty() {
        line.push_str(&format!(
            "、読めなかったファイル {} 件",
            report.errors.len()
        ));
    }
    if let Some(method) = report.morphology.describe() {
        line.push_str(&format!("、{method}"));
    }
    writeln!(out, "{line}")
}

/// 要約の 1 レーンぶん (「AI 臭さの指摘 3 件 (警告 2・情報 1)」)。0 件なら内訳を付けない。
fn lane_count(lane: Lane, n: SeverityCounts) -> String {
    let mut s = format!("{}の指摘 {} 件", lane.label_ja(), n.total());
    if n.total() > 0 {
        s.push_str(&format!(" ({})", n.breakdown()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::FailOn;
    use crate::document::Document;
    use crate::engine::{Engine, EngineOptions};

    fn plain(bytes: Vec<u8>) -> String {
        anstream::adapter::strip_str(&String::from_utf8(bytes).unwrap()).to_string()
    }

    fn engine() -> Engine {
        Engine::with_rules(crate::engine::tests::test_rules(), EngineOptions::default()).unwrap()
    }

    fn report(files: Vec<FileReport>) -> RunReport {
        RunReport {
            files,
            errors: Vec::new(),
            morphology: Default::default(),
        }
    }

    fn render_str(report: &RunReport, opts: &RenderOptions) -> String {
        let mut buf = Vec::new();
        render(report, opts, &mut buf).unwrap();
        plain(buf)
    }

    /// 独自ルールの指摘 1 件を持つファイル (組み込みのテスト用ルールには独自ルールがないため)。
    fn custom_file(source: &str, needle: &str) -> FileReport {
        let doc = Document::markdown(source);
        let start = doc.source.find(needle).unwrap();
        let span = Span::new(start, start + needle.len());
        let d = Diagnostic::new(
            "X01",
            "TEAM_TERM",
            Severity::Warning,
            Lane::Custom,
            RuleStatus::Stable,
            span,
            "用語集と違う表記です",
        )
        .with_context(Span::new(0, doc.source.trim_end().len()));
        FileReport {
            doc,
            diagnostics: vec![d],
            warnings: Vec::new(),
            score: None,
        }
    }

    #[test]
    fn caret_accounts_for_full_width_characters() {
        let src = "このように、それは働き方だと言えるだろう。";
        let hl_start = src.find("と言える").unwrap();
        let hl = Span::new(hl_start, hl_start + "と言えるだろう".len());
        let (text, caret) = excerpt(src, Span::new(0, src.len()), hl);
        assert_eq!(text, src);
        // 「このように、それは働き方だ」は 13 字 (全角 2 幅) → 26 列
        assert_eq!(caret, Some((26, 14)));
    }

    #[test]
    fn long_context_is_windowed_around_the_highlight() {
        let src = format!("{}ここが対象{}", "前".repeat(80), "後".repeat(80));
        let start = src.find("ここ").unwrap();
        let hl = Span::new(start, start + "ここが対象".len());
        let (text, caret) = excerpt(&src, Span::new(0, src.len()), hl);
        assert!(text.starts_with('…') && text.ends_with('…'), "{text}");
        assert!(text.width() <= MAX_CONTEXT_WIDTH);
        let (col, width) = caret.unwrap();
        assert_eq!(width, 10);
        let before: String = text.chars().take_while(|&c| c != 'こ').collect();
        assert_eq!(col, before.width());
    }

    #[test]
    fn newlines_in_context_become_spaces() {
        let src = "一行目\n二行目";
        let (text, caret) = excerpt(
            src,
            Span::new(0, src.len()),
            Span::new(src.find('二').unwrap(), src.len()),
        );
        assert_eq!(text, "一行目 二行目");
        assert_eq!(caret, Some((7, 6)));
    }

    #[test]
    fn renders_file_diagnostics_and_summary() {
        let e = engine();
        let report = report(vec![
            e.lint(Document::markdown(
                "これは言えるでしょう。上限の設定の検討。\n",
            )),
            e.lint(Document::markdown("問題のない文。\n")),
        ]);
        let s = render_str(&report, &RenderOptions::default());
        assert!(s.contains("📄 <input>.md"), "{s}");
        assert!(s.contains("  AI 臭さの指摘 1 件\n"), "{s}");
        assert!(s.contains("1:4  警告  [AI 臭さ]  T01 STABLE_SLOP\n"), "{s}");
        assert!(
            s.contains("  読みやすさの指摘 1 件 (自然度には入りません)\n"),
            "{s}"
        );
        assert!(s.contains("  情報  [読みやすさ]  T03 READABILITY\n"), "{s}");
        assert!(s.contains("指摘なし"), "{s}");
        assert!(
            s.contains(
                "✖ AI 臭さの指摘 1 件 (警告 1)、読みやすさの指摘 1 件 (情報 1) — 2 ファイルを検査"
            ),
            "{s}"
        );

        let quiet = RenderOptions {
            quiet: true,
            ..Default::default()
        };
        let s = render_str(&report, &quiet);
        assert!(!s.contains("指摘なし"), "{s}");
        assert!(!s.contains("ファイルを検査"), "{s}");
    }

    #[test]
    fn diagnostics_are_grouped_by_lane_in_document_order() {
        let e = engine();
        // 文書の順では読みやすさの指摘が先に来るが、節は AI 臭さを先に並べる
        let report = report(vec![e.lint(Document::markdown(
            "上限の設定の検討をする。これは言えるでしょう。\n\n比較の設定の手順。また言えるでしょう。\n",
        ))]);
        let s = render_str(&report, &RenderOptions::default());
        let slop = s.find("  AI 臭さの指摘 2 件\n").expect("AI 臭さの節");
        let readability = s
            .find("  読みやすさの指摘 2 件 (自然度には入りません)\n")
            .expect("読みやすさの節");
        assert!(slop < readability, "{s}");
        // 節の中は文書の順
        let first = s.find("1:16  警告  [AI 臭さ]  T01").expect("1 つ目の T01");
        let second = s.find("3:12  警告  [AI 臭さ]  T01").expect("2 つ目の T01");
        assert!(
            slop < first && first < second && second < readability,
            "{s}"
        );
        let t03 = s.find("1:3  情報  [読みやすさ]  T03").expect("T03");
        assert!(readability < t03, "{s}");
        // 節と節のあいだは空行で区切る
        assert!(s.contains("\n\n  読みやすさの指摘 2 件"), "{s}");
    }

    #[test]
    fn experimental_findings_keep_their_tag() {
        let e = Engine::with_rules(
            crate::engine::tests::test_rules(),
            EngineOptions {
                experimental: true,
                ..Default::default()
            },
        )
        .unwrap();
        let report = report(vec![e.lint(Document::markdown("様々な案がある。\n"))]);
        let s = render_str(&report, &RenderOptions::default());
        assert!(
            s.contains("1:1  情報  [AI 臭さ]  T02 EXPERIMENTAL_SLOP [実験的]\n"),
            "{s}"
        );
    }

    #[test]
    fn custom_rules_get_their_own_section_and_count() {
        let e = engine();
        let report = report(vec![
            e.lint(Document::markdown("上限の設定の検討。\n")),
            custom_file("ユーザー様に連絡する。\n", "ユーザー様"),
        ]);
        let s = render_str(&report, &RenderOptions::default());
        assert!(
            s.contains("  独自ルールの指摘 1 件 (自然度には入りません)\n"),
            "{s}"
        );
        assert!(
            s.contains("1:1  警告  [独自ルール]  X01 TEAM_TERM\n"),
            "{s}"
        );
        // 独自ルールの指摘があれば、AI 臭さが 0 件でも ✖
        assert!(
            s.contains(
                "✖ AI 臭さの指摘 0 件、独自ルールの指摘 1 件 (警告 1)、読みやすさの指摘 1 件 (情報 1) — 2 ファイルを検査"
            ),
            "{s}"
        );
    }

    #[test]
    fn readability_alone_is_not_marked_as_a_failure() {
        let e = engine();
        let report = report(vec![e.lint(Document::markdown("上限の設定の検討。\n"))]);
        let s = render_str(&report, &RenderOptions::default());
        assert!(
            s.contains("✔ AI 臭さの指摘 0 件、読みやすさの指摘 1 件 (情報 1) — 1 ファイルを検査"),
            "{s}"
        );

        // --fail-on に当たれば、読みやすさの指摘だけでも ✖ にして理由を添える
        let fail = RenderOptions {
            fail_on: FailOn::At(Severity::Info),
            ..Default::default()
        };
        let s = render_str(&report, &fail);
        assert!(
            s.contains(
                "✖ AI 臭さの指摘 0 件、読みやすさの指摘 1 件 (情報 1)、--fail-on info に該当 — 1 ファイルを検査"
            ),
            "{s}"
        );
    }

    #[test]
    fn no_findings_say_so_in_the_summary() {
        let e = engine();
        let report = report(vec![e.lint(Document::markdown("問題のない文。\n"))]);
        let s = render_str(&report, &RenderOptions::default());
        assert!(s.contains("✔ AI 臭さの指摘 0 件 — 1 ファイルを検査"), "{s}");
    }

    #[test]
    fn severity_breakdown_lists_only_present_levels_heaviest_first() {
        let n = SeverityCounts {
            error: 1,
            warning: 0,
            info: 2,
        };
        assert_eq!(n.breakdown(), "重大 1・情報 2");
        assert_eq!(
            lane_count(Lane::Slop, n),
            "AI 臭さの指摘 3 件 (重大 1・情報 2)"
        );
        assert_eq!(
            lane_count(Lane::Readability, SeverityCounts::default()),
            "読みやすさの指摘 0 件"
        );
    }

    #[test]
    fn suppressed_diagnostics_are_hidden_unless_requested() {
        let e = engine();
        let report = report(vec![e.lint(Document::markdown(
            "<!-- noslop-disable-next-line T01 -- 引用のため -->\nこれは言えるでしょう。\n",
        ))]);
        let s = render_str(&report, &RenderOptions::default());
        assert!(!s.contains("T01"), "{s}");
        assert!(
            !s.contains("  AI 臭さの指摘"),
            "抑制した指摘だけなら節を出さない: {s}"
        );
        assert!(s.contains("✔ AI 臭さの指摘 0 件、抑制 1 件"), "{s}");
        let show = RenderOptions {
            show_suppressed: true,
            ..Default::default()
        };
        let s = render_str(&report, &show);
        assert!(s.contains("  AI 臭さの指摘 0 件、抑制 1 件\n"), "{s}");
        assert!(s.contains("↳ 抑制済み (L1: 引用のため)"), "{s}");
    }
}
