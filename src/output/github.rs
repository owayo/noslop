//! GitHub Actions のワークフローコマンド (注釈) 形式。
//!
//! `::warning file=docs/a.md,line=3,col=4,endLine=3,endColumn=11,title=[AI 臭さ] P01 AI_CONCLUSION::メッセージ`
//!
//! 重大度は error → `error`、warning → `warning`、info → `notice` に対応させる。
//! 注釈の見出し (`title`) には、ルール ID の前にレーン名 (`[AI 臭さ]`・`[読みやすさ]`・
//! `[独自ルール]`) を付ける。抑制した指摘は出さない。

use std::io::{self, Write};

use crate::diagnostic::Severity;
use crate::engine::RunReport;

/// メッセージ部分のエスケープ。
fn escape_data(s: &str) -> String {
    s.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// プロパティ (`file=` や `title=`) のエスケープ。
fn escape_property(s: &str) -> String {
    escape_data(s).replace(':', "%3A").replace(',', "%2C")
}

fn level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "notice",
    }
}

/// 実行結果を注釈として書き出す。
pub fn render(report: &RunReport, out: &mut dyn Write) -> io::Result<()> {
    for file in &report.files {
        let path = escape_property(file.path());
        for d in file.visible() {
            let (line, col) = file.doc.line_col(d.span.start);
            let (end_line, end_col) = file.doc.line_col(d.span.end);
            let title = escape_property(&format!(
                "[{}] {} {}",
                d.lane.label_ja(),
                d.rule_id,
                d.rule_name
            ));
            let mut message = d.message.clone();
            if let Some(hint) = &d.hint {
                message.push('\n');
                message.push_str("💡 ");
                message.push_str(hint);
            }
            writeln!(
                out,
                "::{} file={path},line={line},col={col},endLine={end_line},endColumn={end_col},title={title}::{}",
                level(d.severity),
                escape_data(&message)
            )?;
        }
    }
    for e in &report.errors {
        writeln!(
            out,
            "::error file={},title=noslop::{}",
            escape_property(&e.path),
            escape_data(&e.message)
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::engine::{Engine, EngineOptions};

    #[test]
    fn escapes_data_and_properties() {
        assert_eq!(escape_data("100%\r\n次"), "100%25%0D%0A次");
        assert_eq!(escape_property("a:b,c%"), "a%3Ab%2Cc%25");
    }

    #[test]
    fn emits_one_annotation_per_visible_diagnostic() {
        let engine =
            Engine::with_rules(crate::engine::tests::test_rules(), EngineOptions::default())
                .unwrap();
        let mut report = RunReport::default();
        report.files.push(engine.lint(Document::markdown(
            "これは言えるでしょう。上限の設定の検討をする。\n\n<!-- noslop-disable-next-line -->\nそれも言えるでしょう。\n",
        )));
        let mut buf = Vec::new();
        render(&report, &mut buf).unwrap();
        let s = String::from_utf8(buf).unwrap();
        let lines: Vec<_> = s.lines().collect();
        assert_eq!(lines.len(), 2, "{s}");
        assert!(
            lines[0].starts_with(
                "::warning file=<input>.md,line=1,col=4,endLine=1,endColumn=11,title=[AI 臭さ] T01 STABLE_SLOP::"
            ),
            "{s}"
        );
        // どのレーンでも見出しにレーン名を付ける
        assert!(
            lines[1].starts_with("::notice file=<input>.md,line=1,")
                && lines[1].contains(",title=[読みやすさ] T03 READABILITY::"),
            "{s}"
        );
    }
}
