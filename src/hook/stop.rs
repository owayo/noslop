//! Stop: リポジトリの差分 (HEAD との差分と追跡していないファイル) の、変わった行に重なる指摘を返す。
//!
//! Claude Code の Stop フック (`hook claude-code`) と、フックの入力を渡せない Stop の仕組み向けの
//! `hook git-diff` の 2 つの出口がある。

use serde_json::Value;

use crate::cli::{self, GitDiffHookArgs, HookArgs};

/// Stop の入力に対して標準出力に書く JSON。何も書かないなら `None`。
pub(super) fn stop(
    _event: &Value,
    _args: &HookArgs,
    _env: &cli::Environment,
) -> Result<Option<String>, String> {
    // TODO: リポジトリの差分を検査する
    Ok(None)
}

/// `noslop hook git-diff` の本体。終了コードを返す。
pub fn git_diff(_args: &GitDiffHookArgs) -> u8 {
    // TODO: リポジトリの差分を検査する
    0
}
