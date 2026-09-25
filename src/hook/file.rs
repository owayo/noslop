//! `noslop hook file`: 編集したファイルのパスだけを渡すフックの仕組み (claw-hooks の extension_hooks
//! など) から呼ぶ。
//!
//! 検査と改稿指示は claude-code と同じで、結果は JSON ではなくテキストで書く (呼び出し側がエージェントに
//! 渡す)。変わった行は git の差分 (HEAD との比較) から求める。編集のたびにコミットしない運用でも、
//! 前のコミットから変えた行に絞れる。

use std::io::{self, Write};
use std::path::Path;

use super::git::git_changed_regions;
use super::{KEEP_NOTE, review, truncate_lines};
use crate::cli::{self, FileHookArgs};
use crate::document::Document;

/// `noslop hook file` の本体。指摘があれば改稿指示をテキストで標準出力に書き、終了コードを返す。
///
/// 誤りは標準エラーに書いて 1 で終わる。
pub fn file(args: &FileHookArgs) -> u8 {
    let cwd = std::env::current_dir().ok();
    match review_file(args, cwd.as_deref(), &cli::Environment::from_process()) {
        Ok(Some(text)) => {
            let mut out = io::stdout().lock();
            match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
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

/// [`file`] が書く改稿指示。何も書かないなら `None`。`cwd` はエージェントの作業ディレクトリ
/// (設定ファイルを探し始める場所と表示名の基準)。
fn review_file(
    args: &FileHookArgs,
    cwd: Option<&Path>,
    env: &cli::Environment,
) -> Result<Option<String>, String> {
    let path = match cwd {
        Some(dir) if args.path.is_relative() => dir.join(&args.path),
        _ => args.path.clone(),
    };
    let changed = |doc: &Document| {
        if args.hook.whole_file {
            None
        } else {
            git_changed_regions(&path, doc)
        }
    };
    let Some(review) = review(&path, cwd, &args.hook, env, changed)? else {
        return Ok(None);
    };
    let mut text = review.brief;
    text.push_str(KEEP_NOTE);
    if review.limited {
        text.push_str(&format!(
            "コミットしていない変更 (git の HEAD との差分) の行に重なる指摘だけを返しています。ファイル全体は `noslop check --format brief {}` で確認できます。\n",
            review.name
        ));
    }
    Ok(Some(truncate_lines(&text, args.max_chars)))
}

#[cfg(test)]
mod tests {
    use super::super::testing::{DOC, parse, workspace};
    use super::*;

    fn file_args(extra: &[&str]) -> FileHookArgs {
        let mut argv = vec!["file"];
        argv.extend_from_slice(extra);
        match parse(&argv) {
            cli::HookCommand::File(a) => a,
            _ => panic!("hook file"),
        }
    }

    /// パスだけを受け取るフックも、claude-code と同じ短い改稿指示をテキストで返す。git の外では
    /// ファイル全体を見る。上限を超える分は行の単位で省く。
    #[test]
    fn file_hook_returns_the_compact_brief_as_text() {
        let dir = workspace(DOC);
        let run = |extra: &[&str]| {
            review_file(
                &file_args(extra),
                Some(dir.path()),
                &cli::Environment::default(),
            )
            .unwrap()
        };
        let text = run(&["guide.md"]).unwrap();
        assert!(
            text.starts_with("noslop が guide.md に 独自ルールの指摘を 2 件見つけました。"),
            "{text}"
        );
        assert!(text.contains("- X01 "), "{text}");
        assert!(text.contains(KEEP_NOTE));
        assert!(
            !text.contains("コミットしていない変更"),
            "git の外ではファイル全体: {text}"
        );
        assert!(!text.trim_start().starts_with('{'), "JSON ではなくテキスト");

        let short = run(&["--max-chars", "120", "guide.md"]).unwrap();
        assert!(short.chars().count() <= 120, "{short}");
        assert!(short.contains("残り"), "{short}");

        // 対象外の拡張子・ないファイル・指摘のないファイルは何も返さない
        std::fs::write(dir.path().join("notes.rs"), DOC).unwrap();
        assert_eq!(run(&["notes.rs"]), None);
        assert_eq!(run(&["missing.md"]), None);
        std::fs::write(
            dir.path().join("clean.md"),
            "# 見出し\n\n何もない段落です。\n",
        )
        .unwrap();
        assert_eq!(run(&["clean.md"]), None);
    }
}
