//! git の差分から、変わった行を求める。

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::diagnostic::Span;
use crate::document::Document;

/// git の差分 (HEAD との比較) で変わった行 (原文上の範囲)。git の外・追跡していないファイル・
/// HEAD がない・git を実行できないときは `None` (ファイル全体)。コミットしていない変更がなければ空。
pub(super) fn git_changed_regions(path: &Path, doc: &Document) -> Option<Vec<Span>> {
    let lines = git_changed_lines(path)?;
    Some(
        lines
            .into_iter()
            .map(|line| doc.lines.line_span(&doc.source, line))
            .collect(),
    )
}

fn git_changed_lines(path: &Path) -> Option<BTreeSet<usize>> {
    // git -C でファイルのディレクトリに移るので、相対パスのままでは指す先がずれる
    let path = std::path::absolute(path).ok()?;
    let dir = path.parent()?;
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .arg(&path)
            // パスの * や ? をパターンとして読ませない
            .env("GIT_LITERAL_PATHSPECS", "1")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
    };
    // 追跡していないファイル (新しく作った・無視している) はファイル全体を見る
    if !git(&["ls-files", "--error-unmatch", "--"])?
        .status
        .success()
    {
        return None;
    }
    let diff = git(&[
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "-U0",
        "HEAD",
        "--",
    ])?;
    if !diff.status.success() {
        return None;
    }
    unified_diff_lines(&String::from_utf8_lossy(&diff.stdout))
}

/// `git diff -U0` の出力から、変わった後のファイルの行番号 (1 始まり) を集める。削除だけの箇所は、
/// つなぎ目の前後の行を入れる。hunk の見出しの形が想定と違えば `None`。
pub(super) fn unified_diff_lines(diff: &str) -> Option<BTreeSet<usize>> {
    let mut lines = BTreeSet::new();
    // -U0 では本文の行は + か - で始まるので、@@ で始まる行は hunk の見出しだけ
    for header in diff.lines().filter(|l| l.starts_with("@@ ")) {
        // @@ -<旧の開始>[,<行数>] +<新の開始>[,<行数>] @@
        let new = header.split_whitespace().nth(2)?.strip_prefix('+')?;
        let (start, count) = match new.split_once(',') {
            Some((start, count)) => (start.parse::<usize>().ok()?, count.parse::<usize>().ok()?),
            None => (new.parse::<usize>().ok()?, 1),
        };
        if count == 0 {
            // 削除だけ: start 行の後ろが消えた
            lines.insert(start.max(1));
            lines.insert(start + 1);
        } else {
            lines.extend(start.max(1)..start + count);
        }
    }
    Some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unified_diff_lines_follow_the_hunk_headers() {
        let diff = "diff --git a/g.md b/g.md\nindex 1..2 100644\n--- a/g.md\n+++ b/g.md\n\
            @@ -3 +3 @@ 見出し\n-old\n+new\n\
            @@ -5,2 +6,0 @@\n-a\n-b\n\
            @@ -9,0 +10,2 @@\n+c\n+d\n";
        let lines: Vec<usize> = unified_diff_lines(diff).unwrap().into_iter().collect();
        // 3 行目の書き換え、6 行目の後ろの削除 (つなぎ目の 6・7 行目)、10〜11 行目の追加
        assert_eq!(lines, vec![3, 6, 7, 10, 11]);
        // 新しいファイルの全行と、差分なし
        let lines: Vec<usize> = unified_diff_lines("@@ -0,0 +1,3 @@\n+a\n+b\n+c\n")
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(lines, vec![1, 2, 3]);
        assert_eq!(unified_diff_lines(""), Some(BTreeSet::new()));
        assert_eq!(unified_diff_lines("@@ -1 +x @@\n"), None);
    }
}
