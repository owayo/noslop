//! noslop — 日本語の文章から「AI 臭さ」を機械的に検出する Linter。
//!
//! 文字種・語句パターン・文長の統計と、辞書があるときの形態素解析で検査する。
//! 検出は疑いの提示であり、直すかどうかは書き手が決める。
//!
//! 既定の feature を外すと、Markdown・テキストの検査と改稿の比較を、tree-sitter や CLI の
//! 依存なしで使える。コードの検査には `code`、配布辞書の取得には `dictionary-download`、
//! CLI と連携機能には `cli` を有効にする。既定は `cli` と `bundled-dict`。

#[cfg(feature = "cli")]
pub mod calibrate;
#[cfg(feature = "cli")]
mod changed;
#[cfg(feature = "cli")]
pub mod cli;
#[cfg(feature = "code")]
pub mod code;
pub mod config;
pub mod diagnostic;
pub mod dictionaries;
pub mod diff;
pub mod directive;
pub mod document;
pub mod engine;
pub mod genre;
#[cfg(feature = "cli")]
pub mod gws;
pub mod heading;
#[cfg(feature = "cli")]
pub mod hook;
mod html;
pub mod language;
pub mod markdown;
#[cfg(feature = "cli")]
pub mod mcp;
pub mod morph;
pub mod output;
pub mod plaintext;
#[cfg(feature = "cli")]
pub mod review;
pub mod rules;
pub mod segment;
#[cfg(feature = "cli")]
pub mod skill;
pub mod structure;
pub mod suppress;
pub mod text;
#[cfg(feature = "cli")]
pub mod walk;

pub use diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
pub use document::{Document, ParseError, ParseOptions, SourceFormat};
pub use genre::Genre;
