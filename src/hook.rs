//! エージェントのフック (`noslop hook claude-code`)。
//!
//! Claude Code の PostToolUse フックとして、Write / Edit / MultiEdit で書き換えた文書を検査し、
//! 指摘があれば短い改稿指示を `hookSpecificOutput.additionalContext` で Claude に渡す。
//! additionalContext はツールの結果の横にシステムリマインダーとして入り、ツールの失敗としては
//! 扱われない (`decision: "block"` や終了コード 2 は失敗に見えるので使わない)。
//!
//! 指摘がないとき・対象外のツールやファイルのときは何も出力せず、終了コード 0 で終わる。
//! 返すのは、今回のツール呼び出しで変わった行に重なる指摘だけ (編集のたびに同じ指摘を
//! 繰り返し渡して、残すと決めた箇所まで直させないため)。変わった行は、ツールの結果にある
//! 差分 (`tool_response.structuredPatch`) から求め、なければ Edit / MultiEdit の `new_string`
//! の位置から求める。位置を 1 つに決められないときは、ファイル全体の指摘を返す。

use std::collections::BTreeSet;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::cli::{self, ClaudeCodeArgs};
use crate::diagnostic::{Diagnostic, Span};
use crate::document::{Document, SourceFormat};
use crate::engine::{Engine, RunReport};
use crate::output::{self, RenderOptions, RuleCatalog};

/// 検査するツール。
const TOOLS: [&str; 3] = ["Write", "Edit", "MultiEdit"];

/// additionalContext に入れる文字数の上限。Claude Code は 10,000 文字を超えた値を
/// ファイルに逃がして先頭しか見せないので、余裕を見てそれより短く切る。
const CONTEXT_BUDGET_CHARS: usize = 9_000;

/// フックの入力 (標準入力の JSON) の上限。Write の入力には本文がまるごと入る。
const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;

/// 検査するファイルの上限。これより大きいファイルは、編集のたびに検査すると遅いので飛ばす。
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// `noslop hook claude-code` の本体。終了コードを返す。
///
/// 入力の誤りは標準エラーに書いて 1 で終わる (Claude Code では処理を止めないエラーになる)。
pub fn claude_code(args: &ClaudeCodeArgs) -> u8 {
    let mut input = String::new();
    match io::stdin()
        .take(MAX_INPUT_BYTES + 1)
        .read_to_string(&mut input)
    {
        Ok(n) if n as u64 > MAX_INPUT_BYTES => {
            eprintln!(
                "noslop: フックの入力が大きすぎます (上限 {} MiB)",
                MAX_INPUT_BYTES / 1024 / 1024
            );
            return 1;
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("noslop: フックの入力を読めません: {e}");
            return 1;
        }
    }
    match respond(&input, args) {
        Ok(Some(output)) => {
            let mut out = io::stdout().lock();
            match writeln!(out, "{output}").and_then(|()| out.flush()) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("noslop: 出力に失敗しました: {e}");
                    1
                }
            }
        }
        Ok(None) => 0,
        Err(message) => {
            eprintln!("noslop: {message}");
            1
        }
    }
}

/// フックの入力 (JSON) に対して標準出力に書く JSON。何も書かないなら `None`。
pub fn respond(input: &str, args: &ClaudeCodeArgs) -> Result<Option<String>, String> {
    let event: Value = serde_json::from_str(input)
        .map_err(|e| format!("フックの入力を JSON として読めません: {e}"))?;
    if let Some(name) = event.get("hook_event_name").and_then(Value::as_str)
        && name != "PostToolUse"
    {
        return Ok(None);
    }
    let Some(tool) = event
        .get("tool_name")
        .and_then(Value::as_str)
        .filter(|t| TOOLS.contains(t))
    else {
        return Ok(None);
    };
    let Some(file_path) = event
        .pointer("/tool_input/file_path")
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let cwd = event.get("cwd").and_then(Value::as_str).map(PathBuf::from);
    let path = match &cwd {
        Some(dir) if Path::new(file_path).is_relative() => dir.join(file_path),
        _ => PathBuf::from(file_path),
    };

    let cfg = cli::load_config_from(&args.config, cwd.as_deref()).map_err(|e| e.to_string())?;
    let walk = cli::walk_options(cfg.as_ref()).map_err(|e| e.to_string())?;
    if !crate::walk::has_extension(&path, &walk.extensions)
        || walk
            .exclude
            .as_ref()
            .is_some_and(|ex| ex.is_excluded(&path, false))
    {
        return Ok(None);
    }
    let name = display_name(&path, cwd.as_deref());
    let Some(source) = read_source(&path, &name)? else {
        return Ok(None);
    };

    let mut options = cli::config_engine_options(cfg.as_ref());
    if let Some(genre) = args.genre {
        options.genre = genre;
    }
    options.experimental |= args.experimental;
    options.selection.no_readability = !args.include_readability;
    let engine = Engine::new(options).map_err(|e| e.to_string())?;
    let mut file = engine.lint_source(name.clone(), source, SourceFormat::from_path(&path));

    let changed = if args.whole_file {
        None
    } else {
        changed_regions(tool, &event, &file.doc)
    };
    if let Some(regions) = &changed {
        file.diagnostics.retain(|d| touches(d, regions));
    }
    if file.visible().next().is_none() {
        return Ok(None);
    }

    let opts = RenderOptions {
        genre: engine.options().genre,
        experimental: engine.options().experimental,
        brief_limit: Some(args.brief_limit),
        brief_compact: true,
        catalog: RuleCatalog::from_engine(&engine),
        ..Default::default()
    };
    let report = RunReport {
        files: vec![file],
        errors: Vec::new(),
    };
    let mut buf = Vec::new();
    output::brief::render(&report, &opts, &mut buf).map_err(|e| e.to_string())?;
    let mut context = String::from_utf8(buf).map_err(|e| e.to_string())?;
    context.push_str(
        "このフックは編集のたびに動きます。一度見直して残すと決めた指摘は、再び出ても直す必要はありません。\n",
    );
    if changed.is_some() {
        context.push_str(&format!(
            "今回変わった行に重なる指摘だけを返しています。ファイル全体は `noslop check --format brief {name}` で確認できます。\n"
        ));
    }
    let context = truncate_lines(&context, CONTEXT_BUDGET_CHARS);
    let output = json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": context,
        }
    });
    Ok(Some(output.to_string()))
}

/// 表示名 (作業ディレクトリの中なら相対パス)。
fn display_name(path: &Path, cwd: Option<&Path>) -> String {
    let relative = cwd
        .and_then(|dir| path.strip_prefix(dir).ok())
        .filter(|p| !p.as_os_str().is_empty());
    crate::walk::display(relative.unwrap_or(path))
}

/// 検査するファイルを読む。消えたファイルと大きすぎるファイルは飛ばす (`None`)。
fn read_source(path: &Path, name: &str) -> Result<Option<String>, String> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{name} を読み込めません: {e}")),
    };
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{name} を読み込めません: {e}"))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        // 終了コード 0 の標準エラーは Claude Code のデバッグログにだけ残る
        eprintln!(
            "noslop: {name} は {} MiB を超えるので検査しません",
            MAX_FILE_BYTES / 1024 / 1024
        );
        return Ok(None);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| format!("{name} を UTF-8 として読めません"))
}

/// 今回のツール呼び出しで変わった行 (原文上の範囲)。決められないときは `None` (ファイル全体)。
fn changed_regions(tool: &str, event: &Value, doc: &Document) -> Option<Vec<Span>> {
    if tool == "Write"
        && event.pointer("/tool_response/type").and_then(Value::as_str) == Some("create")
    {
        return None;
    }
    if let Some(lines) = event
        .pointer("/tool_response/structuredPatch")
        .and_then(patch_lines)
    {
        return Some(
            lines
                .into_iter()
                .map(|line| doc.lines.line_span(&doc.source, line))
                .collect(),
        );
    }
    match tool {
        "Edit" | "MultiEdit" => inserted_regions(event.get("tool_input")?, doc),
        _ => None,
    }
}

/// 差分の hunk の配列 (`structuredPatch`) から、変わった後のファイルの行番号 (1 始まり) を集める。
/// 削除だけの箇所は、つなぎ目の前後の行を入れる。形が想定と違えば `None`。
fn patch_lines(patch: &Value) -> Option<BTreeSet<usize>> {
    let hunks = patch.as_array().filter(|h| !h.is_empty())?;
    let mut lines = BTreeSet::new();
    for hunk in hunks {
        let mut line = usize::try_from(hunk.get("newStart")?.as_u64()?).ok()?;
        for text in hunk.get("lines")?.as_array()? {
            match text.as_str()?.chars().next() {
                Some('+') => {
                    lines.insert(line.max(1));
                    line += 1;
                }
                Some('-') => {
                    lines.insert(line.saturating_sub(1).max(1));
                    lines.insert(line.max(1));
                }
                // 「\ No newline at end of file」
                Some('\\') => {}
                _ => line += 1,
            }
        }
    }
    Some(lines)
}

/// Edit / MultiEdit の `new_string` が入った行。位置を 1 つに決められないとき (同じ文字列が
/// ほかにもある、削除だけの編集、別のフックが整形して見つからない) は `None`。
fn inserted_regions(input: &Value, doc: &Document) -> Option<Vec<Span>> {
    let replace_all = |v: &Value| v.get("replace_all").and_then(Value::as_bool) == Some(true);
    let mut edits: Vec<(&str, bool)> = Vec::new();
    if let Some(s) = input.get("new_string").and_then(Value::as_str) {
        edits.push((s, replace_all(input)));
    }
    if let Some(list) = input.get("edits").and_then(Value::as_array) {
        for edit in list {
            edits.push((edit.get("new_string")?.as_str()?, replace_all(edit)));
        }
    }
    if edits.is_empty() {
        return None;
    }
    let source = &doc.source;
    let mut regions = Vec::new();
    for (text, all) in edits {
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
        if text.trim().is_empty() {
            return None;
        }
        let starts: Vec<usize> = source.match_indices(text).map(|(i, _)| i).collect();
        if starts.is_empty() || (starts.len() > 1 && !all) {
            return None;
        }
        for start in starts {
            let first = doc.lines.line(start);
            let last = doc.lines.line(start + text.len() - 1);
            regions.extend((first..=last).map(|line| doc.lines.line_span(source, line)));
        }
    }
    Some(regions)
}

/// 指摘の箇所か文脈 (文・段落) が、変わった行に重なるか。
fn touches(d: &Diagnostic, regions: &[Span]) -> bool {
    std::iter::once(d.span)
        .chain(d.context)
        .any(|s| regions.iter().any(|r| overlaps(s, *r)))
}

fn overlaps(a: Span, b: Span) -> bool {
    if a.start == a.end {
        b.start <= a.start && a.start <= b.end
    } else if b.start == b.end {
        a.start <= b.start && b.start < a.end
    } else {
        a.start < b.end && b.start < a.end
    }
}

/// `budget` 文字に収まるよう、行単位で後ろを省く。1 行が長すぎるときは行の途中で切る。
fn truncate_lines(text: &str, budget: usize) -> String {
    if text.chars().count() <= budget {
        return text.to_string();
    }
    // 省略の注記のぶんを残しておく
    let budget = budget.saturating_sub(100);
    let lines: Vec<&str> = text.lines().collect();
    let mut out = String::new();
    let mut used = 0;
    for (i, line) in lines.iter().enumerate() {
        let n = line.chars().count() + 1;
        if used + n > budget {
            let room = budget - used;
            if room > 20 {
                out.extend(line.chars().take(room - 2));
                out.push_str("…\n");
            }
            out.push_str(&format!(
                "(長いので残り {} 行を省きました。全件は `noslop check --format brief` で確認できます)\n",
                lines.len() - i
            ));
            break;
        }
        out.push_str(line);
        out.push('\n');
        used += n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    const CONFIG: &str = r#"
[[custom]]
id = "X01"
name = "TEAM_TERM"
pattern = "ユーザー様"
message = "「ユーザー様」ではなく「利用者」と書きます"
hint = "用語集の表記に合わせてください"
severity = "warning"
"#;

    fn args(extra: &[&str]) -> ClaudeCodeArgs {
        let mut argv = vec!["noslop", "hook", "claude-code"];
        argv.extend_from_slice(extra);
        match cli::Cli::try_parse_from(argv).unwrap().command {
            cli::Command::Hook(cli::HookCommand::ClaudeCode(a)) => a,
            _ => panic!("hook claude-code"),
        }
    }

    fn event(
        dir: &Path,
        tool: &str,
        file: &str,
        tool_input: Value,
        tool_response: Value,
    ) -> String {
        let mut input = tool_input;
        input["file_path"] = json!(dir.join(file).to_string_lossy());
        json!({
            "session_id": "s",
            "cwd": dir.to_string_lossy(),
            "hook_event_name": "PostToolUse",
            "tool_name": tool,
            "tool_input": input,
            "tool_response": tool_response
        })
        .to_string()
    }

    fn context(output: &str) -> String {
        let v: Value = serde_json::from_str(output).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn run(input: &str, extra: &[&str]) -> Option<String> {
        respond(input, &args(extra)).unwrap().map(|o| context(&o))
    }

    fn workspace(doc: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
        std::fs::write(dir.path().join("guide.md"), doc).unwrap();
        dir
    }

    const DOC: &str =
        "# 案内\n\nユーザー様の声を集めました。\n\n新しい段落です。ユーザー様に届けます。\n";

    #[test]
    fn write_returns_a_compact_brief_as_additional_context() {
        let dir = workspace("# 案内\n\nこの機能はユーザー様の声から生まれました。\n");
        let input = event(
            dir.path(),
            "Write",
            "guide.md",
            json!({ "content": "..." }),
            json!({ "type": "create", "structuredPatch": [] }),
        );
        let ctx = run(&input, &[]).expect("指摘があれば出力する");
        assert!(ctx.contains("guide.md"), "{ctx}");
        assert!(ctx.contains("X01"), "{ctx}");
        assert!(
            ctx.contains("L3: 「ユーザー様」ではなく「利用者」と書きます"),
            "{ctx}"
        );
        assert!(ctx.contains("直さない判断もできます"), "{ctx}");
        assert!(ctx.contains("再実行は 1 回だけ"), "{ctx}");
        assert!(
            !ctx.contains("今回変わった行"),
            "新しいファイルは全体: {ctx}"
        );
    }

    #[test]
    fn structured_patch_limits_findings_to_changed_lines() {
        let dir = workspace(DOC);
        let patch = json!({ "structuredPatch": [{
            "oldStart": 4, "oldLines": 2, "newStart": 4, "newLines": 2,
            "lines": [" ", "-段落です。ユーザー様に届けます。", "+新しい段落です。ユーザー様に届けます。"]
        }] });
        let input = event(
            dir.path(),
            "Edit",
            "guide.md",
            json!({ "old_string": "段落です。", "new_string": "新しい段落です。" }),
            patch,
        );
        let ctx = run(&input, &[]).unwrap();
        assert!(ctx.contains("L5:"), "{ctx}");
        assert!(!ctx.contains("L3:"), "変わっていない行は返さない: {ctx}");
        assert!(ctx.contains("今回変わった行に重なる指摘だけ"), "{ctx}");
        assert!(run(&input, &["--whole-file"]).unwrap().contains("L3:"));
    }

    #[test]
    fn deletions_report_the_lines_around_the_joint() {
        let dir = workspace(DOC);
        // 5 行目の前にあった行を消した
        let patch = json!({ "structuredPatch": [{
            "oldStart": 4, "oldLines": 3, "newStart": 4, "newLines": 2,
            "lines": [" ", "-消した行。", " 新しい段落です。ユーザー様に届けます。"]
        }] });
        let input = event(
            dir.path(),
            "Edit",
            "guide.md",
            json!({ "old_string": "消した行。\n", "new_string": "" }),
            patch,
        );
        let ctx = run(&input, &[]).unwrap();
        assert!(ctx.contains("L5:") && !ctx.contains("L3:"), "{ctx}");
    }

    #[test]
    fn without_a_patch_the_new_string_locates_the_edit() {
        let dir = workspace(DOC);
        let edit = |new: &str, replace_all: bool| {
            event(
                dir.path(),
                "Edit",
                "guide.md",
                json!({ "old_string": "x", "new_string": new, "replace_all": replace_all }),
                json!({}),
            )
        };
        let ctx = run(&edit("新しい段落です。ユーザー様に", false), &[]).unwrap();
        assert!(ctx.contains("L5:") && !ctx.contains("L3:"), "{ctx}");

        // 同じ文字列がほかにもある・削除だけ・見つからないときは、位置を決めずに全体を返す
        for new in ["ユーザー様", "", "見つからない文字列"] {
            let ctx = run(&edit(new, false), &[]).unwrap();
            assert!(ctx.contains("L3:") && ctx.contains("L5:"), "{new}: {ctx}");
        }
        let all = run(&edit("ユーザー様", true), &[]).unwrap();
        assert!(all.contains("L3:") && all.contains("L5:"), "{all}");

        assert_eq!(
            run(&edit("# 案内", false), &[]),
            None,
            "変わった行に指摘がなければ何も返さない"
        );
    }

    #[test]
    fn multi_edit_uses_every_new_string() {
        let doc = "ユーザー様の一行目。\n\n二行目。\n\n三行目のユーザー様。\n";
        let dir = workspace(doc);
        let edits = json!({ "edits": [
            { "old_string": "a", "new_string": "ユーザー様の一行目。" },
            { "old_string": "b", "new_string": "二行目。" }
        ] });
        let ctx = run(
            &event(dir.path(), "MultiEdit", "guide.md", edits, json!({})),
            &[],
        )
        .unwrap();
        assert!(ctx.contains("L1:") && !ctx.contains("L5:"), "{ctx}");
    }

    #[test]
    fn other_tools_events_and_files_are_ignored() {
        let dir = workspace("ユーザー様。\n");
        std::fs::write(dir.path().join("main.rs"), "// ユーザー様\n").unwrap();
        let ev = |tool: &str, file: &str| event(dir.path(), tool, file, json!({}), json!({}));
        assert_eq!(run(&ev("Read", "guide.md"), &[]), None);
        assert_eq!(run(&ev("Write", "main.rs"), &[]), None);
        assert_eq!(run(&ev("Write", "missing.md"), &[]), None);
        let pre = ev("Write", "guide.md").replace("PostToolUse", "PreToolUse");
        assert_eq!(run(&pre, &[]), None);
        assert!(respond("not json", &args(&[])).is_err());
    }

    #[test]
    fn clean_files_produce_no_output() {
        let dir = workspace("# メモ\n\n今日は晴れた。散歩に出かけた。\n");
        let input = event(dir.path(), "Write", "guide.md", json!({}), json!({}));
        assert_eq!(run(&input, &[]), None);
    }

    #[test]
    fn config_exclude_and_extensions_are_respected() {
        let dir = workspace("ユーザー様。\n");
        std::fs::write(
            dir.path().join("noslop.toml"),
            format!("{CONFIG}\n[files]\nextensions = [\"mdx\"]\nexclude = [\"drafts/\"]\n"),
        )
        .unwrap();
        std::fs::write(dir.path().join("page.mdx"), "ユーザー様。\n").unwrap();
        std::fs::create_dir(dir.path().join("drafts")).unwrap();
        std::fs::write(dir.path().join("drafts/a.mdx"), "ユーザー様。\n").unwrap();
        let ev = |file: &str| event(dir.path(), "Write", file, json!({}), json!({}));
        assert_eq!(run(&ev("guide.md"), &[]), None);
        assert_eq!(run(&ev("drafts/a.mdx"), &[]), None);
        assert!(run(&ev("page.mdx"), &[]).is_some());
    }

    #[test]
    fn patch_lines_follow_the_hunk_format() {
        let patch = json!([
            { "newStart": 2, "lines": [" a", "-b", "+B", " c", "\\ No newline at end of file"] },
            { "newStart": 10, "lines": ["-x"] }
        ]);
        let lines: Vec<usize> = patch_lines(&patch).unwrap().into_iter().collect();
        assert_eq!(lines, vec![2, 3, 9, 10]);
        assert_eq!(patch_lines(&json!([])), None);
        assert_eq!(patch_lines(&json!([{ "lines": ["+a"] }])), None);
        assert_eq!(patch_lines(&json!("diff")), None);
    }

    #[test]
    fn long_context_is_cut_within_the_budget() {
        let text: String = (0..500).map(|i| format!("{i} 行目の文です。\n")).collect();
        let cut = truncate_lines(&text, 1_000);
        assert!(cut.chars().count() <= 1_000, "{}", cut.chars().count());
        assert!(cut.ends_with("確認できます)\n"), "{cut}");
        assert_eq!(truncate_lines("短い。\n", 1_000), "短い。\n");

        let long = format!("{}\n次の行\n", "長".repeat(5_000));
        let cut = truncate_lines(&long, 1_000);
        assert!(cut.starts_with("長長"), "1 行目が長くても先頭は残す: {cut}");
        assert!(cut.chars().count() <= 1_000);
    }
}
