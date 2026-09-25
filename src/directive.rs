//! 抑制コメント (`<!-- noslop-disable-next-line P01 -- 理由 -->`) の読み取り。
//!
//! 読み取るだけで、診断への適用はエンジン側で行う。コードのファイルでは、その言語のコメント
//! (`// noslop-disable-next-line P01 -- 理由`) の中身全体が同じ書式のものも抑制として読む
//! ([`parse_code_comment`])。

use std::sync::LazyLock;

use regex::Regex;

use crate::diagnostic::Span;
use crate::document::{Directive, DirectiveKind};

/// コメントの中身 (コメントの記号を外したもの) 全体が抑制の書式か (先頭から末尾まで)。
static BODY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?s)\A\s*noslop-(disable-next-line|disable-line|disable-file|disable|enable)(?:\s+(.*?))?\s*\z",
    )
    .expect("directive regex")
});

static COMMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").expect("comment regex"));

/// 原文の `region` にある抑制コメントを読み取って `out` に追加する。
///
/// HTML コメントは入れ子にならない (最初の `-->` で閉じる) ので、先にコメントを
/// 1 つずつ切り出し、コメント全体が書式に一致するものだけを抑制として読む。
/// 「`<!-- 例: <!-- noslop-disable-file --> -->`」のように、普通のコメントの中に
/// 書いた記法の例は抑制にならない。
pub fn scan(source: &str, region: Span, out: &mut Vec<Directive>) {
    let text = &source[region.range()];
    for comment in COMMENT_RE.find_iter(text) {
        let span = Span::new(region.start + comment.start(), region.start + comment.end());
        if let Some(directive) = parse_body(html_inner(comment.as_str()), span) {
            out.push(directive);
        }
    }
}

/// コードのコメントの中身 (その言語のコメントの記号を外したもの) 全体が抑制の書式なら、抑制を返す。
///
/// 書式は HTML コメントの中身と同じ (`noslop-disable-next-line P01, R03 -- 理由`) で、中身全体が
/// HTML コメントの形 (`<!-- noslop-... -->`) でもよい。文の途中に書いた `noslop-disable` は
/// 抑制にならない (記法の説明を書いたコメントを抑制として読まないため)。`span` はコメント全体の
/// 原文上の範囲。`noslop-disable-next-line` は、コメントが終わる行の次の行に当たる。
pub fn parse_code_comment(body: &str, span: Span) -> Option<Directive> {
    let body = body.trim();
    let inner = match body.strip_prefix("<!--") {
        Some(rest) if rest.len() >= "-->".len() && rest.ends_with("-->") => html_inner(body),
        _ => body,
    };
    parse_body(inner, span)
}

/// `<!-- ... -->` の中身 (`COMMENT_RE` に一致した文字列か、同じ形の文字列を渡す)。
fn html_inner(comment: &str) -> &str {
    &comment["<!--".len()..comment.len() - "-->".len()]
}

/// コメントの中身全体が抑制の書式なら、抑制を返す。
fn parse_body(body: &str, span: Span) -> Option<Directive> {
    let caps = BODY_RE.captures(body)?;
    let kind = match &caps[1] {
        "disable-next-line" => DirectiveKind::DisableNextLine,
        "disable-line" => DirectiveKind::DisableLine,
        "disable-file" => DirectiveKind::DisableFile,
        "disable" => DirectiveKind::Disable,
        _ => DirectiveKind::Enable,
    };
    let (rules, reason) = parse_args(caps.get(2).map_or("", |m| m.as_str()));
    Some(Directive {
        kind,
        rules,
        reason,
        span,
    })
}

/// HTML コメント (`<!-- ... -->`) の範囲をすべて返す (テキスト入力でコメントを本文から除くため)。
pub fn comment_spans(source: &str) -> Vec<Span> {
    COMMENT_RE
        .find_iter(source)
        .map(|m| Span::new(m.start(), m.end()))
        .collect()
}

/// `P01, R03 -- 理由` をルールの並びと理由に分ける。
fn parse_args(args: &str) -> (Vec<String>, Option<String>) {
    let (rules_part, reason) = match args.split_once("--") {
        Some((rules, reason)) => (rules, Some(reason.trim())),
        None => (args, None),
    };
    let rules = rules_part
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let reason = reason.filter(|r| !r.is_empty()).map(str::to_string);
    (rules, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan_all(src: &str) -> Vec<Directive> {
        let mut out = Vec::new();
        scan(src, Span::new(0, src.len()), &mut out);
        out
    }

    #[test]
    fn reads_kinds_rules_and_reason() {
        let src = "<!-- noslop-disable-next-line P01, R03 -- 引用なので残す -->\n本文";
        let d = scan_all(src);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, DirectiveKind::DisableNextLine);
        assert_eq!(d[0].rules, vec!["P01", "R03"]);
        assert_eq!(d[0].reason.as_deref(), Some("引用なので残す"));
        assert_eq!(d[0].span.start, 0);
    }

    #[test]
    fn reads_directives_without_arguments() {
        let d = scan_all("<!--noslop-disable-->a<!-- noslop-enable -->");
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].kind, DirectiveKind::Disable);
        assert!(d[0].rules.is_empty());
        assert_eq!(d[1].kind, DirectiveKind::Enable);
        assert!(d[1].reason.is_none());
    }

    #[test]
    fn distinguishes_disable_file_and_line() {
        let d = scan_all("<!-- noslop-disable-file P05 --><!-- noslop-disable-line -->");
        assert_eq!(d[0].kind, DirectiveKind::DisableFile);
        assert_eq!(d[0].rules, vec!["P05"]);
        assert_eq!(d[1].kind, DirectiveKind::DisableLine);
    }

    #[test]
    fn ignores_ordinary_comments() {
        assert!(scan_all("<!-- メモ -->").is_empty());
        assert_eq!(comment_spans("a<!-- x -->b").len(), 1);
    }

    #[test]
    fn reads_code_comment_bodies_in_both_forms() {
        let span = Span::new(3, 40);
        let d = parse_code_comment(
            " noslop-disable-next-line P01, R03 -- 引用なので残す ",
            span,
        )
        .unwrap();
        assert_eq!(d.kind, DirectiveKind::DisableNextLine);
        assert_eq!(d.rules, vec!["P01", "R03"]);
        assert_eq!(d.reason.as_deref(), Some("引用なので残す"));
        assert_eq!(d.span, span);
        // 中身全体が HTML コメントの形でもよい。複数行にまたがる理由も読む
        let d = parse_code_comment("<!-- noslop-disable-file -->", span).unwrap();
        assert_eq!(d.kind, DirectiveKind::DisableFile);
        let d = parse_code_comment("\nnoslop-disable P05 --\n複数行の理由\n", span).unwrap();
        assert_eq!(d.kind, DirectiveKind::Disable);
        assert_eq!(d.rules, vec!["P05"]);
        assert_eq!(d.reason.as_deref(), Some("複数行の理由"));
        assert_eq!(
            parse_code_comment("noslop-enable", span).map(|d| d.kind),
            Some(DirectiveKind::Enable)
        );
    }

    #[test]
    fn code_comments_that_only_mention_directives_are_not_directives() {
        let span = Span::new(0, 1);
        for body in [
            "ここで noslop-disable-next-line を書くと次の行を抑制できる。",
            "`noslop-disable-next-line` を使う",
            "noslop-disable-foo",
            "例: <!-- noslop-disable-file -->",
            "<!-->",
            "<!--->",
            "",
        ] {
            assert!(parse_code_comment(body, span).is_none(), "{body:?}");
        }
    }

    #[test]
    fn directive_examples_inside_ordinary_comments_are_not_directives() {
        assert!(scan_all("<!-- 例: <!-- noslop-disable-file --> -->").is_empty());
        assert!(scan_all("<!-- 書き方は noslop-disable P01 のように書く -->").is_empty());
        assert!(scan_all("<!-- noslop-disable-foo -->").is_empty());
        let d = scan_all("<!-- 例 --><!-- noslop-disable-line -->");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].kind, DirectiveKind::DisableLine);
        assert_eq!(d[0].span.start, "<!-- 例 -->".len());
    }
}
