//! S13: 本文全体が Markdown を包む 1 つのコードフェンスに入っている。

use crate::diagnostic::{Diagnostic, Lane, Severity, Signal};
use crate::rules::{Rule, RuleContext, RuleMeta, quote};

static META: RuleMeta = RuleMeta {
    id: "S13",
    name: "WRAPPED_DOCUMENT",
    title: "本文全体を包むコードフェンス",
    lane: Lane::Slop,
    signal: Some(Signal::Strong),
    default_severity: Severity::Warning,
    summary: "本文全体が「```markdown」のようなコードフェンスに入っていて、Markdown として表示されない",
    explanation: "\
### 何を見るか

Markdown の文書で、本文全体 (先頭の front matter と前後の空行を除いた部分) が 1 つのコードフェンスに\
入っている形を指します。開きのフェンスが行頭から始まり、情報文字列の最初の語が `markdown` か `md` \
(大文字小文字を問わない) のものに限ります。開きのフェンスの行に 1 件出し、閉じのフェンスの行を関連箇所に\
添えます。閉じのフェンスがなく文書の終わりまで続く形 (閉じの記号が最後の行の末尾に付いていて、閉じに\
ならない形を含む) も指します。

中に外側と同じ長さのフェンスでコードブロックを入れ子にした形も指します。CommonMark では内側の閉じの\
フェンスで外側も閉じますが、最後の行が外側を閉じるフェンスで、その間のフェンスがすべて対になり、言語を\
指定したコードブロックを 1 つ以上含むときは、本文全体を包んだものとして扱います。先頭に Markdown の\
見本を置いて本文とコードを続けた文書では、間のフェンスが対にならないので指しません。言語の指定のない\
フェンス、字下げしたフェンス、前後に別の段落・見出し・参照リンクの定義があるフェンスも指しません。

フェンスの中身は、このルールを止めても Markdown の本文として検査します。中身にあるコードブロックは、\
これまでどおりコードとして読み飛ばします。

### なぜ問題か

チャットで「Markdown で書いて」と頼むと、応答の全体がコードブロックで返ることがあります。それをそのまま\
貼った文書は、Markdown として表示すると本文全体が等幅のコードになり、見出しも段落も効きません。文書の\
全体を Markdown の書き方の見本として載せる場合を除けば、生成した応答を貼ったときに残る形です。

### 直し方

先頭と末尾のフェンスの行を消してください。閉じのフェンスがない形では、先頭の行と、最後の行の末尾に\
残った閉じの記号を消します。書き方の見本として載せているなら、そのままでかまいません。

### 例

- 直す前: 1 行目が「```markdown」、最後の行が「```」で、その間に見出しと本文がある
- 直した後: 1 行目と最後の行を消し、見出しから始まる Markdown にする

### 根拠

人の文書 440 本 (Web の解説・技術記事・業務文書・随筆と、書式を保った技術記事・一般向けの記事) には\
出ず、2026 年のモデルの生成文書 2,576 本では 116 本 (4.5%) に出ました。閉じのフェンスがない 2 本と、\
中のフェンスで外側が閉じる入れ子の形の 12 本を含みます。出たのは OpenAI のモデル (gpt-oss の 3 本を含む) \
の文書だけで、生成の指示は「本文だけを Markdown で出力してください」でした。2025 年頃のモデルなどの生成\
文書 390 本には出ないので、人の文書 440 本と生成文書 390 本で手掛かりの強さを測る標準の測定では、件数が\
足りず測れません。言語の指定のないフェンスで本文全体を包んだ生成文書も 9 本ありましたが、コード例や\
ログを載せた人の文書と見分けられるかを確かめていないので指しません。

手掛かりの強さは「強い」です。人の文書 440 本と 2026 年のモデルの生成文書 2,576 本で測ると、指摘が出た\
文書は人 0 本・生成 116 本で、生成の割合が人の 3 倍以上・人の割合が 5% 以下という強い手掛かりの基準を\
満たします。生成した応答をそのまま貼った形そのものを照合する仕組みでもあります。
",
};

/// S13 WRAPPED_DOCUMENT。
pub struct WrappedDocument;

impl Rule for WrappedDocument {
    fn meta(&self) -> &'static RuleMeta {
        &META
    }

    fn check(&self, ctx: &RuleContext<'_>, out: &mut Vec<Diagnostic>) {
        // 包むフェンスは Markdown の文書の読み込みでだけ見つける (テキスト・コードは常に None)
        let Some(wrapper) = &ctx.doc.body.wrapper else {
            return;
        };
        let opening = quote(ctx.doc.slice(wrapper.opening));
        let (message, hint) = match wrapper.closing {
            Some(_) => (
                format!("本文全体が「{opening}」のコードフェンスに入っています"),
                "先頭と末尾のフェンスの行を消して、本文を Markdown として表示されるようにしてください",
            ),
            None => (
                format!(
                    "本文全体が「{opening}」のコードフェンスに入り、閉じのフェンスがないまま文書の終わりまで続いています"
                ),
                "先頭のフェンスの行と、最後の行の末尾に残った閉じの記号を消してください",
            ),
        };
        let mut related = vec![wrapper.opening];
        related.extend(wrapper.closing);
        out.push(
            META.diagnostic(wrapper.opening, message)
                .with_hint(hint)
                .with_related(related),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::Severity;
    use crate::document::Document;
    use crate::rules::testing::{matched, run};

    #[test]
    fn flags_the_opening_fence_of_a_wrapped_document() {
        let source = "```markdown\n# 題\n\n本文です。\n```\n";
        let found = run(&WrappedDocument, source);
        assert_eq!(matched(source, &found), ["```markdown"]);
        let d = &found[0];
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.signal, Some(Signal::Strong));
        assert_eq!(d.lane, Lane::Slop);
        let related: Vec<&str> = d.related.iter().map(|s| &source[s.range()]).collect();
        assert_eq!(related, ["```markdown", "```"]);
        assert!(d.message.contains("「```markdown」"), "{}", d.message);
    }

    #[test]
    fn unclosed_and_tilde_fences_after_front_matter_are_flagged() {
        let unclosed = "```md\n# 題\n\n本文です。```";
        let found = run(&WrappedDocument, unclosed);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].related.len(), 1);
        assert!(found[0].message.contains("閉じのフェンスがない"));

        let tilde = "---\ntitle: メモ\n---\n\n~~~Markdown\n本文です。\n~~~\n";
        let found = run(&WrappedDocument, tilde);
        assert_eq!(matched(tilde, &found), ["~~~Markdown"]);
    }

    #[test]
    fn nested_fences_that_close_the_wrapper_early_are_flagged() {
        // 内側の ```sh の閉じで、CommonMark では外側も閉じる
        let source = "```markdown\n# 題\n\n本文です。\n\n```sh\nls\n```\n\n続きです。\n```\n";
        let found = run(&WrappedDocument, source);
        assert_eq!(matched(source, &found), ["```markdown"]);
        // 閉じの関連箇所は、内側の閉じではなく最後の行
        let closing = found[0].related[1];
        assert_eq!(&source[closing.range()], "```");
        assert_eq!(closing.start, source.rfind("```").unwrap());
        assert!(
            found[0]
                .message
                .contains("「```markdown」のコードフェンスに入っています")
        );
    }

    #[test]
    fn other_fences_are_not_flagged() {
        for source in [
            // 言語の指定がない・別の言語
            "```\n本文です。\n```\n",
            "```rust\nfn main() {}\n```\n",
            // 字下げしたフェンス
            "  ```markdown\n  本文です。\n  ```\n",
            // 前後に段落・参照リンクの定義がある
            "以下が記事です。\n\n```markdown\n本文です。\n```\n",
            "```markdown\n本文です。\n```\n\n以上です。\n",
            "```markdown\n本文です。\n```\n\n[x]: https://example.com\n",
            // 先頭に見本を置き、本文とコードが続く (間のフェンスが対にならない)
            "```markdown\n# 見本\n```\n\n本文です。\n\n```sh\nls\n```\n",
            // 間のフェンスは対になるが、言語を指定したコードブロックがない
            "```markdown\n# 見本\n```\nコード\n```\n本文です。\n```\n",
            // 中身が空
            "```markdown\n```\n",
            // 普通の文書
            "# 題\n\n本文です。\n\n```markdown\n# 見本\n```\n",
        ] {
            assert!(run(&WrappedDocument, source).is_empty(), "{source:?}");
        }
    }

    #[test]
    fn text_documents_are_not_flagged() {
        let doc = Document::plain_text("```markdown\n本文です。\n```\n");
        let mut out = Vec::new();
        WrappedDocument.check(&RuleContext::new(&doc), &mut out);
        assert!(out.is_empty());
    }
}
