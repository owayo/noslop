//! PreToolUse (Bash): gws で Google ドキュメント・スプレッドシートに書き込む値を検査する。

use serde_json::Value;

use crate::cli::{self, HookArgs};

/// PreToolUse の入力に対して標準出力に書く JSON。何も書かないなら `None`。
pub(super) fn pre_tool_use(
    _event: &Value,
    _args: &HookArgs,
    _env: &cli::Environment,
) -> Result<Option<String>, String> {
    // TODO: gws の書き込みの値を取り出して検査する
    Ok(None)
}
