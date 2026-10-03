//! コードの静的文言が CLI・変更行・フック・MCP・diff に同じ設定で届くことを確かめる。

use assert_cmd::Command;
use serde_json::{Value, json};
use std::{fs, path::Path};
use tempfile::TempDir;

const CONFIG: &str = r#"
[[custom]]
id = "X01"
name = "SAMPLE_TERM"
pattern = "仮の文言"
message = "表示する文を見直します"
severity = "warning"
"#;
const TSX: &str = "const message = \"仮の文言です。\";\nconst view = <div title=\"仮の文言です。\"><p>仮の<strong>文言</strong>です。</p><button>仮の文言</button><button>仮の文言</button></div>;\n";

fn noslop() -> Command {
    static EMPTY: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    let empty = EMPTY.get_or_init(|| tempfile::tempdir().unwrap()).path();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_noslop"));
    cmd.env("HOME", empty)
        .env("USERPROFILE", empty)
        .env("HASAMI_DATA_DIR", empty)
        .env_remove("HASAMI_DICT")
        .env_remove("CLAUDE_PROJECT_DIR")
        .env_remove("CLICOLOR_FORCE");
    cmd
}
fn workspace() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
    dir
}
fn check(dir: &Path, path: &str) -> Value {
    let out = noslop()
        .current_dir(dir)
        .args(["check", path, "--only-rules", "X01", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn diagnostics(report: &Value) -> &[Value] {
    report["files"][0]["diagnostics"].as_array().unwrap()
}

#[test]
fn cli_reports_original_code_positions_for_literals_attributes_and_inline_text() {
    let dir = workspace();
    fs::write(dir.path().join("sample.tsx"), TSX).unwrap();
    let report = check(dir.path(), "sample.tsx");
    let findings = diagnostics(&report);
    assert_eq!(findings.len(), 5);
    assert_eq!(report["files"][0]["format"], "code");
    for (i, d) in findings.iter().enumerate() {
        let start = d["range"]["start"]["offset"].as_u64().unwrap() as usize;
        let end = d["range"]["end"]["offset"].as_u64().unwrap() as usize;
        assert_eq!(
            &TSX[start..end],
            if i == 2 {
                "仮の<strong>文言"
            } else {
                "仮の文言"
            }
        );
    }
    let src = "\u{FEFF}const x = \"\\u4eeeの文言\";\r\n";
    fs::write(dir.path().join("escaped.ts"), src).unwrap();
    let report = check(dir.path(), "escaped.ts");
    let d = &diagnostics(&report)[0];
    let start = d["range"]["start"]["offset"].as_u64().unwrap() as usize;
    let end = d["range"]["end"]["offset"].as_u64().unwrap() as usize;
    assert_eq!(&src[start..end], "\\u4eeeの文言");
    assert_eq!(d["range"]["start"]["line"], 1);
    assert_eq!(d["range"]["start"]["column"], 12);
}

#[test]
fn static_text_setting_is_layered_and_no_config_restores_the_default() {
    let dir = workspace();
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join(".config/noslop")).unwrap();
    fs::write(
        home.path().join(".config/noslop/config.toml"),
        format!("{CONFIG}\n[code]\nstatic_text = false\n"),
    )
    .unwrap();
    fs::write(
        dir.path().join("noslop.toml"),
        "[code]\nextensions = [\"tsx\"]\n",
    )
    .unwrap();
    fs::write(dir.path().join("sample.tsx"), TSX).unwrap();
    let run = |args: &[&str]| {
        let out = noslop()
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .current_dir(dir.path())
            .args(["check", "sample.tsx", "--format", "json"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success());
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    assert!(diagnostics(&run(&["--only-rules", "X01"])).is_empty());
    fs::write(
        dir.path().join("noslop.toml"),
        "[code]\nstatic_text = true\n",
    )
    .unwrap();
    assert_eq!(diagnostics(&run(&["--only-rules", "X01"])).len(), 5);
    fs::write(
        home.path().join(".config/noslop/config.toml"),
        format!("{CONFIG}\n[code]\nstatic_text = true\n"),
    )
    .unwrap();
    fs::write(
        dir.path().join("noslop.toml"),
        "[code]\nstatic_text = false\n",
    )
    .unwrap();
    assert!(diagnostics(&run(&["--only-rules", "X01"])).is_empty());
    fs::write(
        dir.path().join("sample.tsx"),
        "const x = \"この説明で十分と言えるでしょう。\";",
    )
    .unwrap();
    let report = run(&["--no-config", "--only-rules", "P01"]);
    assert!(!diagnostics(&report).is_empty());
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=you@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn changed_lines_select_static_text_for_cli_and_both_file_hooks() {
    let dir = workspace();
    fs::write(
        dir.path().join("noslop.toml"),
        format!("{CONFIG}\n[code]\nextensions = [\"tsx\"]\n"),
    )
    .unwrap();
    let before =
        "const old = \"仮の文言は残します。\";\nconst edited = <p>読みやすい案内です。</p>;\n";
    let after = "const old = \"仮の文言は残します。\";\nconst edited = <p>仮の<strong>文言</strong>です。</p>;\n";
    fs::write(dir.path().join("sample.tsx"), before).unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "Initial fixture"]);
    fs::write(dir.path().join("sample.tsx"), after).unwrap();
    let out = noslop()
        .current_dir(dir.path())
        .args([
            "check",
            "--git-diff",
            "--only-rules",
            "X01",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(diagnostics(&report).len(), 1);
    assert_eq!(diagnostics(&report)[0]["range"]["start"]["line"], 2);
    let out = noslop()
        .current_dir(dir.path())
        .args(["hook", "file", "sample.tsx"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("X01") && !text.contains("残します"), "{text}");
    let input = json!({"hook_event_name": "PostToolUse", "tool_name": "Edit", "tool_input": {"file_path": dir.path().join("sample.tsx"), "old_string": before.lines().nth(1).unwrap(), "new_string": after.lines().nth(1).unwrap()}, "tool_response": {}});
    let out = noslop()
        .current_dir(dir.path())
        .args(["hook", "claude-code"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("X01") && !text.contains("残します"), "{text}");
}

fn mcp(dir: &Path, tool: &str, args: Value) -> Value {
    let request = json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"_meta":{
        "io.modelcontextprotocol/protocolVersion":"2026-07-28",
        "io.modelcontextprotocol/clientInfo":{"name":"test","version":"0"},
        "io.modelcontextprotocol/clientCapabilities":{}
    }, "name":tool, "arguments":args}});
    let out = noslop()
        .current_dir(dir)
        .arg("mcp")
        .write_stdin(format!("{request}\n"))
        .output()
        .unwrap();
    assert!(out.status.success());
    let response: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(response["error"].is_null(), "{response}");
    serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn mcp_and_diff_share_static_text_and_the_opt_out() {
    let dir = workspace();
    let report = mcp(
        dir.path(),
        "check",
        json!({"text":TSX, "filename":"sample.tsx", "report":"full", "format":"json"}),
    );
    assert_eq!(
        diagnostics(&report)
            .iter()
            .filter(|d| d["ruleId"] == "X01")
            .count(),
        5
    );
    let before = "const x = \"案内を表示します。\";";
    let after = "const x = \"仮の文言を表示します。\";";
    fs::write(dir.path().join("before.tsx"), before).unwrap();
    fs::write(dir.path().join("after.tsx"), after).unwrap();
    let run_diff = || {
        let out = noslop()
            .current_dir(dir.path())
            .args([
                "diff",
                "before.tsx",
                "after.tsx",
                "--only-rules",
                "X01",
                "--format",
                "json",
            ])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    let diff = run_diff();
    assert_eq!(
        diff["findings"]["new"].as_array().unwrap().len(),
        1,
        "{diff}"
    );
    fs::write(
        dir.path().join("noslop.toml"),
        format!("{CONFIG}\n[code]\nstatic_text = false\n"),
    )
    .unwrap();
    let report = mcp(
        dir.path(),
        "check",
        json!({"text":TSX, "filename":"sample.tsx", "report":"full", "format":"json"}),
    );
    assert!(diagnostics(&report).is_empty());
    assert!(run_diff()["findings"]["new"].as_array().unwrap().is_empty());
}
