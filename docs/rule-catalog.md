# ルールの一覧

[README に戻る](../README.md)

## ルール

ルールは 3 系統に分かれます。

| 系統 | ID | 見るもの |
|------|----|---------|
| 語句パターン | `P` | 文の中の定型句・言い回し |
| リズム・統計 | `R` | 文長のばらつき、文末・文頭の反復、段落の構成など、文や段落をまたいだ集計 |
| 構造 | `S` | 太字・箇条書き・見出しなど Markdown の体裁 |

各ルールはレーンとステータスを持ちます。

- **レーン** — `slop` は AI 臭さの検出です。`readability` は読みやすさの指摘（長すぎる一文、二重否定など）で、AI らしさとは無関係なので、AI 臭さとは分けて出します。設定ファイルの独自ルールは、既定で `custom` のレーンに入ります。出力では、それぞれを「AI 臭さ」「読みやすさ」「独自ルール」と表示します。
- **ステータス** — `stable` はコーパスで誤検知率を確かめたもので、既定で有効です。`experimental` は未校正か、辞書なしの近似で校正条件から外れるもので、`--experimental` か設定で有効にしたときだけ動きます。語句ルールは語句ごとにもステータスを持ちます。

| ID | 名前 | 内容 | レーン | ステータス |
|----|------|------|-------|-----------|
| P01 | `AI_CONCLUSION` | 結論の押し付け・まとめ口調 | slop | stable |
| P02 | `AI_PREFACE` | 定型の前置き・予告 | slop | stable |
| P03 | `AI_CONJUNCTION` | 空疎な接続 | slop | stable |
| P04 | `REDUNDANT_VERB` | 冗長な動詞表現 | readability | experimental |
| P05 | `REDUNDANT_MODIFIER` | ぼやけた修飾語 | slop | experimental |
| P06 | `OVER_EMPHASIS` | 過剰な強調 | slop | stable |
| P07 | `HEDGING` | 予防線・免責 | slop | stable |
| P08 | `EMPTY_ADJECTIVE` | 空虚な形容 | slop | stable |
| P09 | `EMPTY_VERB` | 作業を宣言するだけの動詞 | slop | stable |
| P10 | `DRAMATIC_CLOSER` | 演出的な決め文 | slop | experimental |
| P11 | `COUNT_DECLARATION` | プレゼン的な数の予告 | slop | experimental |
| P12 | `TRANSLATIONESE` | 翻訳調 | slop | stable |
| P13 | `INANIMATE_SUBJECT` | 無生物主語と他動詞 | slop | stable |
| P14 | `EM_DASH` | ダッシュの挿入句 | slop | experimental |
| P15 | `KANJI_RUN` | 連続漢字 | readability | stable |
| P16 | `NO_CHAIN` | 「の」の連鎖 | readability | stable |
| P17 | `DOUBLE_NEGATIVE` | 二重否定 | readability | stable |
| P18 | `CHAT_RESIDUE` | 「ご質問ありがとうございます」などチャット応答の名残 | slop | experimental |
| P19 | `HYPE` | 根拠のない誇張表現 | slop | experimental |
| P20 | `COLON_CONTINUATION` | 「以下の通りです：」のような述語とコロンでの列挙の導入 | slop | experimental |
| P21 | `VAGUE_ATTRIBUTION` | 「専門家は〜と指摘しています」などの出典をぼかした権威付け | slop | experimental |
| R01 | `LOW_BURSTINESS` | 文長の単調さ | slop | stable |
| R02 | `REPETITIVE_ENDING` | 文末の反復 | readability | experimental |
| R03 | `LONG_SENTENCE` | 長すぎる一文 | readability | stable |
| R04 | `BURIED_LIST` | 一文に埋もれた列挙 | readability | experimental |
| R05 | `ANTITHESIS_REPETITION` | 否定→肯定の対比の反復 | slop | stable |
| R06 | `NO_NOMINAL_ENDING` | 長い文書での体言止めの欠如 | slop | stable |
| R07 | `UNIFORM_PARAGRAPHS` | 段落の文数がそろいすぎている | slop | stable |
| R08 | `REPEATED_SENTENCE_LEAD` | 文頭の型の反復 | slop | experimental |
| R09 | `PARAGRAPH_LEAD_CONJUNCTION` | 段落頭の接続詞 | slop | experimental |
| R10 | `CLEFT_BECAUSE` | 「それは〜。なぜなら〜」構文 | slop | experimental |
| R11 | `SELF_ANSWER` | 自分で立てた問いに自分で答える | slop | experimental |
| R12 | `OVERCORRECTION` | 直しすぎの均一さ（長短の機械的な交互・体言止めの過多） | slop | experimental |
| R13 | `COMMA_PROFILE` | 読点を打つ癖（1 文あたりの読点の多さ） | slop | experimental |
| R14 | `DUPLICATE_PASSAGE` | 同じ文・段落の再登場（初出の行・列も表示） | readability | experimental |
| R15 | `FORMULAIC_FUTURE_CLOSER` | 文書末尾の「課題は残る → 今後に期待する」という結び | slop | experimental |
| R16 | `REPEATED_EVALUATIVE_TRIAD` | 短い評価語・抽象語の三項列挙の反復 | slop | experimental |
| S01 | `BOLD_DENSITY` | 太字の多用 | slop | experimental |
| S02 | `BULLET_RATIO` | 箇条書きへの偏り | slop | experimental |
| S03 | `BOILERPLATE_HEADING` | 「まとめ」「おわりに」などの定型見出し | slop | experimental |
| S04 | `NUMBERED_PHASES` | 「フェーズ1」などの番号付きの段階 | slop | experimental |
| S05 | `EMOJI_DENSITY` | 絵文字・装飾記号の多用 | slop | experimental |
| S06 | `BOLD_LABEL_LIST` | 「**項目**: 説明」の定型 | slop | experimental |
| S07 | `HEADING_TEMPLATE` | 見出しの型（「X: Y」・問い・番号）の反復 | slop | experimental |
| S08 | `HEADING_EMPHASIS` | 見出しの中の太字・絵文字 | slop | experimental |
| S09 | `STRUCTURE_DENSITY` | 見出しと箇条書きの密度 | slop | experimental |
| S10 | `LABEL_STYLE` | 段落や項目を絵文字・「ラベル：」で書き出す | slop | experimental |

最新の一覧は `noslop rules`、各ルールの詳細は `noslop explain <ID>` で確認できます。全ルールの説明 (何を見るか・なぜ問題か・直し方・例・根拠) は [docs/rules.md](../docs/rules.md) にまとめてあります。ルールの ID は公開後に意味を変えません。

語句パターン系ルールが既定で見るのは地の文（段落）だけです。校正を地の文で行ったためで、リスト・表・引用も見るには設定の `[scope]` を変えます。リズム・統計系ルールは常に地の文だけで集計します。
