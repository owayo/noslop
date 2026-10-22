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
- **ステータス** — `stable` は既定で有効です。語句と閾値はコーパスで誤検知率を確かめ、明示的な構造を照合するS11・S12は実文書と境界事例を確認しています。`experimental` は未校正か、辞書なしの近似で校正条件から外れるもので、`--experimental` か設定で有効にしたときだけ動きます。語句ルールは語句ごとにもステータスを持ちます。

| ID | 名前 | 内容 | レーン | ステータス |
|----|------|------|-------|-----------|
| P01 | `AI_CONCLUSION` | 文書の結びの、結論の押し付け・まとめ口調（「まとめると」は要約の導入に限る） | slop | stable |
| P02 | `AI_PREFACE` | 定型の前置き・予告 | slop | experimental |
| P03 | `AI_CONJUNCTION` | 空疎な接続 | slop | experimental |
| P04 | `REDUNDANT_VERB` | 冗長な動詞表現 | readability | experimental |
| P05 | `REDUNDANT_MODIFIER` | ぼやけた修飾語 | slop | experimental |
| P06 | `OVER_EMPHASIS` | 過剰な強調 | slop | stable |
| P07 | `HEDGING` | 予防線・免責 | slop | stable |
| P08 | `EMPTY_ADJECTIVE` | 空虚な形容 | slop | stable |
| P09 | `EMPTY_VERB` | 作業を宣言するだけの動詞 | slop | stable |
| P10 | `DRAMATIC_CLOSER` | 演出的な決め文 | slop | experimental |
| P11 | `COUNT_DECLARATION` | プレゼン的な数の予告 | slop | experimental |
| P12 | `TRANSLATIONESE` | 翻訳調 | readability | experimental |
| P13 | `INANIMATE_SUBJECT` | 無生物主語と他動詞 | slop | experimental |
| P14 | `EM_DASH` | ダッシュの挿入句 | slop | experimental |
| P15 | `KANJI_RUN` | 連続漢字 | readability | stable |
| P16 | `NO_CHAIN` | 「の」の連鎖 | readability | stable |
| P17 | `DOUBLE_NEGATIVE` | 二重否定 | readability | stable |
| P18 | `CHAT_RESIDUE` | 「ご質問ありがとうございます」などチャット応答の名残 | slop | experimental |
| P19 | `HYPE` | 根拠のない誇張表現 | slop | experimental |
| P20 | `COLON_CONTINUATION` | 「以下の通りです：」のような述語とコロンでの列挙の導入 | slop | experimental |
| P21 | `VAGUE_ATTRIBUTION` | 「専門家は〜と指摘しています」などの出典をぼかした権威付け | slop | experimental |
| P22 | `CITATION_ARTIFACT` | 出典のリンクに変換されず残った生成ツールの引用マーカー | slop | experimental |
| P23 | `MARKUP_RESIDUE` | 太字にならずに記号のまま残った Markdown の強調記法 (`**`) | slop | experimental |
| P24 | `VAGUE_EFFECT` | 「地味に効く」のような効果を感覚的に評する言い回し (効果が説明されていれば残せる) | slop | experimental |
| P25 | `SOURCE_STANCE` | 本・記事・著者などの態度を解説する言い回し (内容の要約に必要か見直す) | slop | experimental |
| P26 | `ACTION_JARGON` | 業務の対象に使う「打ち手」「落とし込む」「巻き取る」「握る」を具体化する | readability | experimental |
| P27 | `STOCK_CLOSER` | 「参考になれば幸いです」などの結びの定型と言い換え | slop | tech では stable、ほかは experimental |
| P28 | `PERMISSIVE_ACTION` | 「追記しても構いません」などの許可の形で語る行動 (必要な許可は残せる) | readability | experimental |
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
| R17 | `GUIDE_CLICHE_DENSITY` | 解説の定型句の密集（複数系統の語句と本文量で判定） | slop | experimental |
| R18 | `REPEATED_NOMINAL_LIST` | 名詞句を三つ以上並べる体言止めの反復 | slop | experimental |
| R19 | `CLOSING_CALL_TO_ACTION` | 文書末尾で実践を促す定型の呼びかけ | slop | experimental |
| R20 | `MIXED_WRITING_STYLE` | 全文が対象のときだけ、本文全体の敬体と常体の混在を見る | readability | experimental |
| R21 | `REPEATED_SHORT_SLOGAN` | 読点で区切った短い決め台詞の反復（見出しと一文段落） | slop | experimental |
| S01 | `BOLD_DENSITY` | 太字の多用 | slop | experimental |
| S02 | `BULLET_RATIO` | 箇条書きへの偏り | slop | experimental |
| S03 | `BOILERPLATE_HEADING` | 「まとめ」「おわりに」などの定型見出し | slop | experimental |
| S04 | `NUMBERED_PHASES` | 「フェーズ1」などの番号付きの段階 | slop | experimental |
| S05 | `EMOJI_DENSITY` | 絵文字・装飾記号の多用 | slop | experimental |
| S06 | `BOLD_LABEL_LIST` | 「**項目**: 説明」の定型 | slop | tech・general では stable、ほかは experimental |
| S07 | `HEADING_TEMPLATE` | 見出しの型（「X: Y」・問い・番号）の反復 | slop | experimental |
| S08 | `HEADING_EMPHASIS` | 見出しの中の太字・絵文字 | slop | experimental |
| S09 | `STRUCTURE_DENSITY` | 見出しと箇条書きの密度 | slop | experimental |
| S10 | `LABEL_STYLE` | 段落や項目を絵文字・「ラベル：」で書き出す | slop | experimental |
| S11 | `ENUMERATION_COUNT_MISMATCH` | 直後のリストと列挙の予告の項目数を照合する | readability | stable |
| S12 | `EMPTY_SECTION` | Markdown上で本文も子の節もない見出し（全文が対象のときだけ） | readability | stable |

構造の対応と、目的・読者を指定した18観点の意味レビューは[文章レビュー](review.md)にまとめています。

R20 は `--enable-rules R20` または `[rules] enable = ["R20"]` で有効にできます。地の文に敬体と常体が各2文以上あると、根拠となる文末を示します。`[rules.R20] min_each = 1` のように最小文数を変えられます。部分差分では明示的に有効でも判定を実行せず、新規ファイルなど空白以外の全文が差分になっている場合は実行します。`noslop diff` と MCP の `diff` は改稿前が空のときだけ改稿後を全文の対象とし、それ以外では R20 を実行しません。引用・会話・見出し・リスト・表・独立した断片は比べません。gws の書き込み値は宛先の全文を取得しないため、R20 を実行しません。混在が意図した書き分けかは判別できないため、実験的な info の指摘です。

最新の一覧は `noslop rules`、各ルールの詳細は `noslop explain <ID>` で確認できます。全ルールの説明 (何を見るか・なぜ問題か・直し方・例・根拠) は [docs/rules.md](../docs/rules.md) にまとめてあります。ルールの ID は公開後に意味を変えません。

語句パターン系ルールが既定で見るのは地の文（段落）だけです。校正を地の文で行ったためで、リスト・表・引用も見るには設定の `[scope]` を変えます。ただし引用マーカーの残骸（P22）と記法の残骸（P23）は、残った記号がどこにあっても読み手に見えるので、この設定に関わらず見出し・リスト・表・引用も見ます。P22は1行の中の完全なマーカーを対象とし、通常の脚注・コード・リンクのURL部分を除きます。リズム・統計系ルールは地の文を集計します。改行で並べた「ラベル: 値」の行（「**報告日**: 2026年10月1日」「出典：〜」のような項目）は、地の文の文に数えません。行をまるごと占め、文末記号で終わらない行が当たり、値が文になっている行（「**メリット**: 〜がいい。」）は地の文として数えます。R21 は例外で、見出しと短い一文段落に限って、読点で区切る型の反復を数えます。

R21 は `--experimental` または `--enable-rules R21` で有効にできます。文末記号と空白を除いて 30 字以内の見出し・一文段落に候補が 3 件以上あると、各箇所を情報として示します。`[rules.R21] min_count = 4` のように反復回数を変えられ、`noslop calibrate` でもこの閾値を測れます。「A から B へ」や二文の教訓、「唯一」などの語句だけは検出条件に含めません。

P25は単独の「筆者」を出典の主語として数えません。書き手自身への言及と区別できないためです。「本書の筆者」「この記事の筆者」のように出典を直接添えた主語は検査します。出典の態度を論じる必要があれば、指摘を残したままにするか抑制できます。

## 既定の判定の見直し（2026 年 10 月）

人の文書（一般・技術・業務・随筆）と、2025 年頃と 2026 年のモデルの生成文書で既定のルールを測り直しました。人の文書に生成文書と同じかそれ以上に出る語句・閾値は既定から外し、ルールの意図と違うものを拾っていた条件を直しました。その後、書式（太字・箇条書き）を保った人の技術記事と一般向けの記事でも確かめ、一般向けの記事で多く出た P01 の語句と R01 の閾値を見直しました。表は、26.10.110 からの変更点です。

| ルール | 変わったこと |
| --- | --- |
| P01 | 結論の定型句は、文書の結び（日本語を含む地の文の最後の 2 段落）にあるときだけ指摘する。本文の途中の問いかけ（「〜な方も多いのではないでしょうか」）や節の途中のまとめは指摘しない。コードのコメントや表計算のセルのような断片には当てない。「いかがでしょうか」「いかがでしたか」は、一般向けの人の記事の結びにも同じくらい出たので、実験的な項目にした |
| P02・P03・P13 | 実験的にした。前置きの「見ていきましょう」「さて、」、接続の「このように」、無生物主語の「これは〜を意味する」は、人の文章にも同じくらいかそれ以上に出た |
| P06・P08・P09 | 人の文章にも同じくらいかそれ以上に出た語句（「まさしく」「ポイントは」「根本的な」「総合的」「言語化する」「深掘りする」）を実験的な項目にした。「大切なのは」は警告から情報にした。生成文書に多く出た「第一歩」は、情報の弱い手掛かりとして既定で拾う |
| 語句ルール全般 | 引用符の内側の語句と、開き引用符の直後の語句は、語句への言及として指摘しない（以前は P01・P03・P27 だけ） |
| P08 | 「包括的経済連携」「包括的データ戦略」のように、「〜的」に名詞が直接続く用語は指摘しない |
| P13 | 受け身（「〜ことが示された」）と、主語と述語が括弧で隔てられた文は指摘しない |
| P15 | 中国語の文と語句は数えない |
| P17 | 「〜ずにはいられない」「〜ないわけにはいかない」のような慣用の言い回しと、引用符の内側の言い回しは指摘しない |
| R01 | 警告の閾値を −0.38 から −0.50 に、10〜19 文の文書に出す参考値の閾値を −0.45 から −0.50 にした。警告の閾値は、−0.45 でも一般向けの人の記事の 1 割ほどに出た |
| R05 | 比率 2% 未満は指摘しない。2〜5% は情報、5% 以上は重大（下の節） |
| R07 | ほぼすべての段落が 1 文の文書（段落あたり平均 1.5 文未満）は判定しない |
| R02 ほか | 改行で並べた「ラベル: 値」の行（「**報告日**: 2026年10月1日」）を、文末の連続と地の文の統計に数えない |
| S01・S05・S09 | 1000 字あたりの密度を、文書の先頭の front matter を除いた本文で数える |
| S06 | 続いて並ぶ箇条書きごとに数え、箇条書き 1 つに 1 件出す（以前は文書全体の項目を数え、項目ごとに出していた）。書式を保った人の技術記事と一般向けの記事で誤検知率を確かめたので、技術文書（`--genre tech`）と一般向けの文書（`--genre general`。ジャンルを指定しないときの既定）では既定で動かす。業務文書と随筆では実験的 |

26.10.110 の動きに戻すなら、実験的にしたルールは `--enable-rules P02,P03,P13`（設定なら `[rules] enable = ["P02", "P03", "P13"]`）、実験的な項目は `--experimental` で有効にできます。R01 の閾値は `[rules.R01]` の `threshold = -0.38` と `info_threshold = -0.45` で戻せます。既定で動くようにした S06 は、`--ignore-rules S06`（設定なら `[rules.S06] enabled = false`）で止められます。

## 対比の反復（R05）の区切りと設定の移行

対比の反復（R05）は、対比が 3 回以上あり、対比の数を総文数で割った比率が 2% 以上の文書で指摘します。比率が 5% 以上なら重大、5% 未満なら情報です。既定では警告の帯はなく、区切りはジャンルによらず同じです。設定項目は次のとおりです。

| 設定 | 意味 |
| --- | --- |
| `min_ratio`（新設、既定 0.02） | 指摘する比率の下限。これ未満の文書は回数が多くても指摘しない |
| `error_above`（既定 0.05。以前は 0.03、技術文書は 0.045） | この比率以上を重大にする |
| `info_below`（既定は指定なし） | 指定したときだけ、この比率以上・`error_above` 未満を警告にする。以前の既定の警告の帯（2〜3%）に戻すなら `info_below = 0.02` と `error_above = 0.03` を書く |

## 結びの定型の有効化と設定の移行

「参考になれば幸いです」「お役に立てれば幸いです」は、チャットの応答に限らないため、P18 から P27（結びの定型）へ移しました。言い換えも P27 で拾います。P27 は `--genre tech`（設定の `genre = "tech"` も同じ）の既定の検査とフックで情報として出ます。ほかのジャンルとジャンル未指定では実験的なので、`--experimental`、`--enable-rules P27`、または設定の `[rules] enable = ["P27"]` で有効にしてください。技術記事の人の文書50本中2本に一致し、指摘を読み返して結びを見直す情報として確認しました。生成文書の判別には使いません。不要なら `[rules] disable = ["P27"]` で止められます。

結びだけを扱うために P18 を指定していた場合は、次のように移します。P18 のほかの項目も扱う場合は、P18 を残して P27 を追加します。

| 目的 | 移行後 |
| --- | --- |
| 明示的な有効化・無効化 | `--enable-rules P27` / `--ignore-rules P27`、設定の `enable` / `disable` に `P27` を指定する |
| 重大度の上書き | `[rules.P27] severity = "info"` などを指定する |
| 抑制コメント | `noslop-disable` / `noslop-disable-next-line` / `noslop-enable` で、結びの定型を扱う ID を `P27` にする |

文書の途中も検査しますが、依頼文や必要な挨拶は残せます。既定で有効にするジャンルは、人の文章での出方を校正してから決めます。
