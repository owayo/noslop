//! `noslop hook claude-code`: Claude Code のフックの入力 (JSON) をイベントごとに振り分ける。
//!
//! - PostToolUse (Write / Edit / MultiEdit): 書き換えたファイルを検査し、指摘があれば短い改稿指示を
//!   `hookSpecificOutput.additionalContext` で Claude に渡す。additionalContext はツールの結果の横に
//!   システムリマインダーとして入り、ツールの失敗としては扱われない (`decision: "block"` や終了コード 2 は
//!   失敗に見えるので使わない)。返すのは今回のツール呼び出しで変わった行に重なる指摘だけで、変わった行は
//!   ツールの結果にある差分 (`tool_response.structuredPatch`) から求め、なければ Edit / MultiEdit の
//!   `new_string` の位置から求める。位置を 1 つに決められないときは、ファイル全体の指摘を返す。
//! - PreToolUse (Bash): gws で Google ドキュメント・スプレッドシートに書き込む値を検査する ([`super::gws`])。
//! - Stop: リポジトリの差分を検査する ([`super::stop`])。
//!
//! `hook_event_name` がない入力は PostToolUse とみなす。ほかのイベントは何もしない。

use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{CONTEXT_BUDGET_CHARS, KEEP_NOTE, read_input, review, truncate_lines};
use crate::cli::{self, HookArgs};
use crate::diagnostic::Span;
use crate::document::Document;

/// PostToolUse で検査するツール。
const TOOLS: [&str; 3] = ["Write", "Edit", "MultiEdit"];

/// `noslop hook claude-code` の本体。終了コードを返す。
///
/// 入力の誤りは標準エラーに書いて 1 で終わる (Claude Code では処理を止めないエラーになる)。
pub fn claude_code(args: &HookArgs) -> u8 {
    let input = match read_input() {
        Ok(input) => input,
        Err(message) => {
            eprintln!("noslop: {message}");
            return 1;
        }
    };
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
pub fn respond(input: &str, args: &HookArgs) -> Result<Option<String>, String> {
    respond_in(input, args, &cli::Environment::from_process())
}

/// [`respond`] の本体 (手元の環境を受け取る。テストでは空の環境を渡す)。
fn respond_in(
    input: &str,
    args: &HookArgs,
    env: &cli::Environment,
) -> Result<Option<String>, String> {
    let event: Value = serde_json::from_str(input)
        .map_err(|e| format!("フックの入力を JSON として読めません: {e}"))?;
    match event.get("hook_event_name").and_then(Value::as_str) {
        None | Some("PostToolUse") => post_tool_use(&event, args, env),
        Some("PreToolUse") => super::gws::pre_tool_use(&event, args, env),
        Some("Stop") => super::stop::stop(&event, args, env),
        Some(_) => Ok(None),
    }
}

/// PostToolUse: 書き換えたファイルの、変わった行に重なる指摘を返す。
fn post_tool_use(
    event: &Value,
    args: &HookArgs,
    env: &cli::Environment,
) -> Result<Option<String>, String> {
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

    let changed = |doc: &Document| {
        if args.whole_file {
            None
        } else {
            changed_regions(tool, event, doc)
        }
    };
    let Some(review) = review(&path, cwd.as_deref(), args, env, changed)? else {
        return Ok(None);
    };
    let mut context = review.brief;
    context.push_str(KEEP_NOTE);
    if review.limited {
        context.push_str(&format!(
            "今回変わった行に重なる指摘だけを返しています。ファイル全体は `noslop check --format brief {}` で確認できます。\n",
            review.name
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

#[cfg(test)]
mod tests {
    use super::super::testing::{CONFIG, DOC, parse, workspace};
    use super::*;

    fn args(extra: &[&str]) -> HookArgs {
        let mut argv = vec!["claude-code"];
        argv.extend_from_slice(extra);
        match parse(&argv) {
            cli::HookCommand::ClaudeCode(a) => a,
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

    /// 手元の設定 (~/.config/noslop) と辞書に左右されないよう、空の環境で答える。
    fn run(input: &str, extra: &[&str]) -> Option<String> {
        respond_in(input, &args(extra), &cli::Environment::default())
            .unwrap()
            .map(|o| context(&o))
    }

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
        assert_eq!(
            run(&ev("Write", "main.rs"), &[]),
            None,
            "[code] extensions にないコードは見ない"
        );
        assert_eq!(run(&ev("Write", "missing.md"), &[]), None);
        // PreToolUse の Write と、知らないイベントは何もしない
        let pre = ev("Write", "guide.md").replace("PostToolUse", "PreToolUse");
        assert_eq!(run(&pre, &[]), None);
        let other = ev("Write", "guide.md").replace("PostToolUse", "SessionStart");
        assert_eq!(run(&other, &[]), None);
        assert!(respond_in("not json", &args(&[]), &cli::Environment::default()).is_err());
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

        // .noslopignore で除外したファイルも見ない
        std::fs::write(dir.path().join(".noslopignore"), "page.mdx\n").unwrap();
        assert_eq!(run(&ev("page.mdx"), &[]), None);
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
}
