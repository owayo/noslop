---
name: noslop
description: >-
  日本語の文章 (Markdown・テキスト) から「AI 臭さ」(LLM の文章に特有の定型句・単調な文のリズム・
  見出しと箇条書きだけの体裁) を辞書なしで検出する CLI。日本語の文章を書いた・直した後の見直し、
  AI が書いたような文を自然にしたいとき、推敲・校正・AI っぽさのチェック、改稿の前後の比較
  (数字や固有名詞が消えていないか・同じ直しを一律に当てていないか) の場面で発動。
  noslop check --report brief --format toon で直す箇所を少ないトークンで受け取り、
  改稿のルールに従って効果の大きい箇所だけを直す。直した後は noslop diff で 1 回だけ確かめる。
allowed-tools: Bash(noslop:*)
---

# noslop

日本語の文章から、LLM が書いた文章に出やすい癖 (`と言えるでしょう` のような定型句、`〜ではなく〜` の対比の繰り返し、文の長さがそろいすぎたリズム、見出しと太字だけで組んだ構成) を、決まった規則で指さす Linter。**指摘は疑いの提示**で、直すかどうかは文脈で決める。件数を減らすこと自体を目的にしない。

## 使う場面

- 日本語の文章 (README・ブログ・議事録・報告書・技術文書) を書いた、または直した直後の見直し
- 「AI っぽさを抜いて」「自然な日本語に推敲して」と頼まれたとき
- 改稿した結果、数字や固有名詞が消えていないか・同じ直しを全体に一律に当てていないかを確かめるとき

コードや英語の文章には使わない (英語の段落は数えない)。

## 手順

1. 直す箇所を受け取る。少ないトークンで受け取るなら TOON、プログラムで扱うなら JSON、人に見せるなら Markdown。

   ```bash
   noslop check --report brief --format toon <FILE>   # 改稿指示 (TOON)
   noslop check --report brief --format json <FILE>   # 改稿指示 (JSON)
   noslop check --format brief <FILE>                 # 改稿指示 (Markdown)
   ```

   技術文書・ビジネス文書・エッセイは `--genre tech|business|essay` を付ける (閾値と、ジャンルの慣習と衝突するルールが変わる)。

2. **改稿のルール (`revisionRules`) に従って直す。** 主張・確信度・数字・固有名詞・引用・想定読者を変えない。原文にない事実・数字・体験を足さない (材料が足りなければ書き手に確かめる)。同じ直しを全箇所に一律に当てず、効果の大きい箇所だけを直す。文脈上必要な箇所は直さずに残してよい (残した理由は報告に書く)。

3. 直した後、**1 回だけ**直す前と比べる。新しく出た指摘・消えた数字や固有名詞・文書全体に一律に当てた直し (読点の一括削除など) が出る。

   ```bash
   noslop diff <直す前> <直した後>
   git show HEAD:<FILE> | noslop diff - <FILE> --stdin-filename <FILE>   # 直前のコミットと比べる
   ```

   確認事項は失敗の判定ではない。消えた事実は、削ってよい情報か書き手に確かめる。再実行はここで打ち切る。

## 改稿指示の読み方 (TOON)

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

- 先頭に `revisionRules` (改稿のルール) と `editorialQuestions` (ルールでは拾えない観点の問い) がある。
- `rules` はファイルごとの指摘のあったルールで、**優先して見る順** (AI 臭さ → 独自ルール → 読みやすさ、校正済み → 実験的、重大度の高い順) に並ぶ。`why` はなぜ疑わしいか、`hint` は直し方の方向。`lane` が `readability` のものは読みやすさの指摘で、優先度は低い。
- `occurrences` は該当箇所で、`ruleId` で `rules` を参照する。行・列は 1 始まり (列は文字数)。`excerpt` は指摘を含む文の抜粋 (文書全体の集計に基づく指摘は `null`)。1 ルールあたり `--brief-limit` 件 (既定 5) まで載り、残りの件数は `rules` の `omittedCount`。
- 指摘がなければ `files` は空で、`note` に断り書きが入る。指摘がないことは、内容の正しさの保証ではない。

## そのほかのコマンド

| やりたいこと | コマンド |
|---|---|
| ルールの詳細 (何を見るか・なぜ問題か・直し方・例・根拠) | `noslop explain <ID>` (例: `noslop explain P01`) |
| ルールの一覧 | `noslop rules` |
| 全指摘のレポート (抑制した指摘・位置・metrics・fingerprint を含む) | `noslop check --format json <FILE>` (TOON なら `--format toon`) |
| 標準入力の文章を検査する | `pbpaste \| noslop check - --stdin-filename draft.md --report brief --format toon` |
| ディレクトリをまとめて検査する | `noslop check docs --report brief --format toon` |

## 注意

- 自然度スコアや指摘の件数を上げ下げすることを目的にしない。スコアは改稿指示のデータには入れていない。
- 書き手が今後も残すと決めた箇所だけに抑制コメントを書く (`<!-- noslop-disable-next-line P01 -- 理由 -->`)。指摘を消すために足さない。
- 実験的なルール (`experimentalOnly: true`、`--experimental` で有効) は未校正。校正済みの指摘を優先する。
- 設定ファイル `noslop.toml` があれば、カレントから親へたどって読む (`--no-config` で読まない)。
