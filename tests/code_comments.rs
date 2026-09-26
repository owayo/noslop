//! コードのファイルのコメントを検査する統合テスト (直接の指定・`[code] extensions`・標準入力・MCP)。
//!
//! 組み込みルールの増減に左右されないよう、件数は設定ファイルの独自ルール (`X01`) と
//! `--only-rules` で確かめる。Bash の文法はどのビルドにも入るので、言語を問わない確かめは Bash で書く。

use std::fs;
use std::path::Path;

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
severity = "warning"
"#;

/// コメントに 1 か所、文字列に 1 か所「ユーザー様」がある Bash のスクリプト。
const SCRIPT: &str = "#!/bin/bash
# ユーザー様の設定を読み込む。
echo \"ユーザー様へのお知らせ\" # 文字列は読まない
";

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
fn empty_dir() -> &'static Path {
    static EMPTY: std::sync::OnceLock<TempDir> = std::sync::OnceLock::new();
    EMPTY.get_or_init(|| tempfile::tempdir().unwrap()).path()
}

/// 設定ファイル (`extra` を足したもの) と、Markdown の文書と Bash のスクリプトを置いたディレクトリ。
fn workspace(extra: &str) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("noslop.toml"), format!("{CONFIG}{extra}")).unwrap();
    fs::create_dir_all(dir.path().join("docs")).unwrap();
    fs::create_dir_all(dir.path().join("scripts")).unwrap();
    fs::write(
        dir.path().join("docs/guide.md"),
        "# 案内\n\nユーザー様の声です。\n",
    )
    .unwrap();
    fs::write(dir.path().join("scripts/run.sh"), SCRIPT).unwrap();
    dir
}

fn json(output: &[u8]) -> Value {
    serde_json::from_slice(output).unwrap_or_else(|e| {
        panic!(
            "JSON として読めません: {e}\n{}",
            String::from_utf8_lossy(output)
        )
    })
}

/// `noslop check --format json --only-rules X01 <args>` の結果。
fn check_json(dir: &Path, args: &[&str]) -> Value {
    let out = noslop()
        .current_dir(dir)
        .args(["check", "--format", "json", "--only-rules", "X01"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    json(&out.stdout)
}

/// ファイルの (パス, 形式, 指摘の行と列の並び)。
type FileSummary = (String, String, Vec<(u64, u64)>);

/// JSON で返ったファイルごとの要約。
fn files(report: &Value) -> Vec<FileSummary> {
    report["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let at = f["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| {
                    let start = &d["range"]["start"];
                    (
                        start["line"].as_u64().unwrap(),
                        start["column"].as_u64().unwrap(),
                    )
                })
                .collect();
            (
                f["path"].as_str().unwrap().to_string(),
                f["format"].as_str().unwrap().to_string(),
                at,
            )
        })
        .collect()
}

#[test]
fn directly_specified_code_files_are_read_as_comments() {
    let dir = workspace("");
    let report = check_json(dir.path(), &["scripts/run.sh"]);
    // コメントの 1 か所だけを指し、文字列は読まない。位置は原文の行と列
    assert_eq!(
        files(&report),
        [(
            "scripts/run.sh".to_string(),
            "code".to_string(),
            vec![(2, 3)]
        )]
    );
    let d = &report["files"][0]["diagnostics"][0];
    let offset = d["range"]["start"]["offset"].as_u64().unwrap() as usize;
    assert_eq!(&SCRIPT[offset..offset + "ユーザー様".len()], "ユーザー様");
    // 抜き出す文はコメントの記号を含まない
    assert_eq!(d["excerpt"], "ユーザー様の設定を読み込む。");
}

#[test]
fn directories_collect_code_only_with_code_extensions() {
    // [code] extensions がなければ、ディレクトリをたどってもコードは集めない
    let dir = workspace("");
    let report = check_json(dir.path(), &["."]);
    assert_eq!(
        files(&report),
        [(
            "docs/guide.md".to_string(),
            "markdown".to_string(),
            vec![(3, 1)]
        )]
    );
    let dir = workspace("\n[code]\nextensions = [\"sh\"]\n");
    let report = check_json(dir.path(), &["."]);
    assert_eq!(
        files(&report),
        [
            (
                "docs/guide.md".to_string(),
                "markdown".to_string(),
                vec![(3, 1)]
            ),
            (
                "scripts/run.sh".to_string(),
                "code".to_string(),
                vec![(2, 3)]
            ),
        ]
    );
}

#[test]
fn code_extensions_must_name_readable_languages() {
    let dir = workspace("\n[code]\nextensions = [\"md\"]\n");
    noslop()
        .current_dir(dir.path())
        .args(["check", "."])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "[code] extensions の `md` はコードの拡張子として読めません",
        ));
}

#[test]
fn stdin_filename_selects_the_language() {
    let dir = workspace("");
    let out = noslop()
        .current_dir(dir.path())
        .args([
            "check",
            "-",
            "--stdin-filename",
            "run.sh",
            "--format",
            "json",
            "--only-rules",
            "X01",
        ])
        .write_stdin(SCRIPT)
        .output()
        .unwrap();
    let report = json(&out.stdout);
    assert_eq!(
        files(&report),
        [("run.sh".to_string(), "code".to_string(), vec![(2, 3)])]
    );
}

#[test]
fn directives_in_code_comments_suppress_findings() {
    let dir = workspace("");
    let src = "# noslop-disable-next-line X01 -- 引用なので残す
# ユーザー様の設定を読み込む。
# 使い方は noslop-disable-next-line X01 のように書く。ユーザー様の設定は残す。
";
    fs::write(dir.path().join("scripts/quoted.sh"), src).unwrap();
    let report = check_json(dir.path(), &["scripts/quoted.sh"]);
    let diagnostics = report["files"][0]["diagnostics"].as_array().unwrap();
    let suppressed: Vec<(u64, bool)> = diagnostics
        .iter()
        .map(|d| {
            (
                d["range"]["start"]["line"].as_u64().unwrap(),
                !d["suppressed"].is_null(),
            )
        })
        .collect();
    // 文の途中に書いた記法は抑制にならない
    assert_eq!(suppressed, [(2, true), (3, false)]);
    assert_eq!(diagnostics[0]["suppressed"]["reason"], "引用なので残す");
}

#[test]
fn rust_sources_read_doc_comments_as_markdown() {
    let dir = workspace("");
    let src = "//! ユーザー様向けの説明。
/// 設定を読む (`ユーザー様` はコードなので読まない)。
///
/// ```
/// let s = \"ユーザー様\";
/// ```
fn load() {
    let s = \"// ユーザー様\";
    let x = 1; // ユーザー様の値
}
";
    fs::write(dir.path().join("lib.rs"), src).unwrap();
    let report = check_json(dir.path(), &["lib.rs"]);
    assert_eq!(
        files(&report),
        [(
            "lib.rs".to_string(),
            "code".to_string(),
            vec![(1, 5), (9, 19)]
        )]
    );
}

#[test]
fn mcp_check_reads_comments_for_a_code_filename() {
    let dir = workspace("");
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "0" },
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "_meta": meta,
            "name": "check",
            "arguments": { "text": SCRIPT, "filename": "run.sh", "report": "full", "format": "json" }
        }
    });
    let out = noslop()
        .current_dir(dir.path())
        .arg("mcp")
        .write_stdin(format!("{request}\n"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let response: Value = serde_json::from_slice(&out.stdout).unwrap();
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    let report: Value = serde_json::from_str(text).unwrap();
    let file = &report["files"][0];
    assert_eq!(file["path"], "run.sh");
    assert_eq!(file["format"], "code");
    let custom: Vec<u64> = file["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["ruleId"] == "X01")
        .map(|d| d["range"]["start"]["line"].as_u64().unwrap())
        .collect();
    assert_eq!(custom, [2]);
}

/// 文法はすべてバイナリに入っているので、どの言語も設定やビルドの指定なしで読める。
#[test]
fn every_language_is_read_without_build_options() {
    let dir = workspace("");
    fs::write(dir.path().join("a.cpp"), "// ユーザー様の設定です。\n").unwrap();
    fs::write(dir.path().join("b.swift"), "// ユーザー様の値です。\n").unwrap();
    fs::write(dir.path().join("c.kt"), "// ユーザー様の画面です。\n").unwrap();
    let out = noslop()
        .current_dir(dir.path())
        .args([
            "check",
            "--format",
            "json",
            "--only-rules",
            "X01",
            "a.cpp",
            "b.swift",
            "c.kt",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.is_empty(), "{stderr}");
    let report = json(&out.stdout);
    assert_eq!(report["summary"]["diagnostics"], 3);
    assert_eq!(report["files"][0]["format"], "code");
}
