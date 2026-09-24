//! noslop — 日本語の文章から「AI 臭さ」を機械的に検出する Linter。
//!
//! 形態素解析の辞書を使わず (Zero-Dictionary)、文字種と語句のパターン、文長の統計だけで
//! 判定する。検出は疑いの提示であり、直すかどうかは書き手が決める。

pub mod calibrate;
pub mod cli;
pub mod config;
pub mod diagnostic;
pub mod dictionaries;
pub mod diff;
pub mod directive;
pub mod document;
pub mod engine;
pub mod genre;
pub mod heading;
pub mod hook;
pub mod markdown;
pub mod mcp;
pub mod morph;
pub mod output;
pub mod plaintext;
pub mod rules;
pub mod segment;
pub mod skill;
pub mod suppress;
pub mod text;
pub mod walk;

pub use diagnostic::{Diagnostic, Lane, RuleStatus, Severity, Span};
pub use document::{Document, ParseOptions, SourceFormat};
pub use genre::Genre;
