//! gws (Google Workspace CLI) のコマンドから、Google ドキュメント・スプレッドシートに書き込む値を
//! 取り出す。
//!
//! Bash のコマンド行を tree-sitter-bash で読み ([`shell`])、gws の呼び出しごとに、サービス・リソース・
//! メソッドの表 ([`METHODS`]) から、書き込む値の場所 (フラグと、フラグに渡す JSON の中のパス) を引く。
//! 表にないコマンド (読み取りなど) は読まない。値は実行しなくても決まるものだけを読み、変数やほかの
//! コマンドの出力で決まる値は黙って飛ばす。コマンドを実行したり、コマンドが指すファイルを読んだりは
//! しない。

mod shell;

use std::fmt::Write as _;

use serde_json::Value as Json;

pub use shell::{Arg, gws_invocations};

/// 値の検査の仕方。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// ひと続きの文章 (ドキュメントの本文)。値 1 つを 1 つの文書にして、すべてのルールを当てる。
    Prose,
    /// 互いに独立した短い値 (セル・タイトル・置き換えの文字列)。1 回の呼び出しの値をまとめて断片の
    /// 集まりの文書にし、1 文ずつ判定するルールだけを当てる。
    Fragment,
}

/// 書き込む値 1 つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value {
    /// 値の場所 (`--text`・`requests[0].insertText.text`・`values[1][2]`・`--values[0]` など)。
    pub label: String,
    /// 値を取り出した表の項目 (`values[*][*]` など)。同じ項目の値をまとめて呼ぶのに使う。
    pub source: String,
    pub kind: ValueKind,
    /// スプレッドシートのセルか (`=` で始まる値は数式として読まれる)。
    pub cell: bool,
    pub text: String,
}

/// gws の書き込み 1 回 (コマンド行にある gws の呼び出し 1 つ)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Write {
    /// サービス・リソース・メソッド (`docs documents batchUpdate`・`sheets +append` など)。
    pub method: String,
    /// `--dry-run` (書き込まずに確かめるだけ) か。
    pub dry_run: bool,
    /// 書き込み先 (`--document`・`--spreadsheet` と、`--params` の `documentId`・`spreadsheetId`・
    /// `range`)。値が決まらないものは `None`。
    pub destination: Vec<(String, Option<String>)>,
    /// 書き込む値 (表の項目の順。項目の中は現れる順)。
    pub values: Vec<Value>,
}

/// コマンド行にある gws の書き込みのうち、書き込む値を 1 つ以上読めたもの (コマンド行に出てくる順)。
pub fn writes(command: &str) -> Vec<Write> {
    gws_invocations(command)
        .iter()
        .filter_map(|args| write(args))
        .collect()
}

/// 書き込む値の場所。
struct Source {
    /// 値を渡すフラグ。
    flag: &'static str,
    /// フラグの値からの取り出し方。
    shape: Shape,
    kind: ValueKind,
    /// スプレッドシートのセルか。
    cell: bool,
}

/// フラグの値からの取り出し方。
enum Shape {
    /// フラグの値そのもの。
    Whole,
    /// カンマで区切った値 (区切りの前後の空白も値に含める。gws の `+append --values` と同じ)。
    Commas,
    /// フラグの値の JSON の中の文字列。パスは `.` でつないだキーと、配列の全要素を表す `[*]` で書く。
    Json(&'static str),
}

/// 書き込みのコマンド (gws の位置引数に並ぶサービス・リソース・メソッド) と、書き込む値の場所。
struct Method {
    path: &'static [&'static str],
    sources: &'static [Source],
}

const fn source(flag: &'static str, shape: Shape, kind: ValueKind, cell: bool) -> Source {
    Source {
        flag,
        shape,
        kind,
        cell,
    }
}

/// `--json` の本文の中のセルの値。
const fn cells(path: &'static str) -> Source {
    source("--json", Shape::Json(path), ValueKind::Fragment, true)
}

/// 書き込む値を読む gws のコマンド (gws 0.22 で確かめた形)。サービスやメソッドを足すときは、ここに
/// 行を足す。
const METHODS: &[Method] = &[
    Method {
        path: &["docs", "+write"],
        sources: &[source("--text", Shape::Whole, ValueKind::Prose, false)],
    },
    Method {
        path: &["docs", "documents", "batchUpdate"],
        sources: &[
            source(
                "--json",
                Shape::Json("requests[*].insertText.text"),
                ValueKind::Prose,
                false,
            ),
            source(
                "--json",
                Shape::Json("requests[*].replaceAllText.replaceText"),
                ValueKind::Fragment,
                false,
            ),
        ],
    },
    Method {
        path: &["docs", "documents", "create"],
        sources: &[source(
            "--json",
            Shape::Json("title"),
            ValueKind::Fragment,
            false,
        )],
    },
    Method {
        path: &["sheets", "+append"],
        sources: &[
            source("--values", Shape::Commas, ValueKind::Fragment, true),
            source(
                "--json-values",
                Shape::Json("[*][*]"),
                ValueKind::Fragment,
                true,
            ),
        ],
    },
    Method {
        path: &["sheets", "spreadsheets", "values", "update"],
        sources: &[cells("values[*][*]")],
    },
    Method {
        path: &["sheets", "spreadsheets", "values", "append"],
        sources: &[cells("values[*][*]")],
    },
    Method {
        path: &["sheets", "spreadsheets", "values", "batchUpdate"],
        sources: &[cells("data[*].values[*][*]")],
    },
    Method {
        path: &[
            "sheets",
            "spreadsheets",
            "values",
            "batchUpdateByDataFilter",
        ],
        sources: &[cells("data[*].values[*][*]")],
    },
    Method {
        path: &["sheets", "spreadsheets", "batchUpdate"],
        sources: &[
            cells("requests[*].updateCells.rows[*].values[*].userEnteredValue.stringValue"),
            cells("requests[*].appendCells.rows[*].values[*].userEnteredValue.stringValue"),
            cells("requests[*].repeatCell.cell.userEnteredValue.stringValue"),
        ],
    },
];

/// 書き込み先を表すフラグ (ヘルパーのコマンド)。
const DESTINATION_FLAGS: [&str; 2] = ["--document", "--spreadsheet"];

/// 書き込み先を表す `--params` のキー。
const DESTINATION_PARAMS: [&str; 3] = ["documentId", "spreadsheetId", "range"];

/// 値を取らないフラグ (gws 0.22)。ほかの `--` で始まるフラグは、`=` で値をつながなければ次の引数を
/// 値に取る。
const SWITCHES: [&str; 5] = [
    "--dry-run",
    "--page-all",
    "--help",
    "--version",
    "--resolve-refs",
];

/// 次の引数を値に取る 1 字のフラグ (`-o <PATH>`)。ほかの 1 字のフラグは値を取らない。
const SHORT_WITH_VALUE: [&str; 1] = ["-o"];

/// gws の呼び出しの引数を、位置引数とフラグに分けたもの。
#[derive(Debug, Default)]
struct Call<'a> {
    /// 値の決まる位置引数 (サービス・リソース・メソッド)。値の決まらない語は、空になって消えることも
    /// フラグになることもあるので入れない (`$EXTRA` を足しただけで書き込みを見逃さないように)。
    positional: Vec<&'a str>,
    /// フラグと値。値が決まらない (ない) ものは `None`。
    flags: Vec<(&'a str, Option<&'a str>)>,
    dry_run: bool,
    help: bool,
}

impl<'a> Call<'a> {
    fn parse(args: &'a [Arg]) -> Self {
        let mut call = Call::default();
        let mut rest = args.iter();
        let mut options_done = false;
        while let Some(arg) = rest.next() {
            let Some(s) = arg.as_static() else {
                continue;
            };
            if options_done || !s.starts_with('-') || s == "-" {
                call.positional.push(s);
            } else if s == "--" {
                options_done = true;
            } else if let Some((name, value)) = s.split_once('=').filter(|_| s.starts_with("--")) {
                call.flags.push((name, Some(value)));
            } else if SWITCHES.contains(&s)
                || (!s.starts_with("--") && !SHORT_WITH_VALUE.contains(&s))
            {
                call.dry_run |= s == "--dry-run";
                call.help |= s == "--help" || s == "-h";
            } else {
                call.flags.push((s, rest.next().and_then(Arg::as_static)));
            }
        }
        call
    }

    /// フラグの値。フラグがなければ `None`、値が決まらなければ `Some(None)`。同じフラグが 2 回あれば
    /// 後のもの。
    fn flag(&self, name: &str) -> Option<Option<&'a str>> {
        self.flags
            .iter()
            .rev()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| *v)
    }
}

/// gws の呼び出し 1 つ (プログラム名より後ろの引数) の書き込み。表にないコマンドと、値を 1 つも
/// 読めないものは `None`。
pub fn write(args: &[Arg]) -> Option<Write> {
    let call = Call::parse(args);
    if call.help {
        return None;
    }
    let method = METHODS
        .iter()
        .find(|m| m.path == call.positional.as_slice())?;
    let mut values = Vec::new();
    for source in method.sources {
        if let Some(Some(raw)) = call.flag(source.flag) {
            extract(source, raw, &mut values);
        }
    }
    if values.is_empty() {
        return None;
    }
    Some(Write {
        method: method.path.join(" "),
        dry_run: call.dry_run,
        destination: destination(&call),
        values,
    })
}

/// 書き込み先。
fn destination(call: &Call) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = DESTINATION_FLAGS
        .iter()
        .filter_map(|flag| Some((flag.to_string(), call.flag(flag)?.map(str::to_string))))
        .collect();
    match call.flag("--params") {
        Some(Some(params)) => {
            if let Ok(Json::Object(map)) = serde_json::from_str::<Json>(params) {
                for key in DESTINATION_PARAMS {
                    if let Some(v) = map.get(key) {
                        let v = v.as_str().map_or_else(|| v.to_string(), str::to_string);
                        out.push((key.to_string(), Some(v)));
                    }
                }
            }
        }
        Some(None) => out.push(("--params".to_string(), None)),
        None => {}
    }
    out
}

/// 表の項目 `source` の値を、フラグの値 `raw` から取り出す。
fn extract(source: &Source, raw: &str, out: &mut Vec<Value>) {
    let mut push = |label: String, text: String| {
        out.push(Value {
            label,
            source: source_name(source),
            kind: source.kind,
            cell: source.cell,
            text,
        });
    };
    match source.shape {
        Shape::Whole => push(source.flag.to_string(), raw.to_string()),
        Shape::Commas => {
            for (i, v) in raw.split(',').enumerate() {
                push(format!("{}[{i}]", source.flag), v.to_string());
            }
        }
        Shape::Json(path) => {
            // JSON として読めない値は gws が受け付けない
            let Ok(json) = serde_json::from_str::<Json>(raw) else {
                return;
            };
            let mut label = label_prefix(source.flag).to_string();
            let mut found = Vec::new();
            collect(&json, &steps(path), &mut label, &mut found);
            for (label, text) in found {
                push(label, text);
            }
        }
    }
}

/// 値の場所の頭に付けるフラグ名 (本文の `--json` は付けない)。
fn label_prefix(flag: &str) -> &str {
    if flag == "--json" { "" } else { flag }
}

/// 表の項目の呼び名 (`values[*][*]`・`--values`・`--json-values[*][*]` など)。
fn source_name(source: &Source) -> String {
    match source.shape {
        Shape::Whole | Shape::Commas => source.flag.to_string(),
        Shape::Json(path) => format!("{}{path}", label_prefix(source.flag)),
    }
}

/// JSON のパスの 1 段。
#[derive(Debug, PartialEq, Eq)]
enum Step<'a> {
    Key(&'a str),
    /// 配列の全要素。
    Each,
}

/// `requests[*].insertText.text` のようなパスを段に分ける。
fn steps(path: &str) -> Vec<Step<'_>> {
    let mut out = Vec::new();
    for part in path.split('.') {
        let key = part.split('[').next().unwrap_or("");
        if !key.is_empty() {
            out.push(Step::Key(key));
        }
        out.extend(part.matches("[*]").map(|_| Step::Each));
    }
    out
}

/// `json` の中で `steps` の指す文字列を、場所 (`label` に続けたもの) と一緒に集める。
fn collect(json: &Json, steps: &[Step], label: &mut String, out: &mut Vec<(String, String)>) {
    let Some((step, rest)) = steps.split_first() else {
        if let Json::String(s) = json {
            out.push((label.clone(), s.clone()));
        }
        return;
    };
    let len = label.len();
    match step {
        Step::Key(key) => {
            if let Some(v) = json.get(key) {
                if !label.is_empty() {
                    label.push('.');
                }
                label.push_str(key);
                collect(v, rest, label, out);
            }
        }
        Step::Each => {
            if let Json::Array(items) = json {
                for (i, v) in items.iter().enumerate() {
                    let _ = write!(label, "[{i}]");
                    collect(v, rest, label, out);
                    label.truncate(len);
                }
            }
        }
    }
    label.truncate(len);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(write: &Write) -> Vec<(&str, &str, ValueKind, bool)> {
        write
            .values
            .iter()
            .map(|v| (v.label.as_str(), v.text.as_str(), v.kind, v.cell))
            .collect()
    }

    fn one(command: &str) -> Write {
        let mut found = writes(command);
        assert_eq!(found.len(), 1, "{command}: {found:?}");
        found.remove(0)
    }

    #[test]
    fn docs_write_text_is_prose() {
        let w = one("gws docs +write --document D1 --text '本文です。'");
        assert_eq!(w.method, "docs +write");
        assert!(!w.dry_run);
        assert_eq!(
            labels(&w),
            vec![("--text", "本文です。", ValueKind::Prose, false)]
        );
        assert_eq!(w.values[0].source, "--text");
        assert_eq!(
            w.destination,
            vec![("--document".to_string(), Some("D1".to_string()))]
        );
    }

    #[test]
    fn docs_batch_update_reads_insert_and_replace_texts() {
        let json = r#"{"requests":[
            {"insertText":{"text":"一つ目","location":{"index":1}}},
            {"replaceAllText":{"containsText":{"text":"旧"},"replaceText":"新しい語"}},
            {"insertText":{"text":"二つ目"}},
            {"deleteContentRange":{"range":{"startIndex":1,"endIndex":2}}}
        ]}"#;
        let w = one(&format!(
            "gws docs documents batchUpdate --params '{{\"documentId\":\"D2\"}}' --json '{json}'"
        ));
        assert_eq!(w.method, "docs documents batchUpdate");
        assert_eq!(
            labels(&w),
            vec![
                (
                    "requests[0].insertText.text",
                    "一つ目",
                    ValueKind::Prose,
                    false
                ),
                (
                    "requests[2].insertText.text",
                    "二つ目",
                    ValueKind::Prose,
                    false
                ),
                (
                    "requests[1].replaceAllText.replaceText",
                    "新しい語",
                    ValueKind::Fragment,
                    false
                ),
            ]
        );
        assert_eq!(w.values[0].source, "requests[*].insertText.text");
        assert_eq!(w.values[2].source, "requests[*].replaceAllText.replaceText");
        assert_eq!(
            w.destination,
            vec![("documentId".to_string(), Some("D2".to_string()))]
        );
    }

    #[test]
    fn docs_create_reads_the_title() {
        let w = one(r#"gws docs documents create --json '{"title":"議事録の下書き"}'"#);
        assert_eq!(
            labels(&w),
            vec![("title", "議事録の下書き", ValueKind::Fragment, false)]
        );
    }

    #[test]
    fn sheets_append_reads_comma_values_and_json_values() {
        let w = one("gws sheets +append --spreadsheet S1 --values '氏名, 所属,=SUM(A1:A2)'");
        assert_eq!(w.method, "sheets +append");
        assert_eq!(
            labels(&w),
            vec![
                ("--values[0]", "氏名", ValueKind::Fragment, true),
                ("--values[1]", " 所属", ValueKind::Fragment, true),
                ("--values[2]", "=SUM(A1:A2)", ValueKind::Fragment, true),
            ]
        );
        assert_eq!(w.values[0].source, "--values");
        assert_eq!(
            w.destination,
            vec![("--spreadsheet".to_string(), Some("S1".to_string()))]
        );

        let w = one(
            r#"gws sheets +append --spreadsheet S1 --json-values '[["一","二"],["三",4,null]]'"#,
        );
        assert_eq!(
            labels(&w),
            vec![
                ("--json-values[0][0]", "一", ValueKind::Fragment, true),
                ("--json-values[0][1]", "二", ValueKind::Fragment, true),
                ("--json-values[1][0]", "三", ValueKind::Fragment, true),
            ]
        );
        assert_eq!(w.values[0].source, "--json-values[*][*]");
    }

    #[test]
    fn sheets_values_methods_read_cells() {
        for method in ["update", "append"] {
            let w = one(&format!(
                r#"gws sheets spreadsheets values {method} --params '{{"spreadsheetId":"S","range":"シート1!A1","valueInputOption":"RAW"}}' --json '{{"values":[["甲","乙"],[1,"丙"]]}}'"#
            ));
            assert_eq!(w.method, format!("sheets spreadsheets values {method}"));
            assert_eq!(
                labels(&w),
                vec![
                    ("values[0][0]", "甲", ValueKind::Fragment, true),
                    ("values[0][1]", "乙", ValueKind::Fragment, true),
                    ("values[1][1]", "丙", ValueKind::Fragment, true),
                ]
            );
            assert_eq!(
                w.destination,
                vec![
                    ("spreadsheetId".to_string(), Some("S".to_string())),
                    ("range".to_string(), Some("シート1!A1".to_string())),
                ]
            );
        }
        for method in ["batchUpdate", "batchUpdateByDataFilter"] {
            let w = one(&format!(
                r#"gws sheets spreadsheets values {method} --json '{{"data":[{{"range":"A1","values":[["丁"]]}},{{"values":[["戊","己"]]}}]}}'"#
            ));
            assert_eq!(
                labels(&w),
                vec![
                    ("data[0].values[0][0]", "丁", ValueKind::Fragment, true),
                    ("data[1].values[0][0]", "戊", ValueKind::Fragment, true),
                    ("data[1].values[0][1]", "己", ValueKind::Fragment, true),
                ]
            );
        }
    }

    #[test]
    fn sheets_batch_update_reads_string_values_of_cell_requests() {
        let json = r#"{"requests":[
            {"updateCells":{"rows":[{"values":[{"userEnteredValue":{"stringValue":"更新"}},{"userEnteredValue":{"numberValue":1}}]}],"fields":"*"}},
            {"appendCells":{"rows":[{"values":[{"userEnteredValue":{"stringValue":"追記"}}]}]}},
            {"repeatCell":{"cell":{"userEnteredValue":{"stringValue":"一括"}},"fields":"*"}},
            {"addSheet":{"properties":{"title":"新しいシート"}}}
        ]}"#;
        let w = one(&format!(
            "gws sheets spreadsheets batchUpdate --json '{json}'"
        ));
        assert_eq!(
            labels(&w),
            vec![
                (
                    "requests[0].updateCells.rows[0].values[0].userEnteredValue.stringValue",
                    "更新",
                    ValueKind::Fragment,
                    true
                ),
                (
                    "requests[1].appendCells.rows[0].values[0].userEnteredValue.stringValue",
                    "追記",
                    ValueKind::Fragment,
                    true
                ),
                (
                    "requests[2].repeatCell.cell.userEnteredValue.stringValue",
                    "一括",
                    ValueKind::Fragment,
                    true
                ),
            ]
        );
    }

    #[test]
    fn flags_can_be_joined_with_equals_and_interleaved() {
        let w = one(
            r#"gws docs --dry-run documents --format json batchUpdate --json='{"requests":[{"insertText":{"text":"本文"}}]}' -o out.json"#,
        );
        assert!(w.dry_run);
        assert_eq!(w.method, "docs documents batchUpdate");
        assert_eq!(w.values[0].text, "本文");
    }

    #[test]
    fn heredoc_bodies_are_read() {
        let w = one(
            "gws docs documents batchUpdate --params '{\"documentId\":\"D\"}' --json \"$(cat <<'EOF'\n{\"requests\": [{\"insertText\": {\"text\": \"一行目\\n二行目\"}}]}\nEOF\n)\"",
        );
        assert_eq!(w.values[0].text, "一行目\n二行目");
    }

    #[test]
    fn several_writes_in_one_command_are_listed_in_order() {
        let found = writes(
            "gws docs +write --document A --text 'あ' && gws sheets +append --spreadsheet B --values 'い'; gws docs documents get --params '{\"documentId\":\"A\"}'",
        );
        let methods: Vec<&str> = found.iter().map(|w| w.method.as_str()).collect();
        assert_eq!(methods, vec!["docs +write", "sheets +append"]);
    }

    #[test]
    fn reads_help_unknown_methods_and_dynamic_values_are_skipped() {
        for command in [
            // 読み取り・表にないコマンド
            "gws docs documents get --params '{\"documentId\":\"D\"}'",
            "gws sheets +read --spreadsheet S --range A1",
            "gws drive files list",
            "gws schema docs.documents.batchUpdate",
            "gws docs +write --document D --text '本文' --help",
            "gws docs +write --document D --text '本文' -h",
            // 値が決まらない・JSON として読めない・値がない
            "gws docs +write --document D --text \"$BODY\"",
            "gws docs +write --document D --text \"$(cat body.txt)\"",
            "gws docs documents batchUpdate --json '{\"requests\": ['",
            "gws docs +write --document D --text",
            "gws docs +write --document D",
            // メソッドの名前が決まらない
            "gws docs $SUB --text '本文'",
        ] {
            assert_eq!(writes(command), Vec::<Write>::new(), "{command}");
        }
    }

    #[test]
    fn dynamic_words_do_not_hide_a_known_write() {
        // 値の決まらない語は空になって消えることがあるので、位置引数の照合に使わない
        for command in [
            "EMPTY=; gws docs +write --document D --text '本文' $EMPTY",
            "gws docs +write $EXTRA --text '本文'",
            "gws $GLOBAL docs +write --text '本文'",
        ] {
            let w = one(command);
            assert_eq!(w.method, "docs +write", "{command}");
            assert_eq!(w.values[0].text, "本文", "{command}");
        }
    }

    #[test]
    fn dynamic_destinations_are_kept_as_unknown() {
        let w = one("gws docs +write --document \"$DOC\" --text '本文'");
        assert_eq!(w.destination, vec![("--document".to_string(), None)]);
        let w = one(
            "gws docs documents batchUpdate --params \"$P\" --json '{\"requests\":[{\"insertText\":{\"text\":\"本文\"}}]}'",
        );
        assert_eq!(w.destination, vec![("--params".to_string(), None)]);
    }

    #[test]
    fn every_table_path_is_well_formed() {
        for method in METHODS {
            assert!(!method.path.is_empty());
            for source in method.sources {
                assert!(source.flag.starts_with("--"), "{}", source.flag);
                if let Shape::Json(path) = source.shape {
                    let steps = steps(path);
                    assert!(!steps.is_empty(), "{path}");
                    let rebuilt: String = steps
                        .iter()
                        .enumerate()
                        .map(|(i, s)| match s {
                            Step::Key(k) if i == 0 => (*k).to_string(),
                            Step::Key(k) => format!(".{k}"),
                            Step::Each => "[*]".to_string(),
                        })
                        .collect();
                    assert_eq!(rebuilt, path, "パスの書式");
                }
            }
        }
        assert_eq!(steps("[*][*]"), vec![Step::Each, Step::Each]);
    }
}
