//! 連携機能 (`check --format brief`・`mcp`・`hook claude-code`) の統合テスト。
//!
//! 組み込みルールの増減に左右されないよう、件数を確かめるテストは設定ファイルの
//! 独自ルール (`X01`) と `--only-rules` を使う。

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Value, json};
use tempfile::TempDir;

const CONFIG: &str = r#"
[[custom]]
id = "X01"
name = "TEAM_TERM"
pattern = "ユーザー様"
message = "「ユーザー様」ではなく「利用者」と書きます"
hint = "用語集の表記に合わせてください"
severity = "warning"
"#;

const DOC_WITH_TERM: &str = "# 利用案内\n\n新しい機能はユーザー様の声から生まれました。\n\nユーザー様に感謝します。ユーザー様の意見を待っています。\n";
const DOC_CLEAN: &str = "# メモ\n\n今日は晴れた。散歩に出かけた。\n";

fn noslop() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noslop"));
    cmd.env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("CLAUDE_PROJECT_DIR");
    cmd
}

fn workspace() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
    fs::create_dir_all(dir.path().join("docs")).unwrap();
    fs::write(dir.path().join("docs/guide.md"), DOC_WITH_TERM).unwrap();
    fs::write(dir.path().join("docs/memo.md"), DOC_CLEAN).unwrap();
    dir
}

#[test]
fn brief_lists_rules_occurrences_and_clean_files() {
    let dir = workspace();
    let out = noslop()
        .current_dir(dir.path())
        .args([
            "check",
            "--format",
            "brief",
            "--only-rules",
            "X01",
            "--brief-limit",
            "2",
            "docs",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.starts_with("# noslop の改稿指示\n"), "{s}");
    assert!(s.contains("## 改稿のルール"), "{s}");
    assert!(s.contains("noslop-disable-next-line <ID> -- 理由"), "{s}");
    assert!(s.contains("## docs/guide.md"), "{s}");
    assert!(s.contains("#### 1. X01 TEAM_TERM"), "{s}");
    assert!(s.contains("(独自ルール・警告 3 件)"), "{s}");
    assert!(
        s.contains("- 直し方の方向: 用語集の表記に合わせてください"),
        "{s}"
    );
    assert!(
        s.contains("  - L3: 「ユーザー様」ではなく「利用者」と書きます"),
        "{s}"
    );
    assert!(
        s.contains("    - 原文: 新しい機能はユーザー様の声から生まれました。"),
        "{s}"
    );
    assert!(s.contains("  - ほか 1 件"), "{s}");
    assert!(s.contains("## 編集の問い"), "{s}");
    assert!(s.contains("## 指摘のないファイル\n\n- docs/memo.md"), "{s}");
}

#[test]
fn brief_without_findings_says_nothing_needs_fixing() {
    let dir = workspace();
    noslop()
        .current_dir(dir.path())
        .args([
            "check",
            "--format",
            "brief",
            "--only-rules",
            "X01",
            "docs/memo.md",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "指摘はありません。noslop の観点では直す必要はありません。",
        ))
        .stdout(predicate::str::contains("改稿のルール").not());
}

#[test]
fn brief_limit_must_be_positive() {
    noslop()
        .args(["check", "--format", "brief", "--brief-limit", "0", "-"])
        .write_stdin("本文。")
        .assert()
        .code(2)
        .stderr(predicate::str::contains("1 以上の整数"));
}

fn hook_input(dir: &TempDir, tool: &str, file: &str, tool_input: Value) -> String {
    let mut input = tool_input;
    input["file_path"] = json!(dir.path().join(file).to_string_lossy());
    json!({
        "session_id": "abc123",
        "cwd": dir.path().to_string_lossy(),
        "hook_event_name": "PostToolUse",
        "tool_name": tool,
        "tool_input": input,
        "tool_response": { "filePath": "x", "type": "update" },
        "tool_use_id": "toolu_01",
    })
    .to_string()
}

#[test]
fn claude_code_hook_passes_findings_as_additional_context() {
    let dir = workspace();
    let out = noslop()
        .current_dir(dir.path())
        .args(["hook", "claude-code", "--brief-limit", "1"])
        .write_stdin(hook_input(
            &dir,
            "Write",
            "docs/guide.md",
            json!({ "content": DOC_WITH_TERM }),
        ))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let hso = &v["hookSpecificOutput"];
    assert_eq!(hso["hookEventName"], "PostToolUse");
    assert!(v.get("decision").is_none(), "失敗に見える block は使わない");
    let ctx = hso["additionalContext"].as_str().unwrap();
    assert!(ctx.contains("docs/guide.md"), "{ctx}");
    assert!(
        ctx.contains("docs/guide.md に 独自ルールの指摘を 3 件見つけました"),
        "独自ルールは AI 臭さの疑いに混ぜずに数える: {ctx}"
    );
    assert!(
        ctx.contains("- X01 TEAM_TERM (独自ルール・警告 3 件)"),
        "{ctx}"
    );
    assert!(ctx.contains("  - ほか 2 件"), "{ctx}");
    assert!(ctx.contains("直さない判断もできます"), "{ctx}");
    assert!(ctx.contains("再実行は 1 回だけ"), "{ctx}");
}

#[test]
fn claude_code_hook_is_silent_for_other_tools_files_and_clean_documents() {
    let dir = workspace();
    fs::write(dir.path().join("main.rs"), "// ユーザー様\n").unwrap();
    for input in [
        hook_input(&dir, "Bash", "docs/guide.md", json!({ "command": "ls" })),
        hook_input(&dir, "Write", "main.rs", json!({})),
        hook_input(&dir, "Write", "docs/memo.md", json!({})),
    ] {
        noslop()
            .current_dir(dir.path())
            .args(["hook", "claude-code", "--genre", "tech"])
            .write_stdin(input)
            .assert()
            .code(0)
            .stdout(predicate::str::is_empty());
    }
}

#[test]
fn claude_code_hook_reports_bad_input_without_blocking() {
    noslop()
        .args(["hook", "claude-code"])
        .write_stdin("not json")
        .assert()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("JSON"));
}

/// 2026-07-28 版の、リクエストごとに版を名乗る `_meta`。
fn stateless_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "0" },
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

fn mcp_session(dir: &TempDir, lines: &[Value]) -> Vec<Value> {
    let input: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let out = noslop()
        .current_dir(dir.path())
        .arg("mcp")
        .write_stdin(input)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect()
}

#[test]
fn mcp_server_answers_over_stdio_with_the_discovered_config() {
    let dir = workspace();
    let responses = mcp_session(
        &dir,
        &[
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "test", "version": "0" }
                }
            }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "check",
                    "arguments": { "text": DOC_WITH_TERM, "filename": "guide.md" }
                }
            }),
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": { "name": "explain", "arguments": { "rule": "X01" } }
            }),
        ],
    );
    assert_eq!(responses.len(), 4);
    assert_eq!(responses[0]["result"]["protocolVersion"], "2025-06-18");
    let tools: Vec<&str> = responses[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(tools, ["check", "diff", "explain", "rules"]);
    let check = &responses[2]["result"];
    assert_eq!(check["isError"], false);
    let text = check["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("X01 TEAM_TERM"),
        "設定ファイルの独自ルールが効く: {text}"
    );
    let explain = responses[3]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(explain.starts_with("X01 TEAM_TERM"), "{explain}");
}

#[test]
fn mcp_server_speaks_the_stateless_protocol_too() {
    let dir = workspace();
    let meta = stateless_meta();
    let responses = mcp_session(
        &dir,
        &[
            json!({ "jsonrpc": "2.0", "id": "d", "method": "server/discover", "params": { "_meta": meta } }),
            json!({
                "jsonrpc": "2.0",
                "id": "c",
                "method": "tools/call",
                "params": {
                    "_meta": meta,
                    "name": "check",
                    "arguments": { "text": DOC_CLEAN, "report": "full", "format": "json" }
                }
            }),
        ],
    );
    assert_eq!(responses[0]["result"]["supportedVersions"][0], "2026-07-28");
    assert_eq!(responses[0]["result"]["resultType"], "complete");
    let result = &responses[1]["result"];
    assert_eq!(result["resultType"], "complete");
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "noslop"
    );
    let report: Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(report["schemaVersion"], 2);
    let file = &report["files"][0];
    assert_eq!(file["path"], "<text>");
    // 文書全体の点数は出さず、未抑制の指摘をレーンごと・重大度ごとに数える
    assert!(file.get("score").is_none(), "{file}");
    assert_eq!(
        file["counts"]["custom"],
        json!({ "error": 0, "warning": 0, "info": 0 })
    );
}

#[test]
fn mcp_server_finds_the_config_from_the_claude_code_project_dir() {
    let project = workspace();
    let elsewhere = tempfile::tempdir().unwrap();
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "_meta": stateless_meta(),
            "name": "explain",
            "arguments": { "rule": "X01" }
        }
    });
    let out = noslop()
        .current_dir(elsewhere.path())
        .env("CLAUDE_PROJECT_DIR", project.path())
        .arg("mcp")
        .write_stdin(format!("{line}\n"))
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["result"]["isError"], false, "{v}");
}

#[test]
fn mcp_server_rejects_unversioned_requests_before_initialize() {
    let dir = workspace();
    let responses = mcp_session(
        &dir,
        &[json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })],
    );
    assert_eq!(responses[0]["error"]["code"], -32602);
}

#[test]
fn mcp_server_reports_config_errors_as_tool_errors() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("noslop.toml"), "genre = \"poem\"\n").unwrap();
    let responses = mcp_session(
        &dir,
        &[json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "_meta": stateless_meta(),
                "name": "check",
                "arguments": { "text": "本文。" }
            }
        })],
    );
    assert_eq!(responses[0]["result"]["isError"], true);
    let text = responses[0]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("設定"), "{text}");
}
