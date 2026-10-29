# 出力形式

[README に戻る](../README.md)

## 出力形式

### text（既定）

```text
📄 docs/meeting.md
  AI 臭さの指摘 2 件
  3:21  情報  [AI 臭さ]  P08 EMPTY_ADJECTIVE [中程度の手掛かり]
    「不可欠」は中身を説明せずに重みだけを足す形容です
    │ 定例会議を減らしたことは、チームの集中に不可欠な変化だったと言えるだろう。
    │                                         ^^^^^^
    💡 何がどう重要・網羅的なのか、観点や根拠そのものを書いてください
  3:30  警告  [AI 臭さ]  P01 AI_CONCLUSION [強い手掛かり]
    「と言えるだろう」は結論を定型句で押し付ける締めです
    │ 定例会議を減らしたことは、チームの集中に不可欠な変化だったと言えるだろう。
    │                                                           ^^^^^^^^^^^^^^
    💡 定型句を外して言い切るか、結論を支える事実や数値を書いてください

✖ AI 臭さの指摘 2 件 (警告 1・情報 1) — 1 ファイルを検査、辞書あり (同梱の ipadic)
```

行・列はどちらも 1 始まりで、列は文字数で数えます。

- 指摘はファイルごとに、レーンの節（AI 臭さ → 独自ルール → 読みやすさ）に分けて並びます。節の中は文書の順です。指摘のないレーンの節は出しません
- 各指摘の重大度の隣に、レーン名（`[AI 臭さ]`・`[独自ルール]`・`[読みやすさ]`）が付きます。AI 臭さのレーンの指摘には、見出し行の末尾に[手掛かりの強さ](#手掛かりの強さ)（`[強い手掛かり]`・`[中程度の手掛かり]`・`[弱い手掛かり]`）が付きます
- 最後の要約は、節の見出しと同じ呼び名で、レーンごとに件数と重大度の内訳を数えます。AI 臭さは 0 件でも出し、独自ルールと読みやすさは指摘があるときだけ出します
- 要約の先頭の ✖ は、AI 臭さか独自ルールの指摘があること、または `--fail-on` に当たったことを示します。読みやすさの指摘だけなら ✔ です
- 文書全体の点数は出しません（[文書全体の点数を出さない理由](revision.md#文書全体の点数を出さない理由)）

読みやすさの指摘があると、AI 臭さの節のあとに読みやすさの節が続きます。次の例では、読みやすさの指摘（3 行目）のほうが文書の中では先にありますが、節は AI 臭さを先に並べます。

```text
📄 docs/plan.md
  AI 臭さの指摘 1 件
  5:26  警告  [AI 臭さ]  P01 AI_CONCLUSION [強い手掛かり]
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

### 手掛かりの強さ

AI 臭さのレーンの指摘には、手掛かりの強さを 3 段階で付けます。その指摘が、生成した文章に特徴的な傾向の手掛かりとしてどれだけ当てになるかの目安です。

| 強さ | 出力での呼び名 | JSON | 意味 |
|---|---|---|---|
| 強い | 強い手掛かり | `strong` | 人の文書にほとんど出ず、生成文書に偏って出る |
| 中程度 | 中程度の手掛かり | `moderate` | 生成文書に多いが人の文書にも出る。件数が少なく測れていないものも含む |
| 弱い | 弱い手掛かり | `weak` | 人の文書にも同じくらいかそれ以上に出る |

強さは、人の文書と生成文書を検査して、指摘が出た文書の数と割合から決めます。生成文書の 5 本以上に出て、生成文書の割合が人の文書の 3 倍以上、人の文書の割合が 5% 以下のものを「強い」とします。人の文書の 5 本以上に出て、生成文書の割合が人の文書の 1.5 倍以下のものは「弱い」です。どちらにも当たらなければ「中程度」になります。指摘の出た文書が人と生成で合わせて 5 本に満たないなど、測れないものは検出の仕組みから決めます (既定は中程度)。ルールごとの根拠は `noslop explain <ID>` の「根拠」にあります。

強さは、指摘が正しい確率でも、文章が生成された確率でもありません。直す優先度は重大度で表し、段階を足し合わせて点数にもしません。強さを付けるのは AI 臭さのレーンの組み込みルールだけで、読みやすさのレーンと独自ルールの指摘には付けません (JSON では `null`)。語句ルールでは語句ごとに強さが違うことがあり、設定の `[rules.<ID>] severity` で重大度を変えても強さは変わりません。

改稿指示 (brief) は、重大度が同じルールを強さの順に並べます。フックは、重大度が情報で手掛かりが弱い指摘を詳しく返さず、件数だけを添えます ([AI エージェントとの連携](integrations.md#弱い手掛かりの情報の指摘を件数だけにする))。

### json

機械処理向けの安定したスキーマで出力します。要点は次のとおりです（下の例は、上の text 出力と同じ文書の JSON から P01 の指摘だけを抜き出したものです）。

- トップレベルに `schemaVersion`・`tool`（`name`・`version`）・`columnUnit`・`settings`（`genre`・`failOn`・`morphology`）・`files`・`summary`・`errors`
- `settings.morphology` は判定の方式で、`requested`（`auto` / `required` / `off`）・`method`（`dictionary` / `surface`）・`dictionary`（使った辞書の `name`・`source`（`bundled`: 同梱 / `file`: ファイルの辞書。指定したファイルと、`auto` で share ディレクトリから選んだ辞書）・`path`（同梱なら `null`）。辞書なしなら `null`）・`reason`（辞書を使わなかった理由: `disabled` / `not-found` / `not-needed`）を持つ
- 各ファイルに `path`・`format`（`markdown` / `text`）・`characters`・`sentences`・`counts`（抑制していない指摘を、レーン（`slop`・`readability`・`custom`）ごと・重大度（`error`・`warning`・`info`）ごとに数えたもの。0 件も常に出る）・`diagnostics`・`warnings`
- スキーマの版（`schemaVersion`）は 3 です。版 3 で、指摘ごとの `status`（`stable` / `experimental`）と `settings.experimental` を外し、手掛かりの強さの `signal` を足しました。版 1 にあった文書全体の点数（`score`）は、版 2 で外して `counts` に置き換えました（[文書全体の点数を出さない理由](revision.md#文書全体の点数を出さない理由)）
- 各指摘に `ruleId`・`ruleName`・`severity`・`lane`・`signal`・`message`・`hint`・`range`・`context`・`excerpt`・`related`・`metrics`・`fingerprint`・`suppressed`
- `signal` は[手掛かりの強さ](#手掛かりの強さ)（`strong` / `moderate` / `weak`）。AI 臭さのレーンの組み込みルールの指摘だけが持ち、読みやすさのレーンと独自ルールの指摘では `null`
- `noslop diff --format json` も、指摘と件数を同じ形で出します。diff のスキーマの版も 3 で、版 3 で同じく `status` を外して `signal` を足しました
- `range` と `context` は `start` と `end` を持ち、それぞれ `line`・`column`（1 始まり。`columnUnit` のとおり Unicode のスカラー値で数える）と `offset`（UTF-8 のバイト位置）を持つ
- `metrics` にはルールごとの値が入る。語句ルールは、一致した辞書の項目を `item` (正規表現の項目は `/パターン/` の形) に、一致した文字列を `matched` に入れる
- `fingerprint` は行番号に依存しない 16 桁の識別子で、前回の結果との突き合わせに使える。同じ文が繰り返されると同じ値になるので、突き合わせは多重集合で行う
- 抑制した指摘も消さずに出し、`suppressed` に `reason`（理由）と `line`（抑制コメントの行）を残す
- 読めなかったファイルは `errors` に `path` と `message` で入る
- 配列はパス・位置・ルール ID の順に並ぶ

```json
{
  "schemaVersion": 3,
  "tool": { "name": "noslop", "version": "26.9.100" },
  "columnUnit": "unicode-scalar",
  "settings": { "genre": "general", "failOn": "never" },
  "files": [
    {
      "path": "docs/meeting.md",
      "format": "markdown",
      "characters": 51,
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
          "signal": "strong",
          "message": "「と言えるだろう」は結論を定型句で押し付ける締めです",
          "hint": "定型句を外して言い切るか、結論を支える事実や数値を書いてください",
          "range": {
            "start": { "line": 3, "column": 30, "offset": 118 },
            "end": { "line": 3, "column": 37, "offset": 139 }
          },
          "context": {
            "start": { "line": 3, "column": 1, "offset": 31 },
            "end": { "line": 3, "column": 38, "offset": 142 }
          },
          "excerpt": "定例会議を減らしたことは、チームの集中に不可欠な変化だったと言えるだろう。",
          "related": [],
          "metrics": { "item": "と言えるだろう", "matched": "と言えるだろう" },
          "fingerprint": "6d075393a18e8c68",
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
::notice file=docs/meeting.md,line=3,col=21,endLine=3,endColumn=24,title=[AI 臭さ] P08 EMPTY_ADJECTIVE::「不可欠」は中身を説明せずに重みだけを足す形容です%0A💡 何がどう重要・網羅的なのか、観点や根拠そのものを書いてください
::warning file=docs/meeting.md,line=3,col=30,endLine=3,endColumn=37,title=[AI 臭さ] P01 AI_CONCLUSION::「と言えるだろう」は結論を定型句で押し付ける締めです%0A💡 定型句を外して言い切るか、結論を支える事実や数値を書いてください
```

### brief

AI エージェントや編集者に渡す改稿指示を Markdown で出します。先頭に改稿のルール（主張・数字・固有名詞を変えない、原文にない事実を足さない、同じ直しを一律に当てない、指摘は残してよい、件数を減らすことを目的にしない、再実行は 1 回だけ）を置き、ルールごとに「なぜ疑わしいか」「直し方の方向」「該当箇所」をまとめます。語句の置き換え方は指示しません。

ルールは、重大度の高い順に並べ、重大度が同じなら手掛かりの強い順、強さも同じなら件数の多い順に並べます。見出しの括弧には、レーン名・最も重い重大度・件数を書き、AI 臭さのレーンなら手掛かりの強さを添えます。強さは、最も重い指摘のうち最も強いものです (警告の弱い指摘と情報の強い指摘を持つルールは「警告・弱い手掛かり」になります)。

```markdown
## docs/meeting.md

- 指摘: AI 臭さ 2 件 / 独自ルール 0 件 / 読みやすさ 0 件

### 優先して見る箇所

#### 1. P01 AI_CONCLUSION — 結論の押し付け・まとめ口調 (AI 臭さ・警告 1 件・強い手掛かり)

- なぜ疑わしいか: 要約の定型として大量に学習された言い回しで、生成された文章ほど文書の結びに現れます。…
- 直し方の方向: 定型句を外して言い切るか、結論を支える事実や数値を書いてください
- 該当箇所:
  - L3: 「と言えるだろう」は結論を定型句で押し付ける締めです
    - 原文: 定例会議を減らしたことは、チームの集中に不可欠な変化だったと言えるだろう。

#### 2. P08 EMPTY_ADJECTIVE — 空虚な形容 (AI 臭さ・情報 1 件・中程度の手掛かり)

- なぜ疑わしいか: 「多角的に分析した」と書いても、どの角度から見たのかを書かない限り情報は増えません。…
- 直し方の方向: 何がどう重要・網羅的なのか、観点や根拠そのものを書いてください
- 該当箇所:
  - L3: 「不可欠」は中身を説明せずに重みだけを足す形容です
    - 原文: 定例会議を減らしたことは、チームの集中に不可欠な変化だったと言えるだろう。
```

出力の全体の構成は [docs/integrations.md](../docs/integrations.md) にあります。

### 改稿指示の JSON / TOON（`--report brief --format json|toon`）

改稿指示と同じ内容を、プログラムで扱う JSON と、LLM に少ないトークンで渡す [TOON](https://github.com/toon-format/spec)（Token-Oriented Object Notation）で出します。どちらも同じデータで、ファイルごとに 2 つの表を持ちます。

- `rules` — 指摘のあったルール。優先して見る順に並び、`ruleId`・`ruleName`・`title`・`lane`・`maxSeverity`・`signal`（手掛かりの強さ。最も重い指摘のうち最も強いもの。AI 臭さのレーンだけで、ほかは `null`）・`count`・`omittedCount`（`--brief-limit` を超えて載せなかった件数）・`why`（なぜ疑わしいか）・`hint`（直し方の方向）を持つ
- `occurrences` — 該当箇所。`ruleId` で `rules` を参照し、`line`・`column`（1 始まり、列は文字数）・`signal`（その指摘の手掛かりの強さ。語句ルールでは語句ごとに違うことがある）・`message`・`excerpt`（指摘を含む文の抜粋。文を持たない指摘は `null`）を持つ

ファイルごとには、ほかに `counts`（改稿指示に載せた指摘の件数。レーンごとに `slop`・`custom`・`readability`）があります。全体には `settings`（`genre`・`method`（判定の方式。`dictionary` / `surface`）・`dictionary`（使った辞書の名前。手元のパスは載せない））・`revisionRules`（改稿のルール）・`editorialQuestions`（編集の問い）・`note`（指摘がないときの断り書き）・`cleanFiles`・`warnings`・`errors` があります。metrics・fingerprint・抑制した指摘は入れません（指摘の追跡には全指摘のレポートを使います）。種類は `kind: brief` です。

スキーマの版（`schemaVersion`）は 2 です。版 2 では、AI 臭さの件数を `stableSlop`・`experimentalSlop` に分けるのをやめて `slop` にまとめました。`rules` の `experimentalOnly` と `settings.experimental` を外し、`rules` と `occurrences` に `signal` を足しています。

```toon
files[1]:
  - path: docs/meeting.md
    counts:
      slop: 2
      custom: 0
      readability: 0
    rules[2]{ruleId,ruleName,title,lane,maxSeverity,signal,count,omittedCount,why,hint}:
      P01,AI_CONCLUSION,結論の押し付け・まとめ口調,slop,warning,strong,1,0,要約の定型として…,定型句を外して言い切るか、結論を支える事実や数値を書いてください
      P08,EMPTY_ADJECTIVE,空虚な形容,slop,info,moderate,1,0,「多角的に分析した」と書いても…,何がどう重要・網羅的なのか、観点や根拠そのものを書いてください
    occurrences[2]{ruleId,line,column,signal,message,excerpt}:
      P01,3,30,strong,「と言えるだろう」は結論を定型句で押し付ける締めです,定例会議を減らしたことは…と言えるだろう。
      P08,3,21,moderate,「不可欠」は中身を説明せずに重みだけを足す形容です,定例会議を減らしたことは…と言えるだろう。
```

TOON は、ルールと該当箇所を 1 行 1 要素の表にするぶん短くなります。同梱の [examples/ai-smelly.md](../examples/ai-smelly.md) を `--no-config --dict bundled` で検査しました。改稿指示のトークン数（`o200k_base`）は TOON 3,893・Markdown 4,406・JSON 4,566（改行なし）/ 5,661（整形）でした。

### toon

全指摘のレポート（`--format json` と同じデータ）と `noslop diff` の結果を TOON で出します（`--format toon`）。指摘ごとに `metrics` の項目が違うなど、表にできない部分もあります。LLM に指摘を渡すなら、改稿指示の TOON（`--report brief --format toon`）を使ってください。

TOON の符号化は公式の Rust 実装 [toon-format](https://github.com/toon-format/toon-rust)（仕様 v4.4）で行い、末尾に改行は付けません。レーン別件数などの同じ形のオブジェクトは、キー付きの表になることがあります。行の配置を固定とみなさず、TOON のデコーダーで読み取ってください。全指摘・改稿指示・diff の出力は、リファレンス実装（`@toon-format/toon` 4.4.0）の strict モードで JSON と同じデータに戻ることを確かめています。
