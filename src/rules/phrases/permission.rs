//! P28: 行動の勧めや手順を、許可の形で語る言い回し。

use regex::Regex;
use std::sync::LazyLock;

use super::engine::{WEAK_SIGNAL_NOTE, diagnostic};
use crate::diagnostic::{Diagnostic, Lane, RuleStatus, Severity};
use crate::rules::{Rule, RuleContext, RuleMeta, RuleUnit, quote};
use crate::text;

static META: RuleMeta = RuleMeta {
    id: "P28",
    name: "PERMISSIVE_ACTION",
    title: "許可の形で語る行動",
    lane: Lane::Readability,
    status: RuleStatus::Experimental,
    default_severity: Severity::Info,
    summary: "「〜て（も）かまいません」「〜て（も）構わない」などの行動を直接書けるか見直す (実験的)",
    explanation: r"### 何を見るか

「〜て（も）かまいません」「〜て（も）構いません」「〜て（も）かまわない」「〜て（も）構わない」と、後ろの 2 つに「でしょう」が続く形を探します。「読んでも」「急いでも」のように、漢字に「ん」と「で」が続く形や、急ぐ・泳ぐなど限られた漢字に「いで」が続く形も含みます。

「空欄のままでも構いません」「どちらでも構いません」のような状態や選択肢は拾いません。「高くても」「しなくても」のように「くて」が続く形と、「〜として」「〜であって」の状態、「すべて」「かつて」の末尾、状態を表す「〜ていても」「がいても」「にいても」「あっても」「くなっても」「になっても」「となっても」、慣用句の「まったくもって」、同じ節に「どれ」「どんな」「何」「いつ」「いずれ」などの不定語がある形、質問・埋め込み疑問・直後の否定も除きます。「みんな」「どうぞ」「などの」「先ほどの」「思いつく」などは不定語から外します。辞書を使わず直前の文字で近似するため、仮名だけの「つまんでも」や許可した漢字の外にある「揺らいでも」や、状態と区別しにくい「横になっても」「ご覧になっても」や、仮名の「もがいても」などを取りこぼすことがあります。不定語の区切りも表記で近似します。この除外以外の述語の意味は判別しません。有効にしたときの既定の対象は地の文の段落です。リスト・表・引用は語句ルールのスコープ設定で対象にできます。

### なぜ問題か

勧めや手順を許可の形にすると、何をすればよいかが曖昧になることがあります。ただし、選ぶ余地を明示する場合や、実際に許可を伝える場合には必要です。正当な許可まで誤りと断定せず、読みやすさの弱い手掛かりとして扱います。

### 直し方

勧めなら勧めとして、手順なら手順として、することをそのまま書けるか確かめます。任意の行動を必須に変えたり、選択肢を消したりしないでください。原文にない目的や効果は足さず、必要な許可は残せます。

### 例

- 直す前: 確認した内容をその場で追記すると、次の人が状況を読めます。追記しても構いません。
- 直した後: 確認した内容をその場で追記すると、次の人が状況を読めます。追記することを勧めます。

### 根拠

人の文書 171 本中 2 本 (3 件)・生成文書 390 本中 7 本 (8 件) に反応しました。人の文章で本来の許可として使われる型でもあり、反応率の差も小さいため、実験的な情報の指摘にとどめます。文脈上の勧めと許可は判別できず、既定で有効にする前に、人の文書での出方を追加で校正します。",
};

static PERMISSION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[てで]も?(?:かま|構)(?:いません|わない(?:でしょう)?)").expect("行動を許可する文末")
});

/// 「〜いで」を行動として数える漢字。払い・扱いなどの名詞を拾わない。
const GU_STEMS: &str = "急泳脱稼防注繋継仰剥騒研漕嗅担塞紡凌削寛";
static CHOICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:どれ|どちら|どっち|どこ|どの|どんな|どう|だれ|誰|なに|何|いくら|いくつ|いずれ|いかなる|いつ(?:[^も、，,。]|$))[^、，,。]*$")
        .expect("選択肢の不定語")
});

static NOT_CHOICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"みんな|[こそあ]んな|どうぞ|どうしても|何らか|何卒|[なほ]どの|ほどこ|[思追]いつ")
        .expect("不定語でない語")
});

/// 接続の直前の字から、行動らしい形だけを残す。
fn action_before(prefix: &str, connector: char) -> bool {
    let mut chars = prefix.chars().rev();
    let Some(last) = chars.next() else {
        return false;
    };
    if connector == 'で' {
        return match last {
            'ん' => chars.next().is_some_and(text::is_kanji),
            'い' => chars.next().is_some_and(|c| GU_STEMS.contains(c)),
            _ => false,
        };
    }
    if last == 'く'
        || prefix.ends_with('全')
        || [
            "すべ",
            "かつ",
            "あっ",
            "だっ",
            "てい",
            "でい",
            "くなっ",
            "になっ",
            "となっ",
            "にい",
            "がい",
            "くもっ",
        ]
        .iter()
        .any(|e| prefix.ends_with(e))
        || (prefix.ends_with("とし") && !prefix.ends_with("落とし"))
    {
        return false;
    }
    text::is_hiragana(last) || text::is_kanji(last)
}

/// 質問や否定された許可は除く。理由の「から」と推量の「かもしれない」は残す。
fn excluded_after(suffix: &str) -> bool {
    let suffix = suffix.trim_start();
    let suffix = suffix
        .strip_prefix('の')
        .or_else(|| suffix.strip_prefix('ん'))
        .unwrap_or(suffix);
    let suffix = ["でしょう", "です", "だろう"]
        .iter()
        .find_map(|p| suffix.strip_prefix(p))
        .unwrap_or(suffix);
    if suffix.starts_with(['？', '?']) {
        return true;
    }
    if let Some(rest) = suffix.strip_prefix('か') {
        return !(rest.starts_with('ら')
            || rest.starts_with("もしれ")
            || rest.starts_with("も知れ"));
    }
    [
        "わけでは",
        "わけじゃ",
        "訳では",
        "とは限ら",
        "とは言えな",
        "とは言えま",
        "とはいえな",
        "とはいえま",
        "ということでは",
        "というわけでは",
    ]
    .iter()
    .any(|p| suffix.starts_with(p))
}

pub(super) struct PermissiveAction;

impl Rule for PermissiveAction {
    fn unit(&self) -> RuleUnit {
        RuleUnit::Sentence
    }
    fn meta(&self) -> &'static RuleMeta {
        &META
    }
    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        for (idx, block) in ctx.scoped_blocks() {
            for sentence in ctx.doc.block_sentences(idx) {
                let value = &block.text[sentence.range.clone()];
                for hit in PERMISSION.find_iter(value) {
                    let matched = hit.as_str();
                    if !action_before(
                        &value[..hit.start()],
                        matched.chars().next().expect("接続の字"),
                    ) || CHOICE.is_match(&NOT_CHOICE.replace_all(&value[..hit.start()], "・"))
                        || excluded_after(&value[hit.end()..])
                    {
                        continue;
                    }
                    let span = block.to_source(
                        sentence.range.start + hit.start()..sentence.range.start + hit.end(),
                    );
                    let item = if matched.contains("いません") {
                        "かまいません"
                    } else {
                        "かまわない"
                    };
                    out.push(diagnostic(&META, span, sentence.span, matched,
                        format!("「{}」は行動を許可の形で語っています。勧めや手順を直接書けるか確かめてください。{WEAK_SIGNAL_NOTE}", quote(matched)),
                        "勧めや手順なら、することを直接書いてください。任意の行動を必須に変えず、必要な許可は残せます",
                        Severity::Info, RuleStatus::Experimental).with_metric("item", item));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, DocumentKind};
    use crate::engine::{Engine, EngineOptions, Selection};
    use crate::rules::{
        Scope,
        testing::{Options, matched, run, run_with},
    };

    #[test]
    fn finds_permissive_actions_with_original_positions() {
        for (md, expected) in [
            ("重要な項目は左へ寄せてかまいません。", "てかまいません"),
            ("記録はその場で追記しても構いません。", "ても構いません"),
            (
                "締切を翌週にずらしてもかまわないでしょう。",
                "てもかまわないでしょう",
            ),
            ("細かい表記は判断で変えて構わない。", "て構わない"),
            ("みんなに共有しても構いません。", "ても構いません"),
            ("そんなに急いでも構わない。", "でも構わない"),
            ("どうぞ追記しても構いません。", "ても構いません"),
            ("追記しても構わないのです。", "ても構わない"),
            ("下位を落としても構いません。", "ても構いません"),
            ("ファイルを繋いでも構いません。", "でも構いません"),
            ("ログなどの記録を追記しても構いません。", "ても構いません"),
            ("先ほどの内容を追記しても構いません。", "ても構いません"),
            ("思いついたら追記しても構いません。", "ても構いません"),
        ] {
            let findings = run(&PermissiveAction, md);
            assert_eq!(matched(md, &findings), [expected], "{md}");
            assert_eq!(findings[0].lane, Lane::Readability);
            assert_eq!(findings[0].severity, Severity::Info);
            assert_eq!(findings[0].status, RuleStatus::Experimental);
            assert!(findings[0].message.contains(WEAK_SIGNAL_NOTE));
        }
        let md = "前置きです。読んでもかまいません。\n\n急いでも構わない。";
        let findings = run(&PermissiveAction, md);
        assert_eq!(matched(md, &findings), ["でもかまいません", "でも構わない"]);
        assert_eq!(
            findings[0].metrics["item"],
            crate::diagnostic::Metric::Text("かまいません".into())
        );
        assert_eq!(
            findings[1].metrics["item"],
            crate::diagnostic::Metric::Text("かまわない".into())
        );
        for phrase in [
            "てかまいません",
            "てもかまいません",
            "て構いません",
            "ても構いません",
            "てかまわない",
            "てもかまわないでしょう",
            "て構わない",
            "ても構わないでしょう",
        ] {
            let md = format!("確認し**{phrase}**。");
            assert_eq!(matched(&md, &run(&PermissiveAction, &md)), [phrase]);
        }
    }

    #[test]
    fn excludes_states_choices_adjectives_questions_and_inline_code() {
        for md in [
            "どんな順番で並べても構いません。",
            "いずれの形式で書いても構いません。",
            "まったくもって構いません。",
            "全くもって構わない。",
            "人がいても構いません。",
            "結果がゼロとなっても構いません。",
            "追記しても構わないとは言えません。",
            "追記しても構いませんでしょうか。",
            "追記しても構わないですか。",
            "追記しても構わないのですか。",
            "追記しても構わないのか。",
            "追記しても構わない？",
            "エラーになっても構いません。",
            "空欄になっても構いません。",
            "ここにいても構いません。",
            "表示されていても構いません。",
            "空欄があっても構いません。",
            "長くなっても構いません。",
            "誰だって構わない。",
            "現金払いでも構いません。",
            "例外扱いでも構いません。",
            "どれを選んでも構いません。",
            "いつ提出しても構いません。",
            "追記して構わないか確認します。",
            "削除して構わないわけではありません。",
            "空欄のままでも構いません。",
            "どちらでも構いません。",
            "なんでも構わない。",
            "空欄で構いません。",
            "きれいでも構いません。",
            "高くても構わない。",
            "確認しなくても構いません。",
            "空欄としても構いません。",
            "空欄であってもかまいません。",
            "すべて構わない。",
            "全て構いません。",
            "かつて構いません。",
            "追記しても構いませんか？",
            "変更して構わないか。",
            "構いません。",
            "`追記しても構いません` は例です。",
        ] {
            assert!(run(&PermissiveAction, md).is_empty(), "{md}");
        }
        assert_eq!(
            run(
                &PermissiveAction,
                "追記しても構いませんから、必要な内容を確認する。"
            )
            .len(),
            1
        );
    }

    #[test]
    fn respects_scopes_and_opt_in_for_prose_and_fragments() {
        let md = "- 追記しても構いません。\n\n> 追記しても構いません。\n\n| 項目 | 手順 |\n| --- | --- |\n| 記録 | 追記しても構いません。 |\n";
        assert!(run(&PermissiveAction, md).is_empty());
        assert_eq!(
            run_with(
                &PermissiveAction,
                md,
                Options {
                    scope: Scope::ALL,
                    ..Options::default()
                }
            )
            .len(),
            3
        );
        for (experimental, explicit, expected) in
            [(false, false, 0), (true, false, 1), (false, true, 1)]
        {
            let engine = Engine::with_rules(
                vec![Box::new(PermissiveAction)],
                EngineOptions {
                    experimental,
                    selection: Selection {
                        cli_enable: if explicit { vec!["P28".into()] } else { vec![] },
                        ..Selection::default()
                    },
                    ..EngineOptions::default()
                },
            )
            .unwrap();
            let mut doc = Document::plain_text("追記しても構いません。");
            assert_eq!(engine.lint(doc.clone()).diagnostics.len(), expected);
            doc.kind = DocumentKind::Fragments;
            assert_eq!(engine.lint(doc).diagnostics.len(), expected);
        }
    }
}
