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

/// 手元の設定と辞書から切り離した noslop (tests/cli.rs の同名の関数と同じ)。
fn noslop() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noslop"));
    let empty = empty_dir();
    cmd.env_remove("NO_COLOR")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("CLAUDE_PROJECT_DIR")
        .env("HOME", empty)
        .env("USERPROFILE", empty)
        .env("HASAMI_DATA_DIR", empty)
        .env_remove("HASAMI_DICT");
    cmd
}

/// 中に何も置かないディレクトリ (テストの実行ごとに 1 つ)。
fn empty_dir() -> &'static std::path::Path {
    static EMPTY: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    EMPTY.get_or_init(|| tempfile::tempdir().unwrap()).path()
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

/// テスト用の git を動かす (手元の git の設定の署名やフックに左右されないよう切る)。
fn git(dir: &std::path::Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=noslop",
            "-c",
            "user.email=you@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// パスだけを渡すフック (`hook file`) は、git の HEAD との差分で変わった行に重なる指摘だけを
/// テキストで返す。追跡していないファイルや git の外ではファイル全体を見る。
#[test]
fn file_hook_limits_findings_to_uncommitted_lines() {
    let dir = workspace();
    let run = |extra: &[&str]| {
        let out = noslop()
            .current_dir(dir.path())
            .args(["hook", "file"])
            .args(extra)
            .arg("docs/guide.md")
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };

    // git の外ではファイル全体 (3 件)
    let text = run(&[]);
    assert!(
        text.contains("独自ルールの指摘を 3 件見つけました"),
        "{text}"
    );
    assert!(!text.contains("コミットしていない変更"), "{text}");

    if !git(dir.path(), &["init", "-q"]) {
        eprintln!("git を実行できないので、差分の確かめを飛ばします");
        return;
    }
    // 追跡していないファイルもファイル全体
    assert!(run(&[]).contains("3 件見つけました"));

    assert!(git(dir.path(), &["add", "-A"]));
    assert!(git(
        dir.path(),
        &["commit", "-q", "--no-verify", "-m", "init"]
    ));
    // コミットした直後は変わった行がないので、何も返さない
    assert_eq!(run(&[]), "");
    // --whole-file はファイル全体
    assert!(run(&["--whole-file"]).contains("3 件見つけました"));

    // 段落を 1 つ足すと、その行の指摘だけを返す
    let guide = dir.path().join("docs/guide.md");
    let mut doc = fs::read_to_string(&guide).unwrap();
    doc.push_str("\n足した段落でもユーザー様と書きました。\n");
    fs::write(&guide, doc).unwrap();
    let text = run(&[]);
    assert!(
        text.contains("独自ルールの指摘を 1 件見つけました"),
        "{text}"
    );
    assert!(text.contains("  - L7: "), "{text}");
    assert!(
        text.contains("コミットしていない変更 (git の HEAD との差分) の行に重なる指摘だけ"),
        "{text}"
    );
    assert!(
        serde_json::from_str::<Value>(&text).is_err(),
        "JSON ではなくテキスト"
    );
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

/// claw-hooks の判定器の入力 (gws の呼び出し 1 つ。語はどれも値が決まるもの)。
fn command_hook_input(dir: &TempDir, argv: &[&str]) -> String {
    let argv: Vec<Value> = argv
        .iter()
        .map(|v| json!({ "value": v, "static": true, "cardinality": "one" }))
        .collect();
    json!({
        "version": 1,
        "agent": "claude-code",
        "event": "PreToolUse",
        "tool_name": "Bash",
        "session_id": "abc123",
        "cwd": dir.path().to_string_lossy(),
        "analysis": "complete",
        "context_delivery": true,
        "argv": argv,
        "stdin": null,
    })
    .to_string()
}

#[test]
fn command_hook_denies_with_exit_code_2_and_reports_on_stdout() {
    let dir = workspace();
    // 止めた記録の置き場所 (手元のキャッシュに書かない)
    let cache = tempfile::tempdir().unwrap();
    let run = |argv: &[&str]| {
        noslop()
            .env("XDG_CACHE_HOME", cache.path())
            .current_dir(dir.path())
            .args(["hook", "command", "--max-chars", "900"])
            .write_stdin(command_hook_input(&dir, argv))
            .output()
            .unwrap()
    };
    let write = [
        "gws",
        "docs",
        "+write",
        "--document",
        "D1",
        "--text",
        "ユーザー様の声を集めました。",
    ];

    // 本文の警告: 理由を標準エラーに書いて 2 (名乗りは claw-hooks が付ける)
    let out = run(&write);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let reason = String::from_utf8(out.stderr).unwrap();
    assert!(
        reason.starts_with("gws で書き込む文章に指摘があるので、コマンドを止めました。"),
        "{reason}"
    );
    assert!(reason.contains("X01"), "{reason}");
    assert!(reason.chars().count() <= 900);

    // 同じ書き込みを実行し直すと、何も書かずに 0
    let out = run(&write);
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty() && out.stderr.is_empty());

    // 短い値の指摘は、止めずに標準出力で知らせる
    let out = run(&[
        "gws",
        "sheets",
        "+append",
        "--spreadsheet",
        "S1",
        "--values",
        "ユーザー様の区分",
    ]);
    assert_eq!(out.status.code(), Some(0));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.starts_with("gws で書き込んだ値に指摘があります。"),
        "{text}"
    );

    // 入力の誤りは 1 (claw-hooks の on_error に従う)
    noslop()
        .env("XDG_CACHE_HOME", cache.path())
        .args(["hook", "command"])
        .write_stdin("not json")
        .assert()
        .code(1)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("JSON"));
}

/// Claude Code と claw-hooks はフックの終了コード 2 を「止める」と読むので、フックの引数の誤りは 1 で
/// 終える。誤りを 2 で知らせる約束の `hook git-diff` と、ほかのサブコマンドは 2 のまま。
#[test]
fn hook_argument_errors_do_not_look_like_a_block() {
    for sub in ["command", "claude-code", "file"] {
        noslop()
            .args(["hook", sub, "--max-char", "900"])
            .write_stdin("{}")
            .assert()
            .code(1)
            .stderr(predicate::str::contains("--max-char"));
    }
    noslop().args(["hook", "claude_code"]).assert().code(1);
    noslop()
        .args(["hook", "git-diff", "--max-char", "900"])
        .assert()
        .code(2);
    noslop()
        .args(["check", "--max-char", "900"])
        .assert()
        .code(2);
    noslop()
        .args(["hook", "command", "--help"])
        .assert()
        .code(0);
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
