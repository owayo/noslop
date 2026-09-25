//! `hook claude-code` の PreToolUse (Bash): gws で Google ドキュメント・スプレッドシートに書き込む値の
//! 検査の統合テスト。
//!
//! 組み込みルールの増減に左右されないよう、設定ファイルの独自ルール (`X01`) で指摘を作る。止めた記録は
//! `XDG_CACHE_HOME` に渡した一時ディレクトリに置く。

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
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

/// 本文に指摘のある書き込み。
const WRITE: &str = "gws docs +write --document DOC1 --text \"$(cat <<'EOF'\n新しい機能は、ユーザー様の声から生まれました。\nEOF\n)\"";

/// 設定を置いた作業ディレクトリと、手元の設定・辞書・キャッシュから切り離す空のディレクトリ。
struct Workspace {
    dir: TempDir,
    home: TempDir,
    cache: TempDir,
}

impl Workspace {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
        Self {
            dir,
            home: tempfile::tempdir().unwrap(),
            cache: tempfile::tempdir().unwrap(),
        }
    }

    /// 手元の設定・辞書・キャッシュから切り離した noslop (tests/cli.rs の `noslop()` と同じ切り離しに、
    /// キャッシュの置き場所を足したもの)。
    fn noslop(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_noslop"));
        let home = self.home.path();
        cmd.env_remove("NO_COLOR")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("CLAUDE_PROJECT_DIR")
            .env_remove("HASAMI_DICT")
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("HASAMI_DATA_DIR", home)
            .env("LOCALAPPDATA", home)
            .env("XDG_CACHE_HOME", self.cache.path());
        cmd
    }

    fn event(&self, session: Option<&str>, command: &str) -> String {
        let mut event = json!({
            "cwd": self.dir.path().to_string_lossy(),
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_input": { "command": command, "description": "書き込む" }
        });
        if let Some(session) = session {
            event["session_id"] = json!(session);
        }
        event.to_string()
    }

    /// フックを動かし、標準出力 (空なら `None`) を返す。終了コードは 0 であること。
    fn run(&self, session: Option<&str>, command: &str) -> Option<Value> {
        let out = self
            .noslop()
            .args(["hook", "claude-code"])
            .write_stdin(self.event(session, command))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).unwrap();
        (!stdout.trim().is_empty()).then(|| serde_json::from_str(&stdout).unwrap())
    }

    fn state_dir(&self) -> PathBuf {
        self.cache.path().join("noslop").join("hook-state")
    }

    fn records(&self) -> Vec<PathBuf> {
        fs::read_dir(self.state_dir())
            .map(|entries| entries.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
    }
}

fn hook_output(v: &Value) -> &Value {
    let o = &v["hookSpecificOutput"];
    assert_eq!(o["hookEventName"], "PreToolUse", "{v}");
    o
}

fn file_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

#[test]
fn a_write_with_findings_is_denied_and_the_identical_retry_passes_once() {
    let ws = Workspace::new();
    let first = ws.run(Some("session-1"), WRITE).expect("止める");
    let o = hook_output(&first);
    assert_eq!(o["permissionDecision"], "deny", "{first}");
    let reason = o["permissionDecisionReason"].as_str().unwrap();
    assert!(reason.contains("この書き込みを止めました"), "{reason}");
    assert!(
        reason.contains("同じコマンドをそのままもう一度実行してください"),
        "{reason}"
    );
    assert!(
        reason.contains("noslop が gws docs +write の --text に"),
        "{reason}"
    );
    assert!(
        reason.contains("L1: 「ユーザー様」ではなく「利用者」と書きます"),
        "{reason}"
    );

    // 記録はキャッシュの置き場所に、ハッシュの名前と時刻だけで置く (値もコマンドも書かない)
    let records = ws.records();
    assert_eq!(records.len(), 1, "{records:?}");
    let name = file_name(&records[0]);
    assert!(
        name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()),
        "{name}"
    );
    let content = fs::read_to_string(&records[0]).unwrap();
    assert!(
        !content.is_empty() && content.bytes().all(|b| b.is_ascii_digit()),
        "{content}"
    );

    // 同じコマンドをもう一度実行すると、何も出さずに通す (記録は消える)
    assert_eq!(ws.run(Some("session-1"), WRITE), None);
    assert!(ws.records().is_empty());
    // 次の同じ書き込みは、また検査する
    let third = ws.run(Some("session-1"), WRITE).expect("また止める");
    assert_eq!(hook_output(&third)["permissionDecision"], "deny");

    // 別のセッションの同じ書き込みは、記録を使わない
    let other = ws.run(Some("session-2"), WRITE).expect("止める");
    assert_eq!(hook_output(&other)["permissionDecision"], "deny");
}

#[test]
fn without_a_session_the_findings_arrive_as_additional_context() {
    let ws = Workspace::new();
    let out = ws.run(None, WRITE).expect("知らせる");
    let o = hook_output(&out);
    assert!(o.get("permissionDecision").is_none(), "{out}");
    let context = o["additionalContext"].as_str().unwrap();
    assert!(context.contains("書き込みは止めていません"), "{context}");
    assert!(context.contains("X01"), "{context}");
    assert!(ws.records().is_empty());
}

#[test]
fn cells_and_dry_runs_are_reported_without_denying() {
    let ws = Workspace::new();
    let cells =
        "gws sheets +append --spreadsheet SHEET1 --values '担当,ユーザー様の窓口,=SUM(A1:A2)'";
    let out = ws.run(Some("session-1"), cells).expect("知らせる");
    let o = hook_output(&out);
    assert!(o.get("permissionDecision").is_none(), "{out}");
    let context = o["additionalContext"].as_str().unwrap();
    assert!(
        context.contains("noslop が gws sheets +append の --values に"),
        "{context}"
    );
    assert!(
        context.contains("の行と値の対応: L2 = --values[1]"),
        "{context}"
    );
    assert!(
        context.contains("短い値での精度は確かめていない"),
        "{context}"
    );

    let dry = ws
        .run(Some("session-1"), &format!("{WRITE} --dry-run"))
        .expect("知らせる");
    let context = hook_output(&dry)["additionalContext"].as_str().unwrap();
    assert!(context.contains("--dry-run に渡した値"), "{context}");
    assert!(ws.records().is_empty());
}

#[test]
fn other_bash_commands_and_reads_produce_no_output() {
    let ws = Workspace::new();
    for command in [
        "ls -la",
        "git status",
        "gws docs documents get --params '{\"documentId\":\"DOC1\"}'",
        "gws docs +write --document DOC1 --text \"$BODY\"",
        "gws docs +write --document DOC1 --text '晴れた日に散歩した。'",
    ] {
        assert_eq!(ws.run(Some("session-1"), command), None, "{command}");
    }
    assert!(!ws.state_dir().exists());
}
