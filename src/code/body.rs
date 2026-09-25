//! コメントの記号・飾りを外した本文の行と、検査から外すコメントの判定、行のコメントのまとめ方。
//!
//! - 本文の行は原文上の範囲で持つ (記号を外した後も、1 字ずつ原文の位置に戻せるように)
//! - ブロックのコメントは、2 行目からの行頭の `*` を飾りとして外す (その行がすべて `*` で始まるとき)
//! - shebang、ツールへの指示 (`eslint-disable` など)、抑制コメント (`noslop-disable-next-line`) は
//!   コメント 1 つずつ見て外す。抑制コメントは [`Directive`] にする
//! - 同じ記号で書いた行のコメントが、同じ列から隣り合う行に続いていれば 1 つのまとまりにする。
//!   空いた行・コード・記号の違い (`//` と `///`) で分け、コードの後ろに書いたコメントは単独にする
//! - 著作権・ライセンスの表記のまとまりは外す

use std::sync::LazyLock;

use regex::Regex;

use super::extract::{RawComment, Reading, Shape};
use crate::diagnostic::Span;
use crate::directive;
use crate::document::{Directive, LineIndex};

/// ツールへの指示で始まるコメント (リンター・整形・型検査・カバレッジ・エディタの設定など)。
///
/// 説明の文を添えられる指示 (`eslint-disable-next-line no-console -- 理由`) もあるが、コメント
/// ごとツールのものとして外す。普通の説明を誤って外さないよう、ツール名に記号まで続けた形にしている。
static TOOL_DIRECTIVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^(?:",
        // JavaScript・TypeScript
        r"eslint(?:-disable|-enable|-env)?(?:-next-line|-line)?(?:\s|:|$)",
        r"|@ts-(?:ignore|expect-error|nocheck|check)\b",
        r"|prettier-ignore\b|biome-ignore\b|deno-(?:lint|fmt)-ignore\b|dprint-ignore\b",
        r"|(?:istanbul|c8|v8)\s+ignore\b|jshint\s|jslint\s|tslint:",
        r"|@(?:flow|jsx|jsxImportSource|jsxRuntime|jsxFrag|refresh|vite-ignore)\b",
        r"|[#@]__(?:PURE|NO_SIDE_EFFECTS)__|webpack[A-Z][A-Za-z]*:|[#@]\s*sourceMappingURL=",
        r"|<reference\s|<amd-(?:module|dependency)\b",
        // Python
        r"|noqa\b|type:\s*ignore\b|(?:pylint|pyright|mypy|ruff|isort):|fmt:\s*(?:on|off|skip)\b",
        r"|pragma:\s*no\s*(?:cover|branch)\b|nosec\b",
        // 文字コードの宣言とエディタの設定 (`-*- coding: utf-8 -*-`・`vim: set ts=4:`)
        r"|-\*-|(?:en)?coding[:=]|vim?:\s*(?:set\s+)?\w+=|ex:\s*set\s",
        // Go
        r"|go:[a-z]+\b|\+build\s|nolint\b|line\s+\S+:\d+",
        // C・C++
        r"|NOLINT|clang-format\s+(?:on|off)\b|cppcheck-suppress\b",
        // Ruby
        r"|rubocop:|frozen_string_literal:|typed:\s*(?:ignore|false|true|strict|strong)\b",
        // Swift・Kotlin・Java・C#
        r"|swiftlint:|swift-format-ignore\b|sourcery:|@formatter:|CHECKSTYLE[: ]|NOSONAR\b",
        r"|noinspection\s|language=\w|ReSharper\s+(?:disable|restore)\b",
        // PHP
        r"|phpcs:|@(?:phpstan|psalm)-|phpstan-ignore",
        // シェル・YAML ほか
        r"|shellcheck\s+(?:disable|enable|source|shell)=|hadolint\s|yamllint\s",
        r"|markdownlint-|stylelint-(?:disable|enable)|(?:cspell|spell-checker):|lgtm\b|codeql\b",
        r")"
    ))
    .expect("tool directive regex")
});

/// 記号・飾りを外したコメント。
#[derive(Debug, Clone)]
pub(super) struct Comment {
    pub raw: RawComment,
    /// 本文の行 (原文上の範囲。行末の空白と改行は含めない)。空いた行も含む。
    pub lines: Vec<Span>,
    /// 始まりの行と終わりの行 (1 始まり)。
    first_line: usize,
    last_line: usize,
    /// 始まりの位置の、行頭からのバイト数。
    column: usize,
    /// 同じ行のコメントの前にコードがあるか (行末のコメント)。
    trailing: bool,
}

/// 行のコメントをまとめたもの (ブロックのコメントと docstring は 1 つで 1 つ)。
#[derive(Debug, Clone)]
pub(super) struct Group {
    pub reading: Reading,
    /// 本文の行 (原文上の範囲)。Markdown・XML で読むものは、共通の字下げを外してある。
    pub lines: Vec<Span>,
}

/// コメントを本文の行にし、検査から外すものを除く。抑制コメントは [`Directive`] にして返す。
pub(super) fn prepare(source: &str, raws: Vec<RawComment>) -> (Vec<Comment>, Vec<Directive>) {
    let index = LineIndex::new(source);
    let mut comments = Vec::new();
    let mut directives = Vec::new();
    for raw in raws {
        let lines = body_lines(source, &raw);
        let body = joined(source, &lines);
        if raw.shape != Shape::Docstring {
            // shebang (ファイルの 1 行目の `#!`)
            if raw.span.start == 0 && source.starts_with("#!") {
                continue;
            }
            if let Some(d) = directive::parse_code_comment(&body, raw.span) {
                directives.push(d);
                continue;
            }
            if TOOL_DIRECTIVE.is_match(body.trim_start()) {
                continue;
            }
        }
        let line_start = source[..raw.span.start].rfind('\n').map_or(0, |i| i + 1);
        comments.push(Comment {
            first_line: index.line(raw.span.start),
            last_line: index.line(raw.span.end.saturating_sub(1).max(raw.span.start)),
            column: raw.span.start - line_start,
            trailing: !source[line_start..raw.span.start].trim().is_empty(),
            lines,
            raw,
        });
    }
    (comments, directives)
}

/// 同じ記号の行のコメントが同じ列から隣り合う行に続くものをまとめ、著作権・ライセンスの表記を除く。
pub(super) fn group(source: &str, comments: Vec<Comment>) -> Vec<Group> {
    let mut groups: Vec<(Comment, Vec<Span>)> = Vec::new();
    for comment in comments {
        if let Some((last, lines)) = groups.last_mut()
            && continues(last, &comment)
        {
            lines.extend(comment.lines.iter().copied());
            *last = comment;
            continue;
        }
        let lines = comment.lines.clone();
        groups.push((comment, lines));
    }
    groups
        .into_iter()
        .filter(|(_, lines)| !is_license(source, lines))
        .map(|(last, mut lines)| {
            let reading = last.raw.reading;
            if reading != Reading::Plain {
                // 1 行目がブロックの開きの記号と同じ行にあるもの (`/** 最初の行` や `"""最初の行`) は、
                // 1 行目を除いて共通の字下げを外す (Python の docstring の読み方と同じ)
                let opening = last.raw.shape != Shape::Line;
                dedent(source, &mut lines, opening);
            }
            Group { reading, lines }
        })
        .collect()
}

/// `b` が `a` のまとまりに続くか。
fn continues(a: &Comment, b: &Comment) -> bool {
    a.raw.shape == Shape::Line
        && b.raw.shape == Shape::Line
        && a.raw.marker == b.raw.marker
        && a.raw.reading == b.raw.reading
        && !a.trailing
        && !b.trailing
        && b.first_line == a.last_line + 1
        && b.column == a.column
}

/// コメントの記号の内側を行に分け、ブロックの飾りの `*` と行末の空白を外す。
fn body_lines(source: &str, raw: &RawComment) -> Vec<Span> {
    let mut lines = Vec::new();
    let mut start = raw.body.start;
    for piece in source[raw.body.range()].split('\n') {
        lines.push(Span::new(start, start + piece.len()));
        start += piece.len() + 1;
    }
    if raw.shape == Shape::Block && lines.len() > 1 {
        let text = |l: &Span| &source[l.range()];
        let blank = |l: &Span| text(l).trim().is_empty();
        let starred = |l: &Span| text(l).trim_start_matches([' ', '\t']).starts_with('*');
        let rest = &lines[1..];
        if rest.iter().any(|l| !blank(l)) && rest.iter().all(|l| blank(l) || starred(l)) {
            for l in &mut lines[1..] {
                let s = text(l);
                let indent = s.len() - s.trim_start_matches([' ', '\t']).len();
                if s[indent..].starts_with('*') {
                    l.start += indent + 1;
                }
            }
        }
    }
    for l in &mut lines {
        l.end = l.start + source[l.range()].trim_end().len();
    }
    lines
}

/// 本文の行を改行でつないだ文字列。
fn joined(source: &str, lines: &[Span]) -> String {
    lines
        .iter()
        .map(|l| &source[l.range()])
        .collect::<Vec<_>>()
        .join("\n")
}

/// 共通の字下げ (行頭の半角空白とタブのバイト数の最小) を外す。空白だけの行は空にする。
///
/// `opening` が真なら、1 行目は行頭の空白をすべて外し、共通の字下げは 2 行目から数える。
fn dedent(source: &str, lines: &mut [Span], opening: bool) {
    let indent_of = |l: &Span| {
        let s = &source[l.range()];
        s.len() - s.trim_start_matches([' ', '\t']).len()
    };
    let (first, rest) = if opening {
        match lines.split_first_mut() {
            Some((first, rest)) => (Some(first), rest),
            None => return,
        }
    } else {
        (None, lines)
    };
    if let Some(first) = first {
        first.start += indent_of(first);
    }
    let common = rest
        .iter()
        .filter(|l| !source[l.range()].trim().is_empty())
        .map(indent_of)
        .min()
        .unwrap_or(0);
    for l in rest {
        if source[l.range()].trim().is_empty() {
            l.start = l.end;
        } else {
            l.start += common;
        }
    }
}

/// 著作権・ライセンスの表記か (SPDX の識別子を含むか、最初の行が著作権・ライセンスの表記で始まる)。
fn is_license(source: &str, lines: &[Span]) -> bool {
    let texts = || lines.iter().map(|l| source[l.range()].trim());
    if texts().any(|t| t.contains("SPDX-License-Identifier")) {
        return true;
    }
    texts().find(|t| !t.is_empty()).is_some_and(|first| {
        let lower = first.to_ascii_lowercase();
        [
            "copyright",
            "(c)",
            "©",
            "著作権",
            "licensed under",
            "@license",
        ]
        .iter()
        .any(|p| lower.starts_with(p))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(source: &str, body: &str, shape: Shape) -> RawComment {
        let start = source.find(body).unwrap();
        RawComment {
            span: Span::new(0, source.len()),
            body: Span::new(start, start + body.len()),
            shape,
            reading: Reading::Markdown,
            marker: "",
        }
    }

    fn texts<'a>(source: &'a str, lines: &[Span]) -> Vec<&'a str> {
        lines.iter().map(|l| &source[l.range()]).collect()
    }

    #[test]
    fn block_lines_lose_the_star_decoration_and_trailing_spaces() {
        let src = "/**\n * 一行目 \r\n *\n *  字下げ\n */";
        let lines = body_lines(src, &raw(src, &src[3..src.len() - 2], Shape::Block));
        assert_eq!(texts(src, &lines), ["", " 一行目", "", "  字下げ", ""]);
        // 行頭が `*` でない行があれば、飾りとみなさない
        let src = "/* 一行目\n   * 項目\n   二行目 */";
        let lines = body_lines(src, &raw(src, &src[2..src.len() - 2], Shape::Block));
        assert_eq!(texts(src, &lines), [" 一行目", "   * 項目", "   二行目"]);
    }

    #[test]
    fn dedent_keeps_relative_indentation() {
        let src = "  最初\n    続き\n\n      コード\n   ";
        let mut lines: Vec<Span> = {
            let mut out = Vec::new();
            let mut start = 0;
            for piece in src.split('\n') {
                out.push(Span::new(start, start + piece.len()));
                start += piece.len() + 1;
            }
            out
        };
        dedent(src, &mut lines, true);
        assert_eq!(texts(src, &lines), ["最初", "続き", "", "  コード", ""]);
    }

    #[test]
    fn license_headers_are_recognized() {
        for src in [
            "Copyright (c) 2026 Example 著作権者",
            "(C) 2026 例の会社",
            "© 2026 例",
            "著作権は作者にある",
            "SPDX-License-Identifier: MIT\n説明の文",
            "\nLicensed under the MIT License. 詳しくは LICENSE",
        ] {
            let lines = [Span::new(0, src.len())];
            assert!(is_license(src, &lines), "{src:?}");
        }
        let src = "この関数は著作権の表記を探す。";
        assert!(!is_license(src, &[Span::new(0, src.len())]));
    }

    #[test]
    fn tool_directives_are_recognized_but_prose_is_not() {
        for body in [
            "eslint-disable-next-line no-console -- デバッグ用に残す",
            "eslint no-var: off",
            "@ts-expect-error 型が合わない",
            "noqa: E501 長い URL のため",
            "type: ignore[attr-defined]",
            "pylint: disable=invalid-name",
            "nolint:errcheck 閉じる失敗は無視する",
            "NOLINTNEXTLINE(bugprone-*)",
            "rubocop:disable Style/AsciiComments",
            "swiftlint:disable:next force_cast",
            "prettier-ignore",
            "biome-ignore lint: 理由",
            "deno-lint-ignore no-explicit-any",
            "istanbul ignore next",
            "go:generate stringer -type=Kind",
            "-*- coding: utf-8 -*-",
            "vim: set ts=4 sw=4:",
            "shellcheck disable=SC2034",
            "<reference types=\"node\" />",
        ] {
            assert!(TOOL_DIRECTIVE.is_match(body), "{body:?}");
        }
        for body in [
            "eslintの設定を読む",
            "type: 文字列の種類",
            "line 数を数える",
            "go の文法で読む",
            "coding の規約に合わせる",
            "vim で開くと崩れる",
            "ex: 1 を渡すと 2 を返す",
            "設定を読む",
        ] {
            assert!(!TOOL_DIRECTIVE.is_match(body), "{body:?}");
        }
    }
}
