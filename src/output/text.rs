//! 端末向けの出力。
//!
//! ```text
//! 📄 docs/meeting.md  自然度 64/100 (要修正)
//!   3:35  警告  P01 AI_CONCLUSION
//!     「と言えるだろう」は結論を定型句で押し付ける締めです
//!     │ このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。
//!     │                                                                     ^^^^^^^^^^^^^^
//!     💡 定型句を外して言い切るか、結論を支える事実や数値を書いてください
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
        render_summary(report, out)?;
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
    for d in shown {
        render_diagnostic(file, d, out)?;
    }
    for w in &file.warnings {
        let s = AnsiColor::Yellow.on_default();
        writeln!(out, "  {s}⚠ {w}{s:#}")?;
    }
    writeln!(out)
}

fn render_diagnostic(file: &FileReport, d: &Diagnostic, out: &mut dyn Write) -> io::Result<()> {
    let doc = &file.doc;
    let (line, col) = doc.line_col(d.span.start);
    let sev = severity_style(d.severity);
    let dm = dim();
    let b = bold();
    write!(
        out,
        "  {dm}{line}:{col}{dm:#}  {sev}{}{sev:#}  {b}{}{b:#} {}",
        d.severity.label_ja(),
        d.rule_id,
        d.rule_name
    )?;
    let mut tags = Vec::new();
    match d.lane {
        Lane::Readability => tags.push("読解負荷"),
        Lane::Custom => tags.push("独自"),
        Lane::Slop => {}
    }
    if d.status == RuleStatus::Experimental {
        tags.push("実験的");
    }
    for tag in tags {
        write!(out, " {dm}[{tag}]{dm:#}")?;
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

/// 件数の集計。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Counts {
    pub error: usize,
    pub warning: usize,
    pub info: usize,
    pub readability: usize,
    pub suppressed: usize,
}

impl Counts {
    pub(crate) fn of(report: &RunReport) -> Self {
        let mut c = Counts::default();
        for d in report.files.iter().flat_map(|f| f.diagnostics.iter()) {
            if d.is_suppressed() {
                c.suppressed += 1;
            } else if d.lane == Lane::Readability {
                c.readability += 1;
            } else {
                match d.severity {
                    Severity::Error => c.error += 1,
                    Severity::Warning => c.warning += 1,
                    Severity::Info => c.info += 1,
                }
            }
        }
        c
    }

    pub(crate) fn main(&self) -> usize {
        self.error + self.warning + self.info
    }
}

fn render_summary(report: &RunReport, out: &mut dyn Write) -> io::Result<()> {
    let c = Counts::of(report);
    let mut line = if c.main() == 0 {
        let g = AnsiColor::Green.on_default().bold();
        format!("{g}✔ AI 臭さの指摘はありません{g:#}")
    } else {
        let r = AnsiColor::Red.on_default().bold();
        format!(
            "{r}✖ {} 件の指摘{r:#} (重大 {}・警告 {}・情報 {})",
            c.main(),
            c.error,
            c.warning,
            c.info
        )
    };
    if c.readability > 0 {
        line.push_str(&format!("、読解負荷の指さし {} 件", c.readability));
    }
    if c.suppressed > 0 {
        line.push_str(&format!("、抑制 {} 件", c.suppressed));
    }
    line.push_str(&format!(" — {} ファイルを検査", report.files.len()));
    if !report.errors.is_empty() {
        line.push_str(&format!(
            "、読めなかったファイル {} 件",
            report.errors.len()
        ));
    }
    writeln!(out, "{line}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::engine::{Engine, EngineOptions};

    fn plain(bytes: Vec<u8>) -> String {
        anstream::adapter::strip_str(&String::from_utf8(bytes).unwrap()).to_string()
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
        let engine =
            Engine::with_rules(crate::engine::tests::test_rules(), EngineOptions::default())
                .unwrap();
        let report = RunReport {
            files: vec![
                engine.lint(Document::markdown(
                    "これは言えるでしょう。上限の設定の検討。\n",
                )),
                engine.lint(Document::markdown("問題のない文。\n")),
            ],
            errors: Vec::new(),
        };
        let mut buf = Vec::new();
        render(&report, &RenderOptions::default(), &mut buf).unwrap();
        let s = plain(buf);
        assert!(s.contains("📄 <input>.md"), "{s}");
        assert!(s.contains("1:4  警告  T01 STABLE_SLOP"), "{s}");
        assert!(s.contains("[読解負荷]"), "{s}");
        assert!(s.contains("指摘なし"), "{s}");
        assert!(
            s.contains(
                "✖ 1 件の指摘 (重大 0・警告 1・情報 0)、読解負荷の指さし 1 件 — 2 ファイルを検査"
            ),
            "{s}"
        );

        let mut buf = Vec::new();
        let quiet = RenderOptions {
            quiet: true,
            ..Default::default()
        };
        render(&report, &quiet, &mut buf).unwrap();
        let s = plain(buf);
        assert!(!s.contains("指摘なし"), "{s}");
        assert!(!s.contains("ファイルを検査"), "{s}");
    }

    #[test]
    fn suppressed_diagnostics_are_hidden_unless_requested() {
        let engine =
            Engine::with_rules(crate::engine::tests::test_rules(), EngineOptions::default())
                .unwrap();
        let report = RunReport {
            files: vec![engine.lint(Document::markdown(
                "<!-- noslop-disable-next-line T01 -- 引用のため -->\nこれは言えるでしょう。\n",
            ))],
            errors: Vec::new(),
        };
        let mut buf = Vec::new();
        render(&report, &RenderOptions::default(), &mut buf).unwrap();
        let s = plain(buf);
        assert!(!s.contains("T01"), "{s}");
        assert!(s.contains("抑制 1 件"), "{s}");
        let mut buf = Vec::new();
        let show = RenderOptions {
            show_suppressed: true,
            ..Default::default()
        };
        render(&report, &show, &mut buf).unwrap();
        let s = plain(buf);
        assert!(s.contains("↳ 抑制済み (L1: 引用のため)"), "{s}");
    }
}
