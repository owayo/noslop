//! ルール一覧と説明の描画 (CLI と MCP で共有する)。

use crate::diagnostic::{Severity, Signal};
use crate::engine::{Engine, RuleEntry};

use super::pad_end;

/// 手掛かりの強さの説明 (`noslop explain` と生成するルールの一覧で使う)。
fn signal_label(signal: Signal) -> &'static str {
    match signal {
        Signal::Strong => "強い (人の文書にほとんど出ず、生成文書に偏って出る)",
        Signal::Moderate => {
            "中程度 (生成文書に多いが人の文書にも出る。件数が少なく測れていないものも含む)"
        }
        Signal::Weak => "弱い (人の文書にも同じくらいかそれ以上に出る)",
    }
}

/// 校正の基準 (人の文書での誤検知率を、どの重大度以上の指摘で数えるか)。
///
/// 組み込みルールだけに付ける (独自ルールは noslop のコーパスで測っていない)。
fn calibration_basis(entry: &RuleEntry) -> Option<Severity> {
    entry.builtin.then(|| entry.rule.calibration_basis())
}

fn calibration_basis_label(basis: Severity) -> &'static str {
    match basis {
        Severity::Info => "重大度を問わず数えた誤検知率",
        Severity::Warning => "警告以上の指摘で数えた誤検知率",
        Severity::Error => "重大の指摘で数えた誤検知率",
    }
}

pub(crate) fn rules_text(engine: &Engine) -> String {
    let genre = engine.options().genre;
    let mut s = format!("ジャンル: {genre}\n\n");
    let header = format!(
        "{} {} {} {} {} {} {}\n",
        pad_end("ID", 5),
        pad_end("名前", 26),
        pad_end("レーン", 12),
        pad_end("強さ", 7),
        pad_end("重大度", 8),
        pad_end("有効", 5),
        "日本語名"
    );
    s.push_str(&header);
    for e in engine.entries() {
        let m = e.rule.meta();
        s.push_str(&format!(
            "{} {} {} {} {} {} {}\n",
            pad_end(m.id, 5),
            pad_end(m.name, 26),
            pad_end(m.lane.as_str(), 12),
            pad_end(m.signal.map_or("-", Signal::level_ja), 7),
            pad_end(e.severity.unwrap_or(m.default_severity).as_str(), 8),
            pad_end(if e.enabled { "有効" } else { "-" }, 5),
            m.title
        ));
        s.push_str(&format!("      {}\n", m.summary));
    }
    if engine.entries().is_empty() {
        s.push_str("(ルールがありません)\n");
    }
    s
}

pub(crate) fn rules_json(engine: &Engine) -> String {
    let rows: Vec<serde_json::Value> = engine
        .entries()
        .iter()
        .map(|e| {
            let m = e.rule.meta();
            let options: serde_json::Map<String, serde_json::Value> = e
                .rule
                .options()
                .into_iter()
                .map(|(k, v)| (k.to_string(), serde_json::Value::String(v)))
                .collect();
            serde_json::json!({
                "id": m.id,
                "name": m.name,
                "title": m.title,
                "lane": m.lane,
                "signal": m.signal,
                "defaultSeverity": m.default_severity,
                "severity": e.severity.unwrap_or(m.default_severity),
                "enabled": e.enabled,
                "builtin": e.builtin,
                "allowedInGenre": e.rule.allowed_in(engine.options().genre),
                "calibrationBasis": calibration_basis(e),
                "summary": m.summary,
                "options": options,
            })
        })
        .collect();
    let v = serde_json::json!({
        "genre": engine.options().genre.as_str(),
        "rules": rows,
    });
    let mut s = serde_json::to_string_pretty(&v).expect("serialize rules");
    s.push('\n');
    s
}

pub(crate) fn rules_markdown(engine: &Engine) -> String {
    let genre = engine.options().genre;
    let mut s = String::from("# noslop のルール\n\n");
    s.push_str(
        "このファイルは `noslop rules --format markdown --no-config` で生成しています。\n\n",
    );
    s.push_str(&format!("ジャンル `{genre}` での既定の状態です。\n\n"));
    s.push_str("| ID | 名前 | 日本語名 | レーン | 手掛かりの強さ | 既定の重大度 | 既定で有効 |\n");
    s.push_str("|---|---|---|---|---|---|---|\n");
    for e in engine.entries() {
        let m = e.rule.meta();
        s.push_str(&format!(
            "| [{id}](#{anchor}) | `{name}` | {title} | {lane} | {signal} | {sev} | {enabled} |\n",
            id = m.id,
            anchor = m.id.to_ascii_lowercase(),
            name = m.name,
            title = m.title,
            lane = m.lane.as_str(),
            signal = m.signal.map_or("-", Signal::level_ja),
            sev = m.default_severity.as_str(),
            enabled = if e.enabled { "✓" } else { "" },
        ));
    }
    for e in engine.entries() {
        let m = e.rule.meta();
        s.push_str(&format!("\n## {}\n\n", m.id));
        s.push_str(&format!("**{}** — {}\n\n", m.name, m.title));
        s.push_str(&format!("- レーン: {}\n", m.lane.label_ja()));
        if let Some(signal) = m.signal {
            s.push_str(&format!("- 手掛かりの強さ: {}\n", signal_label(signal)));
        }
        s.push_str(&format!("- 既定の重大度: {}\n", m.default_severity));
        if let Some(basis) = calibration_basis(e) {
            s.push_str(&format!(
                "- 校正の基準: {}\n",
                calibration_basis_label(basis)
            ));
        }
        let options = e.rule.options();
        if !options.is_empty() {
            let list: Vec<String> = options
                .iter()
                .map(|(k, v)| format!("`{k} = {v}`"))
                .collect();
            s.push_str(&format!("- 設定項目: {}\n", list.join(", ")));
        }
        s.push_str(&format!("\n{}\n", m.summary));
        if !m.explanation.trim().is_empty() {
            s.push_str(&format!(
                "\n{}\n",
                demote_headings(m.explanation.trim_end())
            ));
        }
    }
    s
}

/// 説明文の見出しを `###` 以下にそろえる (ルールごとの節 `##` の下に入れるため)。
fn demote_headings(text: &str) -> String {
    text.lines()
        .map(|line| {
            let hashes = line.chars().take_while(|&c| c == '#').count();
            if (1..3).contains(&hashes) && line[hashes..].starts_with(' ') {
                format!("###{}", &line[hashes..])
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn explain_text(engine: &Engine, entry: &RuleEntry) -> String {
    let m = entry.rule.meta();
    let genre = engine.options().genre;
    let mut s = format!("{} {} — {}\n\n", m.id, m.name, m.title);
    s.push_str(&format!("  レーン      : {}\n", m.lane.label_ja()));
    if !entry.builtin {
        s.push_str("  定義        : 設定ファイルで定義した独自ルール (既定で有効)\n");
    }
    if let Some(signal) = m.signal {
        s.push_str(&format!("  手掛かり    : {}\n", signal_label(signal)));
    }
    s.push_str(&format!("  既定の重大度: {}\n", m.default_severity));
    if let Some(sev) = entry.severity {
        s.push_str(&format!("  設定の重大度: {sev}\n"));
    }
    if let Some(basis) = calibration_basis(entry) {
        s.push_str(&format!(
            "  校正の基準  : {}\n",
            calibration_basis_label(basis)
        ));
    }
    let allowed = entry.rule.allowed_in(genre);
    let state = if entry.enabled {
        "有効".to_string()
    } else if !allowed {
        "無効 (このジャンルの慣習と衝突するため既定で止めている)".to_string()
    } else {
        "無効".to_string()
    };
    s.push_str(&format!("  ジャンル    : {genre} では{state}\n"));
    let options = entry.rule.options();
    if !options.is_empty() {
        s.push_str("  設定項目    :\n");
        for (k, v) in options {
            s.push_str(&format!("    {k} = {v}\n"));
        }
    }
    s.push_str(&format!("\n{}\n", m.summary));
    if !m.explanation.trim().is_empty() {
        s.push_str(&format!("\n{}\n", m.explanation.trim_end()));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EngineOptions;

    #[test]
    fn builtin_rules_show_their_calibration_basis_and_signal() {
        let engine = Engine::new(EngineOptions::default()).unwrap();
        let v: serde_json::Value = serde_json::from_str(&rules_json(&engine)).unwrap();
        let basis = |id: &str| {
            v["rules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|r| r["id"] == id)
                .unwrap_or_else(|| panic!("{id}"))["calibrationBasis"]
                .clone()
        };
        assert_eq!(basis("P01"), "warning");
        assert_eq!(basis("R06"), "info", "既定の重大度が情報のルールは情報");
        assert_eq!(basis("R05"), "error", "重大の率で校正した");
        assert_eq!(basis("S01"), "info", "どの組み込みルールも校正の基準を持つ");

        let r05 = engine.find("R05").unwrap();
        let text = explain_text(&engine, r05);
        assert!(
            text.contains("校正の基準  : 重大の指摘で数えた誤検知率"),
            "{text}"
        );
        assert!(text.contains("手掛かり    : 強い ("), "{text}");
        // 読みやすさのルールは手掛かりの強さを持たない
        let r03 = engine.find("R03").unwrap();
        assert!(!explain_text(&engine, r03).contains("手掛かり    :"));
        let listing = rules_text(&engine);
        let header = listing.lines().nth(2).unwrap();
        assert!(
            header.contains("強さ") && !header.contains("状態"),
            "{header}"
        );
        let p03 = listing.lines().find(|l| l.starts_with("P03 ")).unwrap();
        assert!(p03.contains(" 弱い ") && p03.contains("有効"), "{p03}");
    }

    #[test]
    fn markdown_headings_are_demoted_under_rule_sections() {
        assert_eq!(
            demote_headings("# 見出し\n## 小見出し\n### そのまま\n本文"),
            "### 見出し\n### 小見出し\n### そのまま\n本文"
        );
    }
}
