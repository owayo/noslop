//! エージェントのフック (`noslop hook claude-code` / `noslop hook file` / `noslop hook git-diff`)。
//!
//! - `claude-code` ([`claude_code`]): Claude Code のフックの入力 (JSON) を受け、イベントごとに検査する。
//!   PostToolUse (Write / Edit / MultiEdit) は書き換えたファイル、PreToolUse (Bash) は gws で Google
//!   ドキュメント・スプレッドシートに書き込む値、Stop はリポジトリの差分を見る。
//! - `file` ([`file`]): 編集したファイルのパスだけを渡すフックの仕組み (claw-hooks の extension_hooks
//!   など) から呼ぶ。変わった行は git の差分 (HEAD との比較) から求め、結果をテキストで返す。
//! - `git-diff` ([`git_diff`]): フックの入力を渡せない Stop の仕組み (claw-hooks の stop_hooks など) から
//!   呼ぶ。リポジトリの差分を検査し、指摘があれば終了コード 1 にする。
//!
//! どれも、変わった行に重なる指摘だけを短い改稿指示 (brief) で返す (編集のたびに同じ指摘を繰り返し
//! 渡して、残すと決めた箇所まで直させないため)。指摘がないとき・対象外のときは何も出力しない。

mod claude_code;
mod file;
mod git;
mod gws;
mod stop;

pub use claude_code::{claude_code, respond};
pub use file::file;
pub use stop::git_diff;

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use crate::cli::{self, HookArgs};
use crate::diagnostic::{Diagnostic, Span};
use crate::document::{Document, ParseOptions, SourceFormat};
use crate::engine::{Engine, FileReport, RunReport};
use crate::output::{self, RenderOptions, RuleCatalog};
use crate::walk::WalkOptions;

/// additionalContext に入れる文字数の上限。Claude Code は 10,000 文字を超えた値を
/// ファイルに逃がして先頭しか見せないので、余裕を見てそれより短く切る。
const CONTEXT_BUDGET_CHARS: usize = 9_000;

/// フックの入力 (標準入力の JSON) の上限。Write の入力には本文がまるごと入る。
const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;

/// 検査するファイルの上限。これより大きいファイルは、編集のたびに検査すると遅いので飛ばす。
const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;

/// 改稿指示の後ろに添える注記 (どちらのフックも)。
const KEEP_NOTE: &str = "このフックは編集のたびに動きます。一度見直して残すと決めた指摘は、再び出ても直す必要はありません。\n";

/// フックの検査の道具一式 (対象の選び方とエンジン)。設定を 1 度だけ読み、1 回のフックの中で
/// 複数のファイルや文書を検査するのに使い回す。
struct Reviewer {
    walk: WalkOptions,
    engine: Engine,
    brief_limit: usize,
}

impl Reviewer {
    /// 設定ファイルを `cwd` から親へ探して読み、フックの引数を重ねてエンジンを作る。
    /// 読みやすさのレーンは、`--include-readability` がなければ止める。
    fn new(args: &HookArgs, cwd: Option<&Path>, env: &cli::Environment) -> Result<Self, String> {
        let cfg = cli::load_config_from(&args.config, cwd, env).map_err(|e| e.to_string())?;
        let walk = cli::walk_options(&cfg).map_err(|e| e.to_string())?;
        let mut options = cli::config_engine_options(&cfg, env);
        if let Some(genre) = args.genre {
            options.genre = genre;
        }
        options.experimental |= args.experimental;
        options.selection.no_readability = !args.include_readability;
        let engine = Engine::new(options).map_err(|e| e.to_string())?;
        Ok(Self {
            walk,
            engine,
            brief_limit: args.brief_limit,
        })
    }

    /// `root` を起点に検査するとき、`path` を検査するか (文書・コードの拡張子、設定の除外、
    /// `.noslopignore`)。
    fn selects(&self, root: &Path, path: &Path) -> bool {
        self.walk.selects(path)
            && !self.walk.is_excluded(root, path, false)
            && !crate::walk::is_noslopignored(path)
    }

    /// ファイルを読んで検査する。消えたファイルと大きすぎるファイルは `None`。
    fn lint_file(&self, path: &Path, name: String) -> Result<Option<FileReport>, String> {
        let Some(source) = read_source(path, &name)? else {
            return Ok(None);
        };
        Ok(Some(self.engine.lint_source(
            name,
            source,
            SourceFormat::from_path(path),
        )))
    }

    /// 読み込み済みの文書を検査する (gws で書き込む値のように、ファイルでない文書)。
    fn lint(&self, doc: Document) -> FileReport {
        self.engine.lint(doc)
    }

    /// 文書の読み込み方の設定 (改行の扱い)。値から文書を組み立てるときに使う。
    fn parse_options(&self) -> &ParseOptions {
        &self.engine.options().parse
    }

    /// 検査の結果を短い改稿指示にする。見せる指摘が 1 件もなければ `None`。
    fn brief(&self, files: Vec<FileReport>) -> Result<Option<String>, String> {
        let files: Vec<FileReport> = files
            .into_iter()
            .filter(|f| f.visible().next().is_some())
            .collect();
        if files.is_empty() {
            return Ok(None);
        }
        let opts = RenderOptions {
            genre: self.engine.options().genre,
            experimental: self.engine.options().experimental,
            brief_limit: Some(self.brief_limit),
            brief_compact: true,
            catalog: RuleCatalog::from_engine(&self.engine),
            ..Default::default()
        };
        let report = RunReport {
            files,
            errors: Vec::new(),
            morphology: self.engine.morphology().clone(),
        };
        let mut buf = Vec::new();
        output::brief::render(&report, &opts, &mut buf).map_err(|e| e.to_string())?;
        String::from_utf8(buf).map(Some).map_err(|e| e.to_string())
    }
}

/// フックの検査の結果。
struct Review {
    /// 短い改稿指示。
    brief: String,
    /// 表示名。
    name: String,
    /// 変わった行に絞ったか (絞れなければファイル全体)。
    limited: bool,
}

/// 編集したファイルを検査し、変わった行 (`changed` が求める。`None` ならファイル全体) に重なる指摘の
/// 改稿指示を作る。対象外のファイル (拡張子・除外・なくなった・大きすぎる) と、指摘がないときは `None`。
fn review(
    path: &Path,
    cwd: Option<&Path>,
    args: &HookArgs,
    env: &cli::Environment,
    changed: impl FnOnce(&Document) -> Option<Vec<Span>>,
) -> Result<Option<Review>, String> {
    let reviewer = Reviewer::new(args, cwd, env)?;
    // ユーザーの設定の除外は、検査の起点 (エージェントの作業ディレクトリ。なければファイルの
    // ディレクトリ) が基準
    let root = cwd.or_else(|| path.parent()).unwrap_or(Path::new("."));
    if !reviewer.selects(root, path) {
        return Ok(None);
    }
    let name = display_name(path, cwd);
    let Some(mut file) = reviewer.lint_file(path, name.clone())? else {
        return Ok(None);
    };
    let changed = changed(&file.doc);
    if let Some(regions) = &changed {
        file.diagnostics.retain(|d| touches(d, regions));
    }
    let Some(brief) = reviewer.brief(vec![file])? else {
        return Ok(None);
    };
    Ok(Some(Review {
        brief,
        name,
        limited: changed.is_some(),
    }))
}

/// 表示名 (作業ディレクトリの中なら相対パス)。
fn display_name(path: &Path, cwd: Option<&Path>) -> String {
    let relative = cwd
        .and_then(|dir| path.strip_prefix(dir).ok())
        .filter(|p| !p.as_os_str().is_empty());
    crate::walk::display(relative.unwrap_or(path))
}

/// フックの入力 (標準入力) を読む。
fn read_input() -> Result<String, String> {
    let mut input = String::new();
    match io::stdin()
        .take(MAX_INPUT_BYTES + 1)
        .read_to_string(&mut input)
    {
        Ok(n) if n as u64 > MAX_INPUT_BYTES => Err(format!(
            "フックの入力が大きすぎます (上限 {} MiB)",
            MAX_INPUT_BYTES / 1024 / 1024
        )),
        Ok(_) => Ok(input),
        Err(e) => Err(format!("フックの入力を読めません: {e}")),
    }
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

/// フックのテストで共有する道具。
#[cfg(test)]
mod testing {
    use crate::cli;

    /// 独自ルールで「ユーザー様」を指摘する設定 (組み込みルールの増減で壊れないように)。
    pub(super) const CONFIG: &str = r#"
[[custom]]
id = "X01"
name = "TEAM_TERM"
pattern = "ユーザー様"
message = "「ユーザー様」ではなく「利用者」と書きます"
hint = "用語集の表記に合わせてください"
severity = "warning"
"#;

    pub(super) const DOC: &str =
        "# 案内\n\nユーザー様の声を集めました。\n\n新しい段落です。ユーザー様に届けます。\n";

    /// `CONFIG` の設定と、本文 `doc` の guide.md を置いた作業ディレクトリ。
    pub(super) fn workspace(doc: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("noslop.toml"), CONFIG).unwrap();
        std::fs::write(dir.path().join("guide.md"), doc).unwrap();
        dir
    }

    /// `noslop hook <sub> <extra...>` の引数を読む。
    pub(super) fn parse(argv: &[&str]) -> cli::HookCommand {
        use clap::Parser;
        let mut all = vec!["noslop", "hook"];
        all.extend_from_slice(argv);
        match cli::Cli::try_parse_from(all).unwrap().command {
            cli::Command::Hook(command) => command,
            _ => panic!("hook"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
