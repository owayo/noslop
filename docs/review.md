# 目的と読者に沿った文章レビュー

[READMEに戻る](../README.md)

`noslop review`は文書の意味を18観点で確認する依頼を作り、返ってきた根拠を原文と照合します。意味の判断は判定器や編集者が行います。noslopが検証するのは検査条件、全観点の有無、引用と原文の一致です。意味レビューは確認候補として表示し、文書の点数やCIを失敗させる判定には使いません。

## 依頼を作る

```bash
noslop review guide.md --purpose "初めての利用者が申込みを完了する" \
  --audience "申込み方法をまだ知らない人" --format json > review-request.json
```

省略時はMarkdownです。既定では依頼を出力するだけで、判定器を呼びません。原文も変更しません。全文を渡してください。コードのコメントやセルなど独立した断片の集まりは受け付けません。目的・読者が不明なら、その条件を必要とする判断は`unknown`として残します。

必要情報などを指定する場合は`--profile reader.json`を使います。[プロファイルの例](../examples/review-profile.json):

```json
{
  "purpose": "初めての利用者が申込みを完了する",
  "audience": "申込み方法をまだ知らない人",
  "requiredInformation": ["対象者", "提出物", "提出先", "受付期限"],
  "knownTerms": ["申込み"],
  "suppliedFacts": {"受付期限": "11月30日", "提出先": "受付フォーム"}
}
```

語や数字が出ただけで必要情報の充足とせず、値や適用条件を読みます。必須項目や未指定の事実を推測で増やしません。`suppliedFacts`は書き手が提供した確認材料です。`--purpose`と`--audience`はプロファイルの値を上書きします。通常の`noslop.toml`とユーザー設定はレビューでは読みません。

## 18観点

| ID | 確認する内容 |
|---|---|
| `purpose_coverage` | 目的に必要な情報を確認できるか |
| `lead` | 要点と詳細、前提と操作の順序 |
| `relevance` | 主題に対する段落の役割 |
| `spacing` | 内容の区切りと空白 |
| `hierarchy` | 項目の親子関係・並列関係 |
| `grouping` | 本文・例・引用・条件の境界 |
| `headings` | 見出しから内容を選べるか |
| `argument` | 主張・事実・推論の対応 |
| `audience` | 読者が知る用語と前提 |
| `tone` | 事実・評価・感情の区別 |
| `analogy` | 具体例や比喩の働き |
| `sentence_size` | 一文に詰めた情報 |
| `boundaries` | 読点や並列表記と意味の切れ目 |
| `ambiguity` | 指示・修飾・否定・比較・例外の範囲 |
| `terminology` | 呼び名の定義と参照 |
| `redundancy` | 意味を保ったまま削れる箇所 |
| `flow` | 語順や音と読み進め方 |
| `politeness` | 相手と目的に合う丁寧さ |

字数や見出しの数を一律にそろえません。必要な概要、根拠、箇条書き、定義、注意の再掲を残します。画面上の空白・折り返し・見た目はMarkdownソースだけでは判断できないため、表示の確認は別に行います。

## 結果を検証する

依頼の`instructions`に従って結果JSONを作ります。全18観点に`needs_review`（確認候補）、`clear`（指摘なし）、`unknown`（判断不能）、`not_applicable`（対象外）のいずれかと理由を返します。`needs_review`には読み手への影響・書き手への問い・原文の根拠が必要です。`clear`にも根拠が必要です。

一つの観点の例です。実際の結果には全18観点が必要です。

```json
{
  "id": "purpose_coverage", "status": "needs_review",
  "reason": "申込みの案内に受付期限が書かれていません。",
  "readerImpact": "いつまでに申し込む必要があるか判断できません。",
  "question": "受付期限を案内に含めますか。",
  "evidence": [{"blockId": 0, "quote": "申込みを受け付けます。"}]
}
```

`blockId`は`document.blocks`の`id`です。`sourceText`から原文を引用してください。同じブロックで同じ引用が複数現れる場合だけ、`occurrence`に1始まりの出現順を指定します。数える対象は重ならない出現です。別ブロックの根拠は別の項目にします。

結果全体は`schemaVersion: 1`、依頼と同じ`requestHash`、`reviewer: {"name": "判定器名", "model": "モデル名または編集者"}`、`observations`と`coverage`の配列です。返答の型は依頼の`responseSchema`にあります。本文の書き換えは返しません。

`coverage`は依頼の`requiredInformation`の各IDについて`present`（あり）、`missing`（欠落候補）、`unknown`（判断不能）と理由を返します。`present`には原文の引用が必要です。欠落は原文に存在しないため、偽の引用を作らず、`searched: "full_document"`を付けて全文を確認した結果とします。

```json
{
  "id": "required_1", "status": "missing",
  "reason": "全文を確認しましたが受付期限が書かれていません。",
  "searched": "full_document", "evidence": []
}
```

必須情報の全項目への回答を検証し、欠落・判断不能があれば`purpose_coverage`を`clear`にできません。欠落がある場合の`purpose_coverage`の確認候補は引用を空にできます。各観点の`requires`にある目的・読者・必須情報が未指定の場合は、`unknown`か`not_applicable`だけを受け付けます。

```bash
noslop review guide.md --profile reader.json --response review-response.json
noslop review guide.md --profile reader.json --response review-response.json --format json
```

依頼時と同じ全文・プロファイル・CLIの目的と読者を渡してください。原文、目的、必須情報、観点、指示の変更はハッシュで区別します。存在しない引用、曖昧な出現位置、観点の抜け・重複、余分なフィールドはエラーです。検証したJSONの`citations`には原文のUTF-8バイト範囲が入り、Markdownには行番号が入ります。欠落を示す引用は、その情報を必要とする箇所の根拠です。引用の一致だけで欠落や因果の判断の正しさを保証するものではありません。

## 判定器とMCP

実行ファイルを明示すると、その判定器へ標準入力で依頼JSONを渡し、標準出力の結果JSONを検証します。

```bash
noslop review guide.md --profile reader.json --reviewer my-reviewer \
  --reviewer-arg=--model --reviewer-arg=chosen-model --timeout 120
```

引数はシェルで展開しません。タイムアウトや失敗ではプロセスグループ・ジョブオブジェクトの子孫も停止します。出力は8MiBまで、待ち時間は1〜600秒です。成功時の終了コードは0、入出力・書式・照合の失敗は2です。候補の件数では終了コードを変えません。外部送信先やモデルはnoslopが選びません。指定した判定器が原文を外部へ送るかどうかは、その実装によります。

MCPの`review`に`text`（全文）、`filename`、`profile`を渡すと依頼JSONが返ります。クライアント自身が意味を読み、同じ引数に`response`を追加して再度呼ぶと検証したJSONが返ります。MCPサーバーは判定器のコマンドを起動しません。

## 構造lint

```bash
noslop check guide.md --enable-rules S11,S12
```

- S11: 段落末尾の明示的な数の予告と、同じ親の直後のリストの直接項目数を照合します。子リストや項目内の段落は足しません。別の節・引用・脚注・コードを結び付けません。関連リストを文脈に含め、リスト側だけの差分でも指摘を残します。
- S12: 見出しから次の同じ階層以上の見出しまでが空白・コメントだけの節を示します。子の節、表、コードがある節は空としません。全文が対象の場合だけ実行します。

両ルールは未校正の`experimental`・`readability`・`info`です。既定のlintには追加しません。意味レビューと構造lintを、それぞれの検査範囲に合わせて使ってください。
