//! 抑制コメントを診断に適用する。
//!
//! 適用範囲の規則:
//!
//! - `noslop-disable-next-line`: コメントの**終端がある行の次の物理行**
//! - `noslop-disable-line`: コメントの**開始位置がある行**
//! - `noslop-disable` 〜 `noslop-enable`: disable のコメント終端から、対応する enable の開始まで。
//!   enable にルールの指定があればそのルールだけを閉じ、指定がなければすべて閉じる。
//!   閉じ忘れは文書末まで。
//! - `noslop-disable-file`: 文書全体
//!
//! 診断が抑制されるかは、診断の開始位置で判定する。ルールは ID か名前で書け、
//! 大文字小文字は区別しない。未知のルールや対応しない enable は警告として記録し、
//! エラーにはしない (文書の中身の問題で実行全体を止めないため)。
//! 抑制した診断は消さずに [`Suppression`] を付ける (「直さずに残した」記録を出力で追えるように)。

use std::collections::{BTreeSet, HashMap};

use crate::diagnostic::{Diagnostic, Suppression};
use crate::document::{DirectiveKind, Document};

/// ルール ID と名前 (小文字) から正規の ID を引く表。
#[derive(Debug, Clone, Default)]
pub struct RuleNames {
    by_key: HashMap<String, String>,
}

impl RuleNames {
    pub fn new<'a>(rules: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut by_key = HashMap::new();
        for (id, name) in rules {
            by_key.insert(id.to_ascii_lowercase(), id.to_string());
            by_key.insert(name.to_ascii_lowercase(), id.to_string());
        }
        Self { by_key }
    }

    /// ID か名前から正規の ID を返す。
    pub fn resolve(&self, key: &str) -> Option<&str> {
        self.by_key
            .get(&key.trim().to_ascii_lowercase())
            .map(String::as_str)
    }
}

/// 抑制の対象となるルールの集合。
#[derive(Debug, Clone, PartialEq, Eq)]
enum RuleSet {
    All,
    Only(BTreeSet<String>),
    AllExcept(BTreeSet<String>),
}

impl RuleSet {
    fn contains(&self, id: &str) -> bool {
        match self {
            RuleSet::All => true,
            RuleSet::Only(s) => s.contains(id),
            RuleSet::AllExcept(s) => !s.contains(id),
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, RuleSet::Only(s) if s.is_empty())
    }

    /// `other` に含まれるルールを取り除いた集合。
    fn minus(&self, other: &RuleSet) -> RuleSet {
        match (self, other) {
            (_, RuleSet::All) => RuleSet::Only(BTreeSet::new()),
            (RuleSet::All, RuleSet::Only(o)) => RuleSet::AllExcept(o.clone()),
            (RuleSet::Only(s), RuleSet::Only(o)) => RuleSet::Only(s - o),
            (RuleSet::AllExcept(s), RuleSet::Only(o)) => RuleSet::AllExcept(s | o),
            (RuleSet::All, RuleSet::AllExcept(o)) => RuleSet::Only(o.clone()),
            (RuleSet::Only(s), RuleSet::AllExcept(o)) => RuleSet::Only(s & o),
            (RuleSet::AllExcept(s), RuleSet::AllExcept(o)) => RuleSet::Only(o - s),
        }
    }

    /// `self` と `other` の両方に含まれるルールがあるか。
    fn overlaps(&self, other: &RuleSet) -> bool {
        !self.minus(&self.minus(other)).is_empty()
    }
}

/// 抑制の範囲。
#[derive(Debug, Clone)]
enum Region {
    File,
    Line(usize),
    /// 原文のバイト範囲 `[start, end)`。
    Range(usize, usize),
}

#[derive(Debug, Clone)]
struct Rule {
    region: Region,
    rules: RuleSet,
    reason: Option<String>,
    /// 抑制コメントの行。
    line: usize,
}

/// 文書の抑制コメントを診断に適用し、警告を返す。
pub fn apply(doc: &Document, names: &RuleNames, diagnostics: &mut [Diagnostic]) -> Vec<String> {
    let mut warnings = Vec::new();
    let rules = collect(doc, names, &mut warnings);
    if rules.is_empty() {
        return warnings;
    }
    for d in diagnostics.iter_mut() {
        let line = doc.lines.line(d.span.start);
        if let Some(rule) = rules.iter().find(|r| {
            r.rules.contains(&d.rule_id)
                && match r.region {
                    Region::File => true,
                    Region::Line(l) => l == line,
                    Region::Range(start, end) => start <= d.span.start && d.span.start < end,
                }
        }) {
            d.suppressed = Some(Suppression {
                reason: rule.reason.clone(),
                line: rule.line,
            });
        }
    }
    warnings
}

fn collect(doc: &Document, names: &RuleNames, warnings: &mut Vec<String>) -> Vec<Rule> {
    let mut rules = Vec::new();
    // 開いている noslop-disable (対象, 開始位置, 理由, 行)
    let mut open: Vec<(RuleSet, usize, Option<String>, usize)> = Vec::new();
    for directive in &doc.directives {
        let line = doc.lines.line(directive.span.start);
        let set = resolve_set(&directive.rules, names, line, warnings);
        match directive.kind {
            DirectiveKind::DisableFile => rules.push(Rule {
                region: Region::File,
                rules: set,
                reason: directive.reason.clone(),
                line,
            }),
            DirectiveKind::DisableLine => rules.push(Rule {
                region: Region::Line(line),
                rules: set,
                reason: directive.reason.clone(),
                line,
            }),
            DirectiveKind::DisableNextLine => {
                let end_line = doc.lines.line(directive.span.end.saturating_sub(1));
                rules.push(Rule {
                    region: Region::Line(end_line + 1),
                    rules: set,
                    reason: directive.reason.clone(),
                    line,
                });
            }
            DirectiveKind::Disable => {
                open.push((set, directive.span.end, directive.reason.clone(), line));
            }
            DirectiveKind::Enable => {
                let mut closed_any = false;
                let mut still_open = Vec::new();
                for (open_set, start, reason, open_line) in open.drain(..) {
                    if open_set.overlaps(&set) {
                        closed_any = true;
                        rules.push(Rule {
                            region: Region::Range(start, directive.span.start),
                            rules: open_set.clone(),
                            reason: reason.clone(),
                            line: open_line,
                        });
                        let rest = open_set.minus(&set);
                        if !rest.is_empty() {
                            still_open.push((rest, directive.span.end, reason, open_line));
                        }
                    } else {
                        still_open.push((open_set, start, reason, open_line));
                    }
                }
                open = still_open;
                if !closed_any {
                    warnings.push(format!(
                        "L{line}: 対応する noslop-disable がない noslop-enable を無視しました"
                    ));
                }
            }
        }
    }
    for (set, start, reason, line) in open {
        rules.push(Rule {
            region: Region::Range(start, usize::MAX),
            rules: set,
            reason,
            line,
        });
    }
    rules
}

fn resolve_set(
    keys: &[String],
    names: &RuleNames,
    line: usize,
    warnings: &mut Vec<String>,
) -> RuleSet {
    if keys.is_empty() {
        return RuleSet::All;
    }
    let mut set = BTreeSet::new();
    for key in keys {
        match names.resolve(key) {
            Some(id) => {
                set.insert(id.to_string());
            }
            None => warnings.push(format!(
                "L{line}: 抑制コメントの未知のルール「{key}」を無視しました"
            )),
        }
    }
    RuleSet::Only(set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::{Lane, RuleStatus, Severity, Span};

    fn names() -> RuleNames {
        RuleNames::new([("P01", "AI_CONCLUSION"), ("R03", "LONG_SENTENCE")])
    }

    /// `needle` の位置に rule の診断を置く。
    fn diag_at(src: &str, needle: &str, rule: &str) -> Diagnostic {
        let start = src
            .find(needle)
            .unwrap_or_else(|| panic!("{needle} not in source"));
        Diagnostic::new(
            rule,
            rule,
            Severity::Warning,
            Lane::Slop,
            RuleStatus::Stable,
            Span::new(start, start + needle.len()),
            "m",
        )
    }

    fn run(src: &str, targets: &[(&str, &str)]) -> (Vec<bool>, Vec<String>) {
        let doc = Document::markdown(src);
        let mut diags: Vec<_> = targets.iter().map(|(n, r)| diag_at(src, n, r)).collect();
        let warnings = apply(&doc, &names(), &mut diags);
        (
            diags.iter().map(Diagnostic::is_suppressed).collect(),
            warnings,
        )
    }

    #[test]
    fn next_line_targets_only_the_following_physical_line() {
        let src =
            "<!-- noslop-disable-next-line P01 -- 引用なので残す -->\n対象の一文。\n次の一文。\n";
        let (s, w) = run(src, &[("対象", "P01"), ("次の", "P01"), ("対象", "R03")]);
        assert_eq!(s, vec![true, false, false]);
        assert!(w.is_empty());
        let doc = Document::markdown(src);
        let mut d = vec![diag_at(src, "対象", "P01")];
        apply(&doc, &names(), &mut d);
        assert_eq!(
            d[0].suppressed,
            Some(Suppression {
                reason: Some("引用なので残す".into()),
                line: 1
            })
        );
    }

    #[test]
    fn next_line_uses_the_line_where_the_comment_ends() {
        let src = "<!--\nnoslop-disable-next-line\n-->\n対象。\n";
        // 複数行コメントは HTML ブロックとして読まれる
        let (s, _) = run(src, &[("対象", "P01")]);
        assert_eq!(s, vec![true]);
    }

    #[test]
    fn same_line_directive() {
        let src = "対象の一文。<!-- noslop-disable-line AI_CONCLUSION -->\n次の一文。\n";
        let (s, _) = run(src, &[("対象", "P01"), ("次の", "P01")]);
        assert_eq!(s, vec![true, false]);
    }

    #[test]
    fn range_is_closed_by_enable_and_partial_enable_keeps_others() {
        let src = "前。\n\n<!-- noslop-disable -->\n\n中の一文。\n\n<!-- noslop-enable P01 -->\n\n後の一文。\n\n<!-- noslop-enable -->\n\n最後。\n";
        let (s, w) = run(
            src,
            &[
                ("前", "P01"),
                ("中の", "P01"),
                ("中の", "R03"),
                ("後の", "P01"),
                ("後の", "R03"),
                ("最後", "R03"),
            ],
        );
        assert_eq!(s, vec![false, true, true, false, true, false]);
        assert!(w.is_empty(), "{w:?}");
    }

    #[test]
    fn unclosed_disable_runs_to_the_end_and_file_directive_covers_everything() {
        let src = "前。\n\n<!-- noslop-disable R03 -->\n\n後。\n";
        let (s, _) = run(src, &[("前", "R03"), ("後", "R03"), ("後", "P01")]);
        assert_eq!(s, vec![false, true, false]);
        let src = "本文。\n\n<!-- noslop-disable-file -->\n";
        let (s, _) = run(src, &[("本文", "P01"), ("本文", "R03")]);
        assert_eq!(s, vec![true, true]);
    }

    #[test]
    fn unknown_rules_and_stray_enable_become_warnings() {
        let src = "<!-- noslop-disable-next-line NOPE -->\n対象。\n\n<!-- noslop-enable -->\n";
        let (s, w) = run(src, &[("対象", "P01")]);
        assert_eq!(
            s,
            vec![false],
            "未知のルールだけの指定は全ルールの抑制にしない"
        );
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].contains("NOPE"));
        assert!(w[1].contains("noslop-enable"));
    }

    #[test]
    fn rule_set_algebra() {
        let p = RuleSet::Only(["P01".to_string()].into());
        assert_eq!(
            RuleSet::All.minus(&p),
            RuleSet::AllExcept(["P01".to_string()].into())
        );
        assert!(RuleSet::All.minus(&p).contains("R03"));
        assert!(!RuleSet::All.minus(&p).contains("P01"));
        assert!(p.minus(&RuleSet::All).is_empty());
        assert!(RuleSet::All.overlaps(&p));
        assert!(!RuleSet::Only(["R03".to_string()].into()).overlaps(&p));
    }
}
