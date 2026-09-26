# 出力形式

[README に戻る](../README.md)

## 出力形式

### text（既定）

```text
📄 docs/meeting.md
  AI 臭さの指摘 2 件
  3:1  情報  [AI 臭さ]  P03 AI_CONJUNCTION
    「このように」は前の内容を機械的に束ねる接続です（人間の文章にもよく出るため、弱い手掛かりとして扱っています）
    │ このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。
    │ ^^^^^^^^^^
    💡 接続語に頼らず、前の内容の具体的な事実や疑問から次の話へつないでください
  3:35  警告  [AI 臭さ]  P01 AI_CONCLUSION
    「と言えるだろう」は結論を定型句で押し付ける締めです
    │ このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。
    │                                                                     ^^^^^^^^^^^^^^
    💡 定型句を外して言い切るか、結論を支える事実や数値を書いてください

✖ AI 臭さの指摘 2 件 (警告 1・情報 1) — 1 ファイルを検査、辞書あり (同梱の ipadic)
```

行・列はどちらも 1 始まりで、列は文字数で数えます。

- 指摘はファイルごとに、レーンの節（AI 臭さ → 独自ルール → 読みやすさ）に分けて並びます。節の中は文書の順です。指摘のないレーンの節は出しません
- 各指摘の重大度の隣に、レーン名（`[AI 臭さ]`・`[独自ルール]`・`[読みやすさ]`）が付きます。実験的な項目の指摘には、末尾に `[実験的]` が付きます
- 最後の要約は、節の見出しと同じ呼び名で、レーンごとに件数と重大度の内訳を数えます。AI 臭さは 0 件でも出し、独自ルールと読みやすさは指摘があるときだけ出します
- 要約の先頭の ✖ は、AI 臭さか独自ルールの指摘があること、または `--fail-on` に当たったことを示します。読みやすさの指摘だけなら ✔ です
- 文書全体の点数は出しません（[文書全体の点数を出さない理由](revision.md#文書全体の点数を出さない理由)）

読みやすさの指摘があると、AI 臭さの節のあとに読みやすさの節が続きます。次の例では、読みやすさの指摘（3 行目）のほうが文書の中では先にありますが、節は AI 臭さを先に並べます。

```text
📄 docs/plan.md
  AI 臭さの指摘 1 件
  5:26  警告  [AI 臭さ]  P01 AI_CONCLUSION
    「と言えるだろう」は結論を定型句で押し付ける締めです
    │ 会議を減らしたことは、チームにとって良い変化だったと言えるだろう。
    │                                                   ^^^^^^^^^^^^^^
    💡 定型句を外して言い切るか、結論を支える事実や数値を書いてください

  読みやすさの指摘 1 件
  3:4  情報  [読みやすさ]  P15 KANJI_RUN
    漢字が 8 字続いています (「全社業務改善計画」)
    │ 来期は全社業務改善計画に沿って、会議の数を半分にします。
    │       ^^^^^^^^^^^^^^^^
    💡 語の切れ目が読み取れるか確かめ、助詞や動詞を補って開いてください

✖ AI 臭さの指摘 1 件 (警告 1)、読みやすさの指摘 1 件 (情報 1) — 1 ファイルを検査、辞書あり (同梱の ipadic)
```

### json

機械処理向けの安定したスキーマで出力します。要点は次のとおりです（下の例は、上の text 出力と同じ文書の JSON から P01 の指摘だけを抜き出したものです）。

- トップレベルに `schemaVersion`・`tool`（`name`・`version`）・`columnUnit`・`settings`（`genre`・`experimental`・`failOn`・`morphology`）・`files`・`summary`・`errors`
- `settings.morphology` は判定の方式で、`requested`（`auto` / `required` / `off`）・`method`（`dictionary` / `surface`）・`dictionary`（使った辞書の `name`・`source`（`bundled`: 同梱 / `file`: ファイルの辞書。指定したファイルと、`auto` で share ディレクトリから選んだ辞書）・`path`（同梱なら `null`）。辞書なしなら `null`）・`reason`（辞書を使わなかった理由: `disabled` / `not-found` / `not-needed`）を持つ
- 各ファイルに `path`・`format`（`markdown` / `text`）・`characters`・`sentences`・`counts`（抑制していない指摘を、レーン（`slop`・`readability`・`custom`）ごと・重大度（`error`・`warning`・`info`）ごとに数えたもの。0 件も常に出る）・`diagnostics`・`warnings`
- スキーマの版（`schemaVersion`）は 2 です。版 1 にあった文書全体の点数（`score`）は、版 2 で外して `counts` に置き換えました（[文書全体の点数を出さない理由](revision.md#文書全体の点数を出さない理由)）
- 各指摘に `ruleId`・`ruleName`・`severity`・`lane`・`status`・`message`・`hint`・`range`・`context`・`excerpt`・`related`・`metrics`・`fingerprint`・`suppressed`
- `range` と `context` は `start` と `end` を持ち、それぞれ `line`・`column`（1 始まり。`columnUnit` のとおり Unicode のスカラー値で数える）と `offset`（UTF-8 のバイト位置）を持つ
- `metrics` にはルールごとの値が入る。語句ルールは、一致した辞書の項目を `item` (正規表現の項目は `/パターン/` の形) に、一致した文字列を `matched` に入れる
- `fingerprint` は行番号に依存しない 16 桁の識別子で、前回の結果との突き合わせに使える。同じ文が繰り返されると同じ値になるので、突き合わせは多重集合で行う
- 抑制した指摘も消さずに出し、`suppressed` に `reason`（理由）と `line`（抑制コメントの行）を残す
- 読めなかったファイルは `errors` に `path` と `message` で入る
- 配列はパス・位置・ルール ID の順に並ぶ

```json
{
  "schemaVersion": 2,
  "tool": { "name": "noslop", "version": "26.9.100" },
  "columnUnit": "unicode-scalar",
  "settings": { "genre": "general", "experimental": false, "failOn": "never" },
  "files": [
    {
      "path": "docs/meeting.md",
      "format": "markdown",
      "characters": 56,
      "sentences": 2,
      "counts": {
        "slop": { "error": 0, "warning": 1, "info": 1 },
        "readability": { "error": 0, "warning": 0, "info": 0 },
        "custom": { "error": 0, "warning": 0, "info": 0 }
      },
      "diagnostics": [
        {
          "ruleId": "P01",
          "ruleName": "AI_CONCLUSION",
          "severity": "warning",
          "lane": "slop",
          "status": "stable",
          "message": "「と言えるだろう」は結論を定型句で押し付ける締めです",
          "hint": "定型句を外して言い切るか、結論を支える事実や数値を書いてください",
          "range": {
            "start": { "line": 3, "column": 35, "offset": 133 },
            "end": { "line": 3, "column": 42, "offset": 154 }
          },
          "context": {
            "start": { "line": 3, "column": 1, "offset": 31 },
            "end": { "line": 3, "column": 43, "offset": 157 }
          },
          "excerpt": "このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。",
          "related": [],
          "metrics": { "item": "と言えるだろう", "matched": "と言えるだろう" },
          "fingerprint": "bd78baa3ef438ee5",
          "suppressed": null
        }
      ],
      "warnings": []
    }
  ],
  "summary": {
    "files": 1,
    "filesWithDiagnostics": 1,
    "diagnostics": 2,
    "bySeverity": { "error": 0, "warning": 1, "info": 1 },
    "byLane": { "slop": 2, "readability": 0, "custom": 0 },
    "suppressed": 0,
    "errors": 0
  },
  "errors": []
}
```

### github

GitHub Actions のワークフローコマンドとして出力し、プルリクエストの差分に注釈を付けます。重大度は `error`→`error`、`warning`→`warning`、`info`→`notice` に対応します。注釈のタイトルには、ルールの ID と名前の前にレーン名（AI 臭さ・読みやすさ・独自ルール）が入ります。抑制した指摘は出しません。

```text
::warning file=docs/meeting.md,line=3,col=35,endLine=3,endColumn=42,title=[AI 臭さ] P01 AI_CONCLUSION::「と言えるだろう」は結論を定型句で押し付ける締めです%0A💡 定型句を外して言い切るか、結論を支える事実や数値を書いてください
```

### brief

AI エージェントや編集者に渡す改稿指示を Markdown で出します。先頭に改稿のルール（主張・数字・固有名詞を変えない、原文にない事実を足さない、同じ直しを一律に当てない、指摘は残してよい、件数を減らすことを目的にしない、再実行は 1 回だけ）を置き、ルールごとに「なぜ疑わしいか」「直し方の方向」「該当箇所」をまとめます。語句の置き換え方は指示しません。

```markdown
## docs/meeting.md

- 指摘: AI 臭さ (校正済み) 2 件 / AI 臭さ (実験的) 0 件 / 独自ルール 0 件 / 読みやすさ 0 件

### 優先して見る箇所

#### 1. P01 AI_CONCLUSION — 結論の押し付け・まとめ口調 (AI 臭さ・警告 1 件)

- なぜ疑わしいか: 要約の定型として大量に学習された言い回しで、生成された文章ほど段落の終わりに現れます。…
- 直し方の方向: 定型句を外して言い切るか、結論を支える事実や数値を書いてください
- 該当箇所:
  - L3: 「と言えるだろう」は結論を定型句で押し付ける締めです
    - 原文: このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。
```

出力の全体の構成は [docs/integrations.md](../docs/integrations.md) にあります。

### 改稿指示の JSON / TOON（`--report brief --format json|toon`）

改稿指示と同じ内容を、プログラムで扱う JSON と、LLM に少ないトークンで渡す [TOON](https://github.com/toon-format/spec)（Token-Oriented Object Notation）で出します。どちらも同じデータで、ファイルごとに 2 つの表を持ちます。

- `rules` — 指摘のあったルール。優先して見る順に並び、`ruleId`・`ruleName`・`title`・`lane`・`maxSeverity`・`experimentalOnly`・`count`・`omittedCount`（`--brief-limit` を超えて載せなかった件数）・`why`（なぜ疑わしいか）・`hint`（直し方の方向）を持つ
- `occurrences` — 該当箇所。`ruleId` で `rules` を参照し、`line`・`column`（1 始まり、列は文字数）・`message`・`excerpt`（指摘を含む文の抜粋。文を持たない指摘は `null`）を持つ

ほかに `settings`（`genre`・`experimental`・`method`（判定の方式。`dictionary` / `surface`）・`dictionary`（使った辞書の名前。手元のパスは載せない））・`revisionRules`（改稿のルール）・`editorialQuestions`（編集の問い）・`note`（指摘がないときの断り書き）・`cleanFiles`・`warnings`・`errors` があります。metrics・fingerprint・抑制した指摘は入れません（指摘の追跡には全指摘のレポートを使います）。スキーマの版は `schemaVersion`、種類は `kind: brief` です。

```toon
files[1]:
  - path: docs/meeting.md
    counts:
      stableSlop: 2
      experimentalSlop: 0
      custom: 0
      readability: 0
    rules[2]{ruleId,ruleName,title,lane,maxSeverity,experimentalOnly,count,omittedCount,why,hint}:
      P01,AI_CONCLUSION,結論の押し付け・まとめ口調,slop,warning,false,1,0,要約の定型として…,定型句を外して言い切るか、結論を支える事実や数値を書いてください
      P03,AI_CONJUNCTION,空疎な接続,slop,info,false,1,0,前の段落で何が述べられたかに…,接続語に頼らず、前の内容の具体的な事実や疑問から次の話へつないでください
    occurrences[2]{ruleId,line,column,message,excerpt}:
      P01,3,35,「と言えるだろう」は結論を定型句で押し付ける締めです,このように、定例会議を減らしたことは…と言えるだろう。
      P03,3,1,「このように」は前の内容を機械的に束ねる接続です…,このように、定例会議を減らしたことは…と言えるだろう。
```

TOON は、ルールと該当箇所を 1 行 1 要素の表にするぶん短くなります。同梱の [examples/ai-smelly.md](../examples/ai-smelly.md) の改稿指示では、トークン数（`o200k_base`）が TOON 2,770・Markdown 2,980・JSON 3,137（改行なし）/ 3,802（整形）でした。

### toon

全指摘のレポート（`--format json` と同じデータ）と `noslop diff` の結果を TOON で出します（`--format toon`）。整形した JSON より 2〜3 割短くなりますが、指摘ごとに `metrics` の項目が違うなど表にできない部分が多いため、改行なしの JSON よりは長くなります。LLM に指摘を渡すなら、改稿指示の TOON（`--report brief --format toon`）を使ってください。

TOON の符号化は公式の Rust 実装 [toon-format](https://github.com/toon-format/toon-rust)（仕様 v3.0）で行い、末尾に改行は付けません。出力は、仕様 v4.1 のリファレンス実装（`@toon-format/toon` 4.1.1）の strict モードで JSON と同じデータに戻ることを確かめています。
