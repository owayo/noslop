//! `noslop hook command`: claw-hooks のコマンドフック (`[[command_hooks]]`) の判定器。
//!
//! claw-hooks はシェルコマンドを解析し、一致したプログラム (gws) の呼び出し 1 つにつき 1 回この判定器を
//! 起動して、呼び出しの引数 (クォートを外したもの) を標準入力の JSON (プロトコルの版 1) で渡す。
//! `sudo`・`env`・`bash -c '...'`・`xargs` の中の呼び出しも claw-hooks が見つける。コマンド行そのものは
//! 渡らない。検査と結論は Claude Code の PreToolUse と同じ ([`super::gws`])。
//!
//! - 止めるときは、理由を標準エラーに書いて終了コード 2 (claw-hooks がコマンドを拒否する)
//! - 止めずに知らせるときは、標準出力に書いて 0 (claw-hooks がエージェントへの補足にする)。
//!   `context_delivery` が偽 (補足が届かない) か、イベントが `PermissionRequest` なら知らせない
//! - claw-hooks が呼び出しを確定できない (`analysis` が `uncertain`) ときは止めず、知らせるだけにする
//! - 入力の誤りと設定の誤りは、標準エラーに書いて 1 (claw-hooks の `on_error` に従う)
//!
//! 文の頭に名乗りは付けない (claw-hooks が `[noslop]` を付ける)。

use std::io::{self, Write as _};
use std::path::Path;
use std::time::SystemTime;

use serde::Deserialize;
use serde_json::Value;

use super::gws::{Request, Style, Verdict, review_writes};
use super::read_input;
use crate::cli::{self, CommandHookArgs};
use crate::gws::{self, Arg, Write};

/// 読めるプロトコルの版。
const PROTOCOL_VERSION: u64 = 1;

/// `noslop hook command` の本体。終了コードを返す。
///
/// 止めるなら理由を標準エラーに書いて 2、知らせるなら標準出力に書いて 0、何もなければ何も書かずに 0。
/// 入力や設定の誤りは標準エラーに書いて 1。
pub fn command(args: &CommandHookArgs) -> u8 {
    let cwd = std::env::current_dir().ok();
    let result = read_input().and_then(|input| {
        judge(
            &input,
            args,
            cwd.as_deref(),
            &cli::Environment::from_process(),
            SystemTime::now(),
        )
    });
    match result {
        Ok(None) => 0,
        Ok(Some(Verdict::Deny(reason))) => {
            // 書けなくても止める (claw-hooks は理由が空なら決まった文で拒否する)
            let mut err = io::stderr().lock();
            let _ = err.write_all(reason.as_bytes()).and_then(|()| err.flush());
            2
        }
        Ok(Some(Verdict::Report(text))) => {
            let mut out = io::stdout().lock();
            match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("noslop: 出力に失敗しました: {e}");
                    1
                }
            }
        }
        Err(message) => {
            eprintln!("noslop: {message}");
            1
        }
    }
}

/// claw-hooks の判定器の入力 (プロトコルの版 1)。使わないフィールド (`agent`・`tool_name`・`stdin`) は
/// 読み飛ばす。gws には値を標準入力から読むオプションがないので、`stdin` は見ない。
#[derive(Debug, Deserialize)]
struct Input {
    event: Event,
    session_id: Option<String>,
    cwd: Option<String>,
    analysis: Analysis,
    context_delivery: bool,
    argv: Vec<Word>,
}

/// フックのイベント (claw-hooks がそろえた名前)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum Event {
    /// コマンドの実行前 (各エージェントの実行前のイベントをそろえたもの)。
    PreToolUse,
    /// Codex CLI の承認の要求。補足は届かない。
    PermissionRequest,
}

/// 呼び出しが、コマンドの構文から確定したものか。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Analysis {
    Complete,
    /// 候補 (静的でない文字列を解析し直したもの、構文の誤りを含むコマンドから見つけたものなど)。
    Uncertain,
}

/// 呼び出しの語 1 つ。
#[derive(Debug, Deserialize)]
struct Word {
    /// クォートを外した値。実行しないと決まらなければ `None`。
    value: Option<String>,
    #[serde(rename = "static")]
    is_static: bool,
    cardinality: Cardinality,
}

/// 語が実行時にいくつの引数になるか。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Cardinality {
    /// ちょうど 1 つ。
    One,
    /// 0 個を含む任意の個数 (クォートしていない展開・グロブ・`xargs` が付け足す引数など)。
    ZeroOrMore,
}

impl Word {
    /// gws の引数としての値。
    fn arg(&self) -> Result<Arg, String> {
        if self.is_static != self.value.is_some() {
            return Err(
                "claw-hooks の入力の argv で、value と static が食い違っています".to_string(),
            );
        }
        Ok(match (&self.value, self.cardinality) {
            (Some(value), Cardinality::One) => Arg::Static(value.clone()),
            (None, Cardinality::One) => Arg::Unknown,
            (_, Cardinality::ZeroOrMore) => Arg::Dynamic,
        })
    }
}

/// 入力を読む。版を確かめてから、フィールドを読む (版が違えば、フィールドの意味も違うかもしれない)。
fn parse_input(input: &str) -> Result<Input, String> {
    let value: Value = serde_json::from_str(input)
        .map_err(|e| format!("claw-hooks の入力を JSON として読めません: {e}"))?;
    match value.get("version").and_then(Value::as_u64) {
        Some(PROTOCOL_VERSION) => {}
        Some(v) => {
            return Err(format!(
                "claw-hooks の入力の版 {v} には対応していません (対応しているのは {PROTOCOL_VERSION})"
            ));
        }
        None => return Err("claw-hooks の入力に版 (version) がありません".to_string()),
    }
    serde_json::from_value(value).map_err(|e| format!("claw-hooks の入力を読めません: {e}"))
}

/// 呼び出しの語を gws の引数にする (先頭のプログラム名を除く)。
fn gws_args(argv: &[Word]) -> Result<Vec<Arg>, String> {
    let Some((_, rest)) = argv.split_first() else {
        return Err("claw-hooks の入力の argv が空です".to_string());
    };
    rest.iter().map(Word::arg).collect()
}

/// 入力に対する結論。何もしないなら `None`。`cwd` は判定器の作業ディレクトリ (入力に cwd がないときに、
/// 設定ファイルを探し始める場所)。
fn judge(
    input: &str,
    args: &CommandHookArgs,
    cwd: Option<&Path>,
    env: &cli::Environment,
    now: SystemTime,
) -> Result<Option<Verdict>, String> {
    let input = parse_input(input)?;
    let writes: Vec<Write> = gws::write(&gws_args(&input.argv)?).into_iter().collect();
    let request = Request {
        writes,
        session: input.session_id.as_deref(),
        cwd: input.cwd.as_deref().map(Path::new).or(cwd),
        may_deny: input.analysis == Analysis::Complete,
        reports: input.context_delivery && input.event == Event::PreToolUse,
    };
    let style = Style {
        prefix: "",
        budget: args.max_chars,
    };
    review_writes(request, &args.hook, env, now, &style)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, UNIX_EPOCH};

    use serde_json::json;

    use super::super::gws::{CONTEXT_HEAD, DENY_HEAD};
    use super::super::testing::{CONFIG, parse};
    use super::*;

    /// 設定を置いた作業ディレクトリと、キャッシュの置き場所。
    struct Workspace {
        dir: tempfile::TempDir,
        cache: tempfile::TempDir,
    }

    impl Workspace {
        fn new() -> Self {
            Self::with_config(CONFIG)
        }

        fn with_config(config: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("noslop.toml"), config).unwrap();
            Self {
                dir,
                cache: tempfile::tempdir().unwrap(),
            }
        }

        fn env(&self) -> cli::Environment {
            cli::Environment {
                cache_dir: Some(self.cache.path().to_path_buf()),
                ..Default::default()
            }
        }

        /// PreToolUse の入力 (呼び出しは確定、補足は届く)。
        fn input(&self, argv: Vec<Value>) -> Value {
            json!({
                "version": 1,
                "agent": "claude-code",
                "event": "PreToolUse",
                "tool_name": "Bash",
                "session_id": "s1",
                "cwd": self.dir.path().to_string_lossy(),
                "analysis": "complete",
                "context_delivery": true,
                "argv": argv,
                "stdin": null,
            })
        }

        fn judge_with(
            &self,
            input: &Value,
            extra: &[&str],
            now: SystemTime,
        ) -> Result<Option<Verdict>, String> {
            judge(&input.to_string(), &args(extra), None, &self.env(), now)
        }

        fn judge(&self, input: &Value) -> Option<Verdict> {
            self.judge_with(input, &[], t0()).unwrap()
        }

        fn records(&self) -> usize {
            fs::read_dir(self.cache.path().join("hook-state")).map_or(0, |entries| entries.count())
        }
    }

    fn args(extra: &[&str]) -> CommandHookArgs {
        let mut argv = vec!["command"];
        argv.extend_from_slice(extra);
        match parse(&argv) {
            cli::HookCommand::Command(a) => a,
            _ => panic!("hook command"),
        }
    }

    fn t0() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_800_000_000)
    }

    /// 値の決まる語。
    fn s(value: &str) -> Value {
        json!({"value": value, "static": true, "cardinality": "one"})
    }

    /// 1 語だが値の決まらない語 (`"$DOC"`)。
    fn unknown() -> Value {
        json!({"value": null, "static": false, "cardinality": "one"})
    }

    /// 消えることも分かれることもある語 (`$EXTRA`)。
    fn many() -> Value {
        json!({"value": null, "static": false, "cardinality": "zero_or_more"})
    }

    /// `gws docs +write --document <doc> --text <text>` の語。
    fn docs_write(doc: &str, text: &str) -> Vec<Value> {
        ["gws", "docs", "+write", "--document", doc, "--text", text]
            .map(s)
            .to_vec()
    }

    /// `gws sheets +append --spreadsheet S --values <values>` の語 (短い値だけの書き込み)。
    fn sheets_append(values: &str) -> Vec<Value> {
        [
            "gws",
            "sheets",
            "+append",
            "--spreadsheet",
            "S",
            "--values",
            values,
        ]
        .map(s)
        .to_vec()
    }

    fn deny(verdict: Option<Verdict>) -> String {
        match verdict {
            Some(Verdict::Deny(reason)) => reason,
            other => panic!("止めるはず: {other:?}"),
        }
    }

    fn report(verdict: Option<Verdict>) -> String {
        match verdict {
            Some(Verdict::Report(text)) => text,
            other => panic!("知らせるだけのはず: {other:?}"),
        }
    }

    const TEXT: &str = "ユーザー様の声を集めました。";

    #[test]
    fn prose_warnings_are_denied_with_the_reason_and_retries_pass() {
        let ws = Workspace::new();
        let input = ws.input(docs_write("D1", TEXT));
        let reason = deny(ws.judge(&input));
        // 名乗りは claw-hooks が付ける
        assert!(reason.starts_with(DENY_HEAD), "{reason}");
        assert!(
            reason.contains("noslop が gws docs +write (--document D1) の --text に"),
            "{reason}"
        );
        assert!(
            reason.contains("L1: 「ユーザー様」ではなく「利用者」と書きます"),
            "{reason}"
        );
        // 同じ書き込みは、止めてから 30 分のあいだ通す
        assert_eq!(ws.judge(&input), None);
        assert_eq!(ws.judge(&input), None);
        assert_eq!(ws.records(), 1);
    }

    /// claw-hooks は呼び出しごとに判定器を走らせ、最初に止めたところで打ち切る。止める書き込みが 2 つ
    /// あるコマンドも、1 つずつ止めて示したあとは通る。
    #[test]
    fn two_writes_in_one_command_pass_once_each_has_been_shown() {
        let ws = Workspace::new();
        let a = ws.input(docs_write("D1", TEXT));
        let b = ws.input(docs_write("D2", "ユーザー様に届けます。"));
        // 1 回目: a で止まり、b の判定器は走らない
        deny(ws.judge(&a));
        // 2 回目: a は通り、b で止まる
        assert_eq!(ws.judge(&a), None);
        deny(ws.judge(&b));
        // 3 回目: どちらも通る
        assert_eq!(ws.judge(&a), None);
        assert_eq!(ws.judge(&b), None);
    }

    #[test]
    fn uncertain_or_inexact_calls_are_only_reported() {
        let ws = Workspace::new();
        let mut uncertain = ws.input(docs_write("D1", TEXT));
        uncertain["analysis"] = json!("uncertain");
        let mut extra = docs_write("D1", TEXT);
        extra.push(many());
        let mut flag = docs_write("D1", TEXT);
        flag.push(unknown());
        for input in [uncertain, ws.input(extra), ws.input(flag)] {
            let text = report(ws.judge(&input));
            assert!(text.starts_with(CONTEXT_HEAD), "{text}");
            assert!(text.contains("X01"), "{text}");
        }
        assert_eq!(ws.records(), 0);

        // フラグの値の位置の 1 語 (書き込み先) だけなら止める
        let mut argv = docs_write("D1", TEXT);
        argv[4] = unknown();
        deny(ws.judge(&ws.input(argv)));
    }

    #[test]
    fn without_context_delivery_only_denials_are_returned() {
        let ws = Workspace::new();
        let quiet = |argv: Vec<Value>| {
            let mut input = ws.input(argv);
            input["context_delivery"] = json!(false);
            input
        };
        assert_eq!(ws.judge(&quiet(sheets_append("ユーザー様の区分"))), None);
        deny(ws.judge(&quiet(docs_write("D1", TEXT))));

        // 止められないと分かれば、設定も読まずに返す (壊れた設定でも誤りにならない)
        let broken = Workspace::with_config("これは TOML ではない [");
        let mut input = broken.input(docs_write("D1", TEXT));
        input["context_delivery"] = json!(false);
        input["analysis"] = json!("uncertain");
        assert_eq!(broken.judge_with(&input, &[], t0()), Ok(None));
        input["analysis"] = json!("complete");
        input["session_id"] = json!(null);
        assert_eq!(broken.judge_with(&input, &[], t0()), Ok(None));
        input["session_id"] = json!("s1");
        assert!(broken.judge_with(&input, &[], t0()).is_err());
    }

    #[test]
    fn permission_requests_deny_but_never_report() {
        let ws = Workspace::new();
        let request = |argv: Vec<Value>| {
            let mut input = ws.input(argv);
            input["event"] = json!("PermissionRequest");
            input
        };
        assert_eq!(ws.judge(&request(sheets_append("ユーザー様の区分"))), None);
        deny(ws.judge(&request(docs_write("D1", TEXT))));
        // 同じ書き込みの記録は、イベントによらず使う
        assert_eq!(ws.judge(&ws.input(docs_write("D1", TEXT))), None);
    }

    #[test]
    fn without_a_session_the_findings_are_only_reported() {
        let ws = Workspace::new();
        let mut input = ws.input(docs_write("D1", TEXT));
        input["session_id"] = json!(null);
        let text = report(ws.judge(&input));
        assert!(text.starts_with(CONTEXT_HEAD), "{text}");
        assert_eq!(ws.records(), 0);
    }

    #[test]
    fn short_values_reads_and_other_programs_are_not_denied() {
        let ws = Workspace::new();
        let text = report(ws.judge(&ws.input(sheets_append("ユーザー様の区分"))));
        assert!(
            text.contains("gws sheets +append (--spreadsheet S) の --values"),
            "{text}"
        );
        for argv in [
            ["gws", "docs", "documents", "get", "--params", "{}"]
                .map(s)
                .to_vec(),
            vec![s("gws")],
            docs_write("D1", "今日は晴れた。散歩に出かけた。"),
            ["echo", "ユーザー様"].map(s).to_vec(),
        ] {
            assert_eq!(ws.judge(&ws.input(argv.clone())), None, "{argv:?}");
        }
        assert_eq!(ws.records(), 0);
    }

    #[test]
    fn the_config_is_found_from_the_input_cwd_or_the_process_cwd() {
        let ws = Workspace::new();
        let mut input = ws.input(docs_write("D1", TEXT));
        input["cwd"] = json!(null);
        // 入力に cwd がなければ、判定器の作業ディレクトリから探す
        let verdict = judge(
            &input.to_string(),
            &args(&[]),
            Some(ws.dir.path()),
            &ws.env(),
            t0(),
        )
        .unwrap();
        deny(verdict);
    }

    #[test]
    fn the_output_fits_max_chars() {
        let ws = Workspace::new();
        let text: String = (0..200)
            .map(|i| format!("{i} 番目のユーザー様です。\n"))
            .collect();
        let input = ws.input(docs_write("D1", &text));
        let reason = deny(
            ws.judge_with(&input, &["--max-chars", "400", "--brief-limit", "50"], t0())
                .unwrap(),
        );
        assert!(reason.chars().count() <= 400, "{}", reason.chars().count());
        assert!(reason.starts_with(DENY_HEAD), "頭は残す: {reason}");
    }

    #[test]
    fn argv_words_become_gws_arguments() {
        let words: Vec<Word> = serde_json::from_value(json!([
            {"value": "gws", "static": true, "cardinality": "one"},
            {"value": "docs", "static": true, "cardinality": "one"},
            {"value": null, "static": false, "cardinality": "one"},
            {"value": null, "static": false, "cardinality": "zero_or_more"},
            {"value": "*.md", "static": true, "cardinality": "zero_or_more"},
        ]))
        .unwrap();
        assert_eq!(
            gws_args(&words).unwrap(),
            vec![
                Arg::Static("docs".to_string()),
                Arg::Unknown,
                Arg::Dynamic,
                Arg::Dynamic,
            ]
        );
    }

    #[test]
    fn bad_input_is_an_error() {
        let ws = Workspace::new();
        let good = ws.input(docs_write("D1", TEXT));
        let mut cases: Vec<(String, &str)> = vec![("これは JSON ではない".to_string(), "JSON")];
        let mut edit = |f: &dyn Fn(&mut Value), expected: &'static str| {
            let mut input = good.clone();
            f(&mut input);
            cases.push((input.to_string(), expected));
        };
        edit(&|v| v["version"] = json!(2), "版 2");
        edit(
            &|v| {
                v.as_object_mut().unwrap().remove("version");
            },
            "版 (version)",
        );
        edit(&|v| v["analysis"] = json!("maybe"), "読めません");
        edit(&|v| v["event"] = json!("PostToolUse"), "読めません");
        edit(
            &|v| {
                v.as_object_mut().unwrap().remove("context_delivery");
            },
            "読めません",
        );
        edit(&|v| v["argv"] = json!([]), "argv が空");
        edit(
            &|v| v["argv"][1] = json!({"value": "docs", "static": false, "cardinality": "one"}),
            "食い違って",
        );
        edit(
            &|v| v["argv"][1] = json!({"value": null, "static": true, "cardinality": "one"}),
            "食い違って",
        );
        for (input, expected) in cases {
            let err = judge(&input, &args(&[]), None, &ws.env(), t0()).unwrap_err();
            assert!(err.contains(expected), "{input}: {err}");
        }
    }
}
