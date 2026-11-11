//! 語句照合の比較用。文書の解析と照合器の構築を除き、P02 の検査を測る。

use std::hint::black_box;
use std::time::Instant;

use noslop::rules::{RuleContext, Scope, builtin_rules};
use noslop::{Document, Genre};

fn main() {
    let rules = builtin_rules(Genre::General);
    let rule = rules.iter().find(|rule| rule.meta().id == "P02").unwrap();
    println!("case,bytes,findings,sample,ns_per_check");
    for (name, text, expected, iterations) in [
        ("short", "さて、内容を紹介していきます。".into(), 2, 1000),
        (
            "paragraphs",
            "「さて、」という語を説明します。さて、続きを書きます。\n\n".repeat(100),
            100,
            100,
        ),
        (
            "long_quoted_200",
            format!("{}終わりです。", "「さて、」さて、".repeat(200)),
            200,
            50,
        ),
        (
            "long_quoted_2000",
            format!("{}終わりです。", "「さて、」さて、".repeat(2000)),
            2000,
            5,
        ),
    ] {
        let doc = Document::markdown(text);
        let ctx = RuleContext {
            doc: &doc,
            genre: Genre::General,
            scope: Scope::default(),
            morph: None,
        };
        let mut findings = Vec::new();
        rule.check(&ctx, &mut findings);
        assert_eq!(findings.len(), expected, "{name}");
        for sample in 0..10 {
            let started = Instant::now();
            for _ in 0..iterations {
                findings.clear();
                rule.check(black_box(&ctx), black_box(&mut findings));
                black_box(&findings);
            }
            // 最初の 3 標本はウォームアップとして捨てる。
            if sample >= 3 {
                println!(
                    "{name},{},{expected},{},{:.0}",
                    doc.source.len(),
                    sample - 3,
                    started.elapsed().as_nanos() as f64 / f64::from(iterations)
                );
            }
        }
    }
}
