//! 語句辞書で動くルール (P01〜P12、P18・P19) の定義。
//!
//! 校正済み (`Stable`) の項目は、人間と AI のコーパスで誤検知率を確かめた語句。
//! 人間の文章にも一定数出た語句は重大度を情報 (info) に下げ、弱い手掛かりとして扱う。
//! 実験的 (`Experimental`) の項目は、手癖として知られているが誤検知率を測っていない語句で、
//! `--experimental` か設定で明示したときだけ使う。

use crate::diagnostic::{Lane, RuleStatus, Severity};
use crate::rules::RuleMeta;

use super::engine::{Entry, PhraseSpec};

use Severity::{Info, Warning};

// ---------------------------------------------------------------------------
// P01 AI_CONCLUSION
// ---------------------------------------------------------------------------

const P01_ENTRIES: &[Entry] = &[
    Entry::lit("と言えるでしょう", Warning),
    Entry::lit("と言えるだろう", Warning),
    Entry::lit("と言えます", Warning),
    Entry::lit("ということになるでしょう", Warning),
    Entry::lit("のではないでしょうか", Warning),
    Entry::lit("結論から言うと", Warning),
    Entry::lit("結論として", Warning),
    Entry::lit("まとめると", Warning),
    Entry::lit("総じて", Warning),
    Entry::lit("いかがでしたか", Warning),
    Entry::lit("いかがでしょうか", Warning),
    Entry::exp_re(r"(?:と言っても)?過言では(?:ない|ありません)", Info),
    Entry::exp_re(
        r"以上、.{1,40}?(?:について|を)(?:解説|紹介|説明)しました",
        Info,
    ),
];

pub(super) static P01: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P01",
        name: "AI_CONCLUSION",
        title: "結論の押し付け・まとめ口調",
        lane: Lane::Slop,
        status: RuleStatus::Stable,
        default_severity: Warning,
        summary: "「と言えるでしょう」「まとめると」のような、結論を定型句で押し付ける締めを指摘する",
        explanation: r"### 何を見るか

「と言えるでしょう」「のではないでしょうか」「まとめると」「いかがでしたか」のように、結論を定型句で包んで差し出す言い回しを探します。

### なぜ問題か

要約の定型として大量に学習された言い回しで、生成された文章ほど段落の終わりに現れます。読み手が判断する前に結論だけを先回りして渡すので、言い切ってもいないのに押し付けがましく響きます。「のではないでしょうか」は断定を避ける逃げ道として使われやすく、重ねるほど意見のない文章に見えます。

### 直し方

定型句を外して言い切るか、結論を支える事実や数値をそのまま書きます。言い切れないなら、何が分かっていないのかを書きます。

### 例

- 直す前: この改善によって、チームの負担は大きく減ったと言えるでしょう。
- 直した後: この改善で、週次の手作業は 6 時間から 1 時間に減った。

### 根拠

既定で有効な語句は、人間とAIのコーパスで AI 側に偏って出ることを確かめたものです。「いかがでしょうか」「まとめると」は AI 側が優勢でした。業務メールの「ご都合はいかがでしょうか」のように自然な用法もあるので、文脈で判断してください。「過言ではない」と「以上、〜について解説しました」は未校正の実験的な項目です。",
    },
    entries: P01_ENTRIES,
    message: "「{m}」は結論を定型句で押し付ける締めです",
    hint: "定型句を外して言い切るか、結論を支える事実や数値を書いてください",
};

// ---------------------------------------------------------------------------
// P02 AI_PREFACE
// ---------------------------------------------------------------------------

const P02_ENTRIES: &[Entry] = &[
    Entry::weak("さて、"),
    Entry::lit("それでは、", Warning),
    Entry::lit("ここで注目したいのは", Warning),
    Entry::lit("見ていきましょう", Warning),
    Entry::lit("紹介していきます", Warning),
    Entry::lit("解説していきます", Warning),
    Entry::lit("深掘りしていきます", Warning),
    Entry::lit("について見ていく", Warning),
    Entry::exp_re(r"今回は.{1,30}?について(?:紹介|解説|説明)します", Info),
    Entry::exp_re(r"本(?:記事|稿|章)では.{1,40}?(?:紹介|解説|説明|論じ)", Info),
    // 上の正規表現と同じ位置から始まる場合は、長い正規表現の一致が優先される
    Entry::exp_lit("本記事では", Info),
    Entry::exp_lit("まず初めに、", Info),
    Entry::exp_re(r"近年、.{0,40}?(?:ますます|一層)", Info),
];

pub(super) static P02: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P02",
        name: "AI_PREFACE",
        title: "定型の前置き・予告",
        lane: Lane::Slop,
        status: RuleStatus::Stable,
        default_severity: Warning,
        summary: "「見ていきましょう」「解説していきます」のような、中身を運ばない前置きと予告を指摘する",
        explanation: r"### 何を見るか

「それでは、」「ここで注目したいのは」「見ていきましょう」「解説していきます」のように、これから何をするかを予告するだけの言い回しを探します。

### なぜ問題か

予告は次の話題へ移る合図として便利ですが、情報を 1 つも運びません。人が書く文章では、話題が変わるきっかけはふつう新しい事実や問いで、合図の言葉だけで次へ進むことはあまりありません。予告が続くと、読み手は本題にたどり着くまで待たされます。

### 直し方

予告の一文を消し、伝えたい内容そのものから書き始めます。前の段落とのつながりは、合図の語ではなく内容で作ります。

### 例

- 直す前: それでは、新しい料金体系を見ていきましょう。
- 直した後: 新しい料金体系では、月額の区分が 3 段階から 2 段階になる。

### 根拠

既定で有効な語句は、人間とAIのコーパスで確かめたものです。「さて、」は人間の文章にも一定数出たため、情報に下げています。「今回は〜について紹介します」「本記事では〜」「まず初めに、」「近年、〜ますます」は未校正の実験的な項目です。「本記事では」は、複数のモデルに同じお題で書かせた記事を数えた公開の小規模な調査で、生成された記事に多く出た語の一つですが、noslop のコーパスでは校正していません。",
    },
    entries: P02_ENTRIES,
    message: "「{m}」は中身を運ばない前置き・予告です",
    hint: "予告を消して、伝えたい内容そのものから書き始めてください",
};

// ---------------------------------------------------------------------------
// P03 AI_CONJUNCTION
// ---------------------------------------------------------------------------

const P03_ENTRIES: &[Entry] = &[
    Entry::weak("このように"),
    Entry::lit("このような中", Warning),
    Entry::exp_re(r"つまり、.{0,60}?ということ(?:でもあります|です)", Info),
];

pub(super) static P03: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P03",
        name: "AI_CONJUNCTION",
        title: "空疎な接続",
        lane: Lane::Slop,
        status: RuleStatus::Stable,
        default_severity: Info,
        summary: "「このように」「このような中」のような、前の内容を機械的に束ねる接続を指摘する",
        explanation: r"### 何を見るか

「このように」「このような中」のように、前の内容をひとまとめにして次へ渡す接続を探します。

### なぜ問題か

前の段落で何が述べられたかに関係なく使える語で、書き手の思考の跡が残りません。段落の頭がこの手の接続ばかりになると、論理のつながりを語で取り繕っているように読めます。言い換えで同じ内容を繰り返す「つまり、〜ということです」も、情報を増やさずに字数だけを増やします。

### 直し方

接続語を外しても前後がつながるか確かめます。つながらないなら、前の段落の具体的な事実や疑問を受けて次の話を始めます。

### 例

- 直す前: このように、在庫管理の自動化には多くの利点がある。
- 直した後: 発注の入力が要らなくなり、欠品は四半期で 12 件から 2 件に減った。

### 根拠

「このように」は人間の文章にも一定数出たため、情報に下げた弱い手掛かりです。「このような中」は警告のままです。「つまり、〜ということです」は未校正の実験的な項目です。",
    },
    entries: P03_ENTRIES,
    message: "「{m}」は前の内容を機械的に束ねる接続です",
    hint: "接続語に頼らず、前の内容の具体的な事実や疑問から次の話へつないでください",
};

// ---------------------------------------------------------------------------
// P04 REDUNDANT_VERB
// ---------------------------------------------------------------------------

const P04_ENTRIES: &[Entry] = &[
    Entry::exp_lit("を行う", Info),
    Entry::exp_lit("を行います", Info),
    Entry::exp_lit("を行った", Info),
    Entry::exp_lit("を行って", Info),
    Entry::exp_lit("を実施する", Info),
    Entry::exp_lit("を実施します", Info),
    Entry::exp_lit("を提供する", Info),
    Entry::exp_lit("を提供します", Info),
    Entry::exp_lit("という形になります", Info),
    Entry::exp_lit("的な部分", Info),
    Entry::exp_lit("を活用する", Info),
    Entry::exp_lit("を活用し", Info),
];

pub(super) static P04: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P04",
        name: "REDUNDANT_VERB",
        title: "冗長な動詞表現",
        lane: Lane::Readability,
        status: RuleStatus::Experimental,
        default_severity: Info,
        summary: "「〜を行う」「〜を実施する」のような、動作を名詞にして間延びさせた表現を指摘する (実験的)",
        explanation: r"### 何を見るか

「確認を行う」「調査を実施する」「という形になります」「的な部分」のように、動作をいったん名詞にしてから汎用の動詞で受ける表現を探します。

### なぜ問題か

一つひとつは誤りではありませんが、積み重なると同じ情報を伝えるのに必要な字数と読む時間が増えます。「〜を行う」は「〜する」で足りることが多く、クッションの語を重ねるほど文が間延びします。

### 直し方

「〜を行う」「〜を実施する」を「〜する」に縮められないか確かめます。手順書で動作を名詞として立てたい場合など、残す理由があれば残します。

### 例

- 直す前: 毎朝、設定の確認を行う。
- 直した後: 毎朝、設定を確認する。

### 根拠

冗長表現を網羅的な辞書で拾う方法は、上手な人間の文章でも 4 分の 1 が発火し (誤検知率 25.5%)、既定では使わない判断がされています。このルールは実験的で、`--experimental` か設定で有効にしたときだけ動きます。AI 臭さではなく読みやすさの指摘なので、自然度スコアには入りません。「を活用する」は、同じお題で複数のモデルに書かせた記事を数えた公開の小規模な調査で、生成された記事に多く出た語の一つです (noslop のコーパスでは未校正)。",
    },
    entries: P04_ENTRIES,
    message: "「{m}」は動作を名詞にして汎用の動詞で受けた、間延びした表現です",
    hint: "「〜を行う」「〜を実施する」は「〜する」に縮められないか確かめてください",
};

// ---------------------------------------------------------------------------
// P05 REDUNDANT_MODIFIER
// ---------------------------------------------------------------------------

const P05_ENTRIES: &[Entry] = &[
    Entry::exp_lit("様々な", Info),
    Entry::exp_lit("さまざまな", Info),
    Entry::exp_lit("多様な", Info),
    Entry::exp_lit("多くの", Info),
    Entry::exp_lit("一定の", Info),
    Entry::exp_lit("適切な", Info),
];

pub(super) static P05: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P05",
        name: "REDUNDANT_MODIFIER",
        title: "ぼやけた修飾語",
        lane: Lane::Slop,
        status: RuleStatus::Experimental,
        default_severity: Info,
        summary: "「様々な」「多くの」のような、具体を省いた修飾語を指摘する (実験的)",
        explanation: r"### 何を見るか

「様々な」「多様な」「多くの」「一定の」「適切な」のように、数や中身を言わずに量や幅、よし悪しだけを示す修飾語を探します。

### なぜ問題か

具体的な数や固有名詞を持っていないときに、それらしく見せる語として便利に使われます。書き手が素材を持っていないことの表れであることが多く、文体を整えても中身は増えません。

### 直し方

数・固有名詞・具体例に置き換えます。置き換える材料がないなら、言い回しではなく素材の不足として扱い、先に情報を集めます。

### 例

- 直す前: 様々な部署から多くの要望が寄せられた。
- 直した後: 営業部と経理部から、あわせて 14 件の要望が寄せられた。

### 根拠

具体を省く癖として目視で確かめる候補に挙がっている語で、誤検知率は測っていません。「適切な」は、同じお題で複数のモデルに書かせた記事を数えた公開の小規模な調査で、生成された記事に最も多く出た語でした。どれも日常の文章に普通に出る語なので、実験的なルールとして `--experimental` か設定で有効にしたときだけ動きます。",
    },
    entries: P05_ENTRIES,
    message: "「{m}」は数や中身を言わずに幅だけを示す修飾語です",
    hint: "数・固有名詞・具体例に置き換えられないか確かめてください",
};

// ---------------------------------------------------------------------------
// P06 OVER_EMPHASIS
// ---------------------------------------------------------------------------

const P06_ENTRIES: &[Entry] = &[
    Entry::lit("非常に重要", Warning),
    Entry::lit("極めて重要", Warning),
    Entry::lit("大切なのは", Warning),
    Entry::lit("言うまでもなく", Warning),
    Entry::lit("言うまでもありません", Warning),
    Entry::lit("まさしく", Warning),
    Entry::weak("重要なのは"),
    Entry::weak("ポイントは"),
    // 「非常に重要です」のように前に強調語があれば、先に始まる長い一致が優先される
    Entry::exp_lit("重要です", Info),
];

pub(super) static P06: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P06",
        name: "OVER_EMPHASIS",
        title: "過剰な強調",
        lane: Lane::Slop,
        status: RuleStatus::Stable,
        default_severity: Warning,
        summary: "「非常に重要」「言うまでもなく」のような、語で強調を上乗せする表現を指摘する",
        explanation: r"### 何を見るか

「非常に重要」「極めて重要」「大切なのは」「言うまでもなく」のように、重要さを語で言い立てる表現を探します。

### なぜ問題か

強調は他と比べて際立つ箇所にだけあるから効きます。生成された文章は全体の温度を均そうとして、ほぼすべての段落に強調語を置きがちです。強調が均等に散らばると、どこが本当に大事なのかが逆に分からなくなります。

### 直し方

強調語を外し、その一文を短く言い切ります。重要だと言うなら、重要である理由 (起きること・失うもの) を書きます。

### 例

- 直す前: ここで非常に重要なのは、バックアップの頻度です。
- 直した後: バックアップは 1 時間ごとに取る。1 日 1 回だと、障害時に最大 23 時間分のデータを失う。

### 根拠

既定で有効な語句は、人間とAIのコーパスで確かめたものです。「重要なのは」「ポイントは」は人間の文章にも一定数出たため、情報に下げています。「まさに」は人間の方が多く使っていたため辞書に入れていません。「重要です」は、同じお題で複数のモデルに書かせた記事を数えた公開の小規模な調査で生成された記事に多く出た語ですが、人間の文章にもよく出るため、未校正の実験的な項目として情報で扱います。",
    },
    entries: P06_ENTRIES,
    message: "「{m}」は語で強調を上乗せする表現です",
    hint: "強調語を外して短く言い切るか、重要である理由を書いてください",
};

// ---------------------------------------------------------------------------
// P07 HEDGING
// ---------------------------------------------------------------------------

const P07_ENTRIES: &[Entry] = &[
    Entry::lit("一概には言えません", Warning),
    Entry::lit("個人差がありますが", Warning),
    Entry::lit("あくまで一例ですが", Warning),
    Entry::exp_lit("という側面もあります", Info),
    Entry::exp_lit("とも限りません", Info),
    Entry::exp_lit("と言われています", Info),
    Entry::exp_lit("という声もあります", Info),
    Entry::exp_lit("ではないかと思います", Info),
];

pub(super) static P07: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P07",
        name: "HEDGING",
        title: "予防線・免責",
        lane: Lane::Slop,
        status: RuleStatus::Stable,
        default_severity: Warning,
        summary: "「一概には言えませんが」「個人差がありますが」のような、責任をぼかす予防線を指摘する",
        explanation: r"### 何を見るか

「一概には言えません」「個人差がありますが」「あくまで一例ですが」のように、断定を避けるための前置きを探します。

### なぜ問題か

事実を断定しすぎないよう気を配ること自体は、悪いことではありません。ただ、反論されにくい書き方を無意識に積み上げると、何も言っていない文章になります。予防線が続くと、読み手はどこまでが書き手の判断なのかを読み取れません。

### 直し方

言い切れない理由を、条件として具体的に書きます。条件を書けないなら予防線を外し、推定であることを「推定」と明示します。

### 例

- 直す前: 一概には言えませんが、この方法は多くの場合で有効です。
- 直した後: データが 1 万件未満なら、この方法で十分に速い。

### 根拠

既定で有効な語句は、人間とAIのコーパスで確かめたものです。「という側面もあります」「と言われています」「という声もあります」「ではないかと思います」などは、目視で確かめる候補に挙がっている未校正の実験的な項目です。",
    },
    entries: P07_ENTRIES,
    message: "「{m}」は責任をぼかす予防線です",
    hint: "言い切れない理由を条件として具体的に書くか、予防線を外してください",
};

// ---------------------------------------------------------------------------
// P08 EMPTY_ADJECTIVE
// ---------------------------------------------------------------------------

const P08_ENTRIES: &[Entry] = &[
    Entry::lit("核心的", Warning),
    Entry::lit("鍵となる", Warning),
    Entry::lit("根本的な", Warning),
    Entry::lit("多角的", Warning),
    Entry::lit("包括的", Warning),
    Entry::lit("総合的", Warning),
    Entry::weak("不可欠"),
];

pub(super) static P08: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P08",
        name: "EMPTY_ADJECTIVE",
        title: "空虚な形容",
        lane: Lane::Slop,
        status: RuleStatus::Stable,
        default_severity: Warning,
        summary: "「多角的」「包括的」「鍵となる」のような、中身を説明せずに重みだけを足す形容を指摘する",
        explanation: r"### 何を見るか

「核心的」「鍵となる」「根本的な」「多角的」「包括的」「総合的」「不可欠」のように、主張の中身を言わずに重要さや網羅の感触だけを足す形容を探します。

### なぜ問題か

「多角的に分析した」と書いても、どの角度から見たのかを書かない限り情報は増えません。「〜は不可欠だ」だけでは、何がどう欠かせないのかが分かりません。重みを演出する語が並ぶほど、中身が薄いことが目立ちます。

### 直し方

形容語の代わりに、実際に見た観点・根拠・影響を書きます。

### 例

- 直す前: 多角的な視点から包括的に分析した。
- 直した後: 売上・解約率・問い合わせ件数の 3 つを月ごとに並べて比べた。

### 根拠

既定で有効な語句は、人間とAIのコーパスで確かめたものです。「不可欠」は人間の文章にも一定数出たため、情報に下げています。",
    },
    entries: P08_ENTRIES,
    message: "「{m}」は中身を説明せずに重みだけを足す形容です",
    hint: "何がどう重要・網羅的なのか、観点や根拠そのものを書いてください",
};

// ---------------------------------------------------------------------------
// P09 EMPTY_VERB
// ---------------------------------------------------------------------------

const P09_ENTRIES: &[Entry] = &[
    Entry::lit("掘り下げる", Warning),
    Entry::lit("深掘りする", Warning),
    Entry::lit("言語化する", Warning),
    Entry::lit("を探求する", Warning),
    Entry::lit("正面から扱う", Warning),
    Entry::lit("正面から見る", Warning),
    Entry::lit("正面から書く", Warning),
    Entry::lit("正面から立てる", Warning),
    Entry::lit("正面から回収する", Warning),
    // 活用違い。「深掘りしていきます」は P02 が拾うので、ここでは「して」単独を含めない
    Entry::exp_re(r"掘り下げ(?:て|ます|ました|たい|ている|ていく)", Info),
    Entry::exp_re(
        r"深掘り(?:します|しました|したい|している|していた|しよう|すべき|できる)",
        Info,
    ),
    Entry::exp_re(
        r"言語化(?:して|します|しました|したい|している|できる|できない)",
        Info,
    ),
];

pub(super) static P09: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P09",
        name: "EMPTY_VERB",
        title: "作業の宣言だけの動詞",
        lane: Lane::Slop,
        status: RuleStatus::Stable,
        default_severity: Warning,
        summary: "「深掘りする」「言語化する」「正面から扱う」のような、作業をしたことだけを宣言する動詞を指摘する",
        explanation: r"### 何を見るか

「掘り下げる」「深掘りする」「言語化する」「探求する」「正面から扱う」のように、何をどう扱ったかを示さずに、扱ったという姿勢だけを宣言する動詞を探します。

### なぜ問題か

「この章では課題を深掘りする」と宣言しても、何が分かったのかは次の文を読むまで一つも伝わりません。姿勢の宣言は、論証の実質の代わりに「きちんと書いている感じ」を出すために使われやすく、宣言が多いほど中身との落差が目立ちます。

### 直し方

宣言の一文を消し、掘り下げた結果・言葉にした内容・扱う論点そのものから書き始めます。

### 例

- 直す前: 本章では、この遅延の問題を深掘りする。
- 直した後: 遅延は、夜間バッチが朝 9 時までに終わらないことから起きている。

### 根拠

既定で有効な語句は、技術文書の書き方の規範で「姿勢だけの宣言」とされる言い回しを含め、人間とAIのコーパスで確かめたものです。「掘り下げて」「深掘りします」「言語化できる」などの活用違いは未校正の実験的な項目です。",
    },
    entries: P09_ENTRIES,
    message: "「{m}」は作業をしたことだけを宣言する動詞です",
    hint: "宣言を消して、掘り下げた結果や扱う論点そのものを書いてください",
};

// ---------------------------------------------------------------------------
// P10 DRAMATIC_CLOSER
// ---------------------------------------------------------------------------

const P10_ENTRIES: &[Entry] = &[
    Entry::exp_lit("に他なりません", Info),
    Entry::exp_re(r"これこそが.{0,20}?(?:真髄|本質|醍醐味)", Info),
    Entry::exp_lit("ところまでが仕事", Info),
];

pub(super) static P10: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P10",
        name: "DRAMATIC_CLOSER",
        title: "演出的な決め文",
        lane: Lane::Slop,
        status: RuleStatus::Experimental,
        default_severity: Info,
        summary: "「これこそが〜の本質です」「〜ところまでが仕事です」のような、語り口で結論を演出する決め文を指摘する (実験的)",
        explanation: r"### 何を見るか

「に他なりません」「これこそが〜の本質」「〜ところまでが仕事です」のように、決め台詞で文末を飾る言い回しを探します。

### なぜ問題か

根拠の強さではなく、言い回しの切れ味で読み手を押し切ろうとする書き方です。決め文で締めるほど、書き手が自分の言い回しに酔っているように読まれやすく、内容への信頼を損ねます。

### 直し方

決め台詞を外し、事実を淡々と書きます。「〜の本質」と言いたいなら、なぜそれが本質なのかを示す事実を書きます。

### 例

- 直す前: この一手こそが、改善の本質に他なりません。
- 直した後: この一手で、改善の効果の大半が出た。

### 根拠

手癖として知られている言い回しですが、コーパスでの誤検知率は測っていません。実験的なルールとして `--experimental` か設定で有効にしたときだけ動きます。「と言っても過言ではない」は P01 の実験的な項目、「に他ならない」は P12 が扱います。",
    },
    entries: P10_ENTRIES,
    message: "「{m}」は語り口で結論を演出する決め文です",
    hint: "決め台詞で締めず、事実を淡々と書いてください",
};

// ---------------------------------------------------------------------------
// P11 COUNT_DECLARATION
// ---------------------------------------------------------------------------

const P11_ENTRIES: &[Entry] = &[
    Entry::exp_re(
        r"(?:理由|ポイント|要因|メリット|デメリット|課題|方法|特徴|軸|要素|観点)は(?:主に|大きく)?[0-9０-９一二三四五六七八九十]+つ(?:あります|です|ある|だ)",
        Info,
    ),
    Entry::exp_re(r"大きく分けて[0-9０-９一二三四五六七八九十]+つ", Info),
    Entry::exp_re(r"以下の[0-9０-９一二三四五六七八九十]+(?:つ|点)", Info),
];

pub(super) static P11: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P11",
        name: "COUNT_DECLARATION",
        title: "プレゼン的な数の予告",
        lane: Lane::Slop,
        status: RuleStatus::Experimental,
        default_severity: Info,
        summary: "「理由は3つあります」「以下の3点」のような、項目の数を先に宣言する書き出しを指摘する (実験的)",
        explanation: r"### 何を見るか

「理由は3つあります」「ポイントは主に2つです」「大きく分けて3つ」「以下の3点」のように、これから並べる項目の数を先に宣言する書き出しを探します。

### なぜ問題か

発表資料や要約の型としてよく学習されているため、生成された文章に強く出ます。文章として読むと教科書的に響き、書き手の体温が消えます。数を先に決めると、本当は 2 つしかない論点や 4 つ目がある論点まで、宣言した数に丸められがちです。

### 直し方

数を宣言せず、理由や要素の中身から地続きに書きます。本当に並列な項目なら、数を言わずに並べれば足ります。

### 例

- 直す前: この方式を採用した理由は3つあります。
- 直した後: この方式を採用したのは、既存の認証基盤とそのまま連携できたからだ。

### 根拠

手癖として知られている型ですが、コーパスでの誤検知率は測っていません。実験的なルールとして `--experimental` か設定で有効にしたときだけ動きます。",
    },
    entries: P11_ENTRIES,
    message: "「{m}」は項目の数を先に宣言するプレゼン調の書き出しです",
    hint: "数を先に宣言せず、理由や要素の中身から書いてください",
};

// ---------------------------------------------------------------------------
// P12 TRANSLATIONESE
// ---------------------------------------------------------------------------

const P12_ENTRIES: &[Entry] = &[
    Entry::re(r"することができ(?:る|ます|た)", Info),
    Entry::re(r"することが可能(?:です|だ|になる)", Info),
    Entry::lit("という点で", Info),
    Entry::re(r"という観点(?:から|で)", Info),
    Entry::re(r"にとって(?:重要|不可欠)", Info),
    Entry::re(r"を持つ(?:こと|存在)", Info),
    Entry::lit("することによって", Info),
    Entry::lit("であることは間違いない", Info),
    Entry::lit("に他ならない", Info),
    // 元は品詞列 (こと + が/は + 「でき」で始まる動詞) で照合していたものの表層での近似。
    // 「できる」の活用を並べる。regex の選択肢は先に書いたほうが勝つので、長い語形を先に置く。
    // 末尾の `\b` は連用中止形 (「〜ことができ、」) を読点を含めずに拾う。「出来事」「出来高」は
    // 「来」の後ろが単語境界にならないので拾わない
    Entry::re(
        r"こと(?:が|は)(?:でき|出来)(?:ませんでした|ません|ましょう|ました|ます|なかった|なければ|なさそう|なく|ない|ず|ぬ|ん|まい|よう|そう|れば|る|た|て|\b)",
        Info,
    ),
];

pub(super) static P12: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P12",
        name: "TRANSLATIONESE",
        title: "翻訳調",
        lane: Lane::Slop,
        status: RuleStatus::Stable,
        default_severity: Info,
        summary: "「〜することができる」「〜という観点から」のような、英語の構文を直訳した言い回しを指摘する",
        explanation: r"### 何を見るか

「〜することができる」「〜することが可能です」「〜という点で」「〜という観点から」「〜にとって重要」「〜を持つこと」「〜することによって」「〜であることは間違いない」「〜に他ならない」のように、英語の構文をそのまま置き換えたような言い回しを探します。「〜することができる」は、「できない」「できません」「できました」「できれば」などの活用と、「〜することはできる」の形も含みます。表記は「できる」と「出来る」のどちらも拾います。

### なぜ問題か

どれも文法的には正しい日本語ですが、その言い回しを選ぶ必然性が文脈にありません。英語の can・in terms of・by doing などを機械的に移した形で、できるかどうかが問題になっていない文にまで「することができる」を付けると、取扱説明書のような硬い調子が文章全体に広がります。

### 直し方

「〜できる」「〜で見ると」「〜すると」「〜には〜がある」のように、より直接的な言い方に置き換えられないか試します。仕様書で可能・不可能を正確に書き分けたい場合など、残す理由があれば残します。

### 例

- 直す前: このツールを使うことによって、集計を自動化することができる。
- 直した後: このツールを使えば、集計を自動化できる。

### 根拠

人間とAIのコーパスで確かめた言い回しで、重大度は情報です。元の検出器は表層の正規表現と品詞列 (こと + が/は + 「でき」で始まる動詞) の両方で照合していました。辞書を使わないため、品詞列の側は「ことが」「ことは」に続く「できる」の活用形の表層で近似しています。元の定義の範囲に合わせて、否定形 (「できない」「できません」「できず」「できなかった」)、「できました」、仮定形と連用形 (「できれば」「できて」「〜ことができ、」) と、「ことは＋できる」の形まで拾います。",
    },
    entries: P12_ENTRIES,
    message: "「{m}」は英語の構文を直訳したような言い回しです",
    hint: "「〜できる」「〜で見ると」「〜すると」など、より直接的な言い方に置き換えられないか確かめてください",
};

// ---------------------------------------------------------------------------
// P18 CHAT_RESIDUE
// ---------------------------------------------------------------------------

const P18_ENTRIES: &[Entry] = &[
    // 書き出しの名残: 文書の読み手は何も尋ねていないので、文書の中では意味をなさない
    Entry::exp_lit("ご質問ありがとうございます", Warning),
    Entry::exp_re(r"承知(?:いた)?しました", Warning),
    Entry::exp_lit("かしこまりました", Warning),
    Entry::exp_re(r"^もちろんです", Warning),
    Entry::exp_re(
        r"以下に.{0,20}?(?:まとめ|示し|記載し|整理し|列挙し)ました",
        Warning,
    ),
    Entry::exp_re(r"喜んで.{0,10}?(?:お手伝い|お答え)", Warning),
    // 結びの名残: 人間のブログの結びの挨拶とも重なるので情報に留める
    Entry::exp_lit("お役に立てれば幸いです", Info),
    Entry::exp_lit("参考になれば幸いです", Info),
    Entry::exp_re(r"ご不明な点が(?:あれば|ございましたら)", Info),
    Entry::exp_re(r"何かあれば(?:お気軽に|いつでも)", Info),
    Entry::exp_re(
        r"(?:他|ほか)に(?:ご質問|知りたいこと|気になること)が(?:あれば|ございましたら)",
        Info,
    ),
];

pub(super) static P18: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P18",
        name: "CHAT_RESIDUE",
        title: "チャット応答の名残",
        lane: Lane::Slop,
        status: RuleStatus::Experimental,
        default_severity: Warning,
        summary: "「ご質問ありがとうございます」「お役に立てれば幸いです」のような、チャットの応答をそのまま貼った名残を指摘する (実験的)",
        explanation: r"### 何を見るか

チャットで AI に答えさせた文をそのまま文書に貼ったときに残る言い回しを探します。書き出しの名残 (「ご質問ありがとうございます」「承知しました」「もちろんです」「以下に〜をまとめました」) は警告、結びの名残 (「お役に立てれば幸いです」「ご不明な点があれば」「他にご質問があれば」) は情報です。

### なぜ問題か

文書の読み手は何も尋ねていないので、質問へのお礼や引き受けの返事は、誰に向けた言葉なのかが分からなくなります。応答の型が残っていると、中身を読む前に「チャットの出力を吟味せずに貼った」と受け取られます。結びの挨拶は、人が書くブログの締めにもよく出る言い回しなので、書き出しより弱い手掛かりとして扱います。

### 直し方

書き出しの名残は削り、本題の一文から始めます。結びは、文書の読み手に向けた言葉 (次に何をすればよいか、どこに問い合わせるか) に書き換えるか、要らなければ削ります。

### 例

- 直す前: ご質問ありがとうございます。以下に、経費精算の手順をまとめました。
- 直した後: 経費精算は、月末の 3 営業日前までに申請画面から出す。

### 根拠

実験的です。文書の中に応答の型が残るのは生成された文章に特有の形ですが、コーパスでの誤検知率は測っていません。お客さまへのメールや問い合わせへの回答文、FAQ では書き出しの言い回しも自然に使われるので、その種の文書では抑制コメントで残す理由を書いてください。",
    },
    entries: P18_ENTRIES,
    message: "「{m}」はチャットの応答をそのまま貼ったような言い回しです",
    hint: "文書の読み手に向けた文に書き換えるか、要らなければ削ってください",
};

// ---------------------------------------------------------------------------
// P19 HYPE
// ---------------------------------------------------------------------------

const P19_ENTRIES: &[Entry] = &[
    Entry::exp_lit("革命的", Info),
    Entry::exp_lit("ゲームチェンジャー", Info),
    Entry::exp_lit("世界初", Info),
    Entry::exp_lit("究極の", Info),
    Entry::exp_lit("完全に解決", Info),
    Entry::exp_lit("すべての課題を", Info),
    Entry::exp_re(r"最高の(?:品質|体験)", Info),
    Entry::exp_lit("魔法のように", Info),
    Entry::exp_lit("奇跡的", Info),
    Entry::exp_lit("可能性を解き放", Info),
    // 「民主化運動」のような歴史・政治の用法は拾わず、「〜を民主化する」だけにする
    Entry::exp_lit("を民主化", Info),
    Entry::exp_lit("スーパーチャージ", Info),
    // 「メソッドを再定義する」のような技術用語は拾わない
    Entry::exp_re(
        r"(?:業界|常識|未来|働き方|体験|あり方|ルール)を再定義",
        Info,
    ),
    Entry::exp_lit("未来を変え", Info),
    Entry::exp_lit("パラダイムシフト", Info),
    Entry::exp_lit("圧倒的な", Info),
    Entry::exp_lit("劇的に", Info),
    Entry::exp_lit("飛躍的に", Info),
    Entry::exp_lit("無限の可能性", Info),
];

pub(super) static P19: PhraseSpec = PhraseSpec {
    meta: RuleMeta {
        id: "P19",
        name: "HYPE",
        title: "誇張表現",
        lane: Lane::Slop,
        status: RuleStatus::Experimental,
        default_severity: Info,
        summary: "「革命的」「ゲームチェンジャー」「飛躍的に」のような、根拠を示さずに効果を大きく言う誇張を指摘する (実験的)",
        explanation: r"### 何を見るか

「革命的」「ゲームチェンジャー」「究極の」「パラダイムシフト」「飛躍的に」「無限の可能性」のように、効果や新しさを大きく言い立てる言葉を探します。「民主化運動」「メソッドを再定義する」のような普通の用法は拾いません。

### なぜ問題か

宣伝文の言い回しを大量に学習しているため、生成された文章は製品や手法の説明で大げさな形容に寄りがちです。誇張は数字や比較の代わりにならず、読み手は「どれくらい」「何と比べて」を知りたいまま置いていかれます。誇張が重なるほど、書かれた効果そのものが疑わしく見えます。

### 直し方

効果を数値か比較で示します。比べる材料がないなら、形容を外して起きたことを淡々と書きます。

### 例

- 直す前: この機能は、承認業務を劇的に変える革命的な仕組みです。
- 直した後: この機能で、承認にかかる時間は平均 2 日から半日になった。

### 根拠

実験的です。誇張表現を拾う既存の文章校正ツールのルールや実務者の観察で挙げられている語ですが、noslop のコーパスでの誤検知率は測っていません。宣伝文や広告の原稿では意図して使うこともあるので、残すなら理由を書いてください。",
    },
    entries: P19_ENTRIES,
    message: "「{m}」は根拠を示さずに効果を大きく言う誇張表現です",
    hint: "効果を数値や比較で示すか、形容を外して起きたことを書いてください",
};

#[cfg(test)]
mod tests {
    use crate::diagnostic::{RuleStatus, Severity};
    use crate::rules::Scope;
    use crate::rules::testing::{Options, matched, run, run_with};

    use super::super::engine::PhraseRule;
    use super::*;

    fn rule(spec: &'static PhraseSpec) -> PhraseRule {
        PhraseRule::new(spec)
    }

    fn experimental() -> Options {
        Options {
            experimental: true,
            ..Options::default()
        }
    }

    #[test]
    fn p01_flags_conclusion_phrases_with_exact_spans() {
        let md =
            "この改善で負担は減ったと言えるでしょう。\n\n結論として、**総じて**良い結果だった。\n";
        let d = run(&rule(&P01), md);
        assert_eq!(
            matched(md, &d),
            vec!["と言えるでしょう", "結論として", "総じて"]
        );
        assert!(d.iter().all(|d| d.severity == Severity::Warning));
        assert_eq!(d[0].rule_id, "P01");
        assert_eq!(d[0].rule_name, "AI_CONCLUSION");
        assert!(d[0].message.contains("「と言えるでしょう」"));
        assert!(d[0].hint.is_some());
        let ctx = d[0].context.expect("context");
        assert_eq!(&md[ctx.range()], "この改善で負担は減ったと言えるでしょう。");
    }

    #[test]
    fn p01_experimental_entries_are_gated() {
        let md = "これは革命と言っても過言ではない。\n";
        assert!(run(&rule(&P01), md).is_empty());
        let d = run_with(&rule(&P01), md, experimental());
        assert_eq!(matched(md, &d), vec!["と言っても過言ではない"]);
        assert_eq!(d[0].status, RuleStatus::Experimental);
        assert_eq!(d[0].severity, Severity::Info);
    }

    #[test]
    fn p01_ignores_natural_text() {
        let md = "週次の手作業は 6 時間から 1 時間に減った。最後に、担当者へ共有した。\n";
        assert!(run(&rule(&P01), md).is_empty());
    }

    #[test]
    fn scope_limits_matching_to_paragraphs_by_default() {
        let md = "- 結論として良い。\n\n> まとめると良い。\n\n| 表 |\n|---|\n| 総じて良い |\n";
        assert!(run(&rule(&P01), md).is_empty());
        let all = Options {
            scope: Scope::ALL,
            ..Options::default()
        };
        let d = run_with(&rule(&P01), md, all);
        assert_eq!(matched(md, &d), vec!["結論として", "まとめると", "総じて"]);
    }

    #[test]
    fn headings_are_never_scanned() {
        let md = "# まとめると\n\n本文です。\n";
        let all = Options {
            scope: Scope::ALL,
            ..Options::default()
        };
        assert!(run_with(&rule(&P01), md, all).is_empty());
    }

    #[test]
    fn p02_weak_signal_is_info_with_note() {
        let md = "さて、本題に入る。それでは、手順を見ていきましょう。\n";
        let d = run(&rule(&P02), md);
        assert_eq!(
            matched(md, &d),
            vec!["さて、", "それでは、", "見ていきましょう"]
        );
        assert_eq!(d[0].severity, Severity::Info);
        assert!(d[0].message.contains("弱い手掛かり"));
        assert_eq!(d[1].severity, Severity::Warning);
    }

    #[test]
    fn p02_experimental_preface_patterns() {
        let md = "今回は新機能について紹介します。まず初めに、概要を話す。\n";
        assert!(run(&rule(&P02), md).is_empty());
        let d = run_with(&rule(&P02), md, experimental());
        assert_eq!(
            matched(md, &d),
            vec!["今回は新機能について紹介します", "まず初めに、"]
        );
    }

    #[test]
    fn p03_distinguishes_weak_and_strong_entries() {
        let md = "このように整理できる。このような中で判断した。\n";
        let d = run(&rule(&P03), md);
        assert_eq!(matched(md, &d), vec!["このように", "このような中"]);
        assert_eq!(d[0].severity, Severity::Info);
        assert_eq!(d[1].severity, Severity::Warning);
    }

    #[test]
    fn experimental_rules_use_all_entries_when_run() {
        // P04 はルール自体が実験的なので、エンジンが有効にした時点で全項目を使う
        let md = "毎朝、設定の確認を行う。\n";
        let d = run(&rule(&P04), md);
        assert_eq!(matched(md, &d), vec!["を行う"]);
        assert_eq!(d[0].status, RuleStatus::Experimental);
        assert_eq!(P04.meta.lane, crate::diagnostic::Lane::Readability);
    }

    #[test]
    fn p05_flags_vague_modifiers() {
        let md = "様々な部署から多くの要望が来た。\n";
        let d = run(&rule(&P05), md);
        assert_eq!(matched(md, &d), vec!["様々な", "多くの"]);
    }

    #[test]
    fn p06_flags_emphasis() {
        let md = "ここで非常に重要なのは頻度だ。言うまでもなく、ポイントは速さだ。\n";
        let d = run(&rule(&P06), md);
        assert_eq!(
            matched(md, &d),
            vec!["非常に重要", "言うまでもなく", "ポイントは"]
        );
        assert_eq!(d[2].severity, Severity::Info);
    }

    #[test]
    fn p07_flags_hedges_and_gates_experimental() {
        let md = "一概には言えませんが、有効です。効果があると言われています。\n";
        let d = run(&rule(&P07), md);
        assert_eq!(matched(md, &d), vec!["一概には言えません"]);
        let d = run_with(&rule(&P07), md, experimental());
        assert_eq!(
            matched(md, &d),
            vec!["一概には言えません", "と言われています"]
        );
    }

    #[test]
    fn p08_flags_empty_adjectives() {
        let md = "多角的な視点から包括的に分析した。成功には対話が不可欠だ。\n";
        let d = run(&rule(&P08), md);
        assert_eq!(matched(md, &d), vec!["多角的", "包括的", "不可欠"]);
        assert_eq!(d[2].severity, Severity::Info);
    }

    #[test]
    fn p09_flags_empty_verbs_without_overlapping_p02() {
        let md = "この章では課題を深掘りする。姿勢を正面から扱う。\n";
        let d = run(&rule(&P09), md);
        assert_eq!(matched(md, &d), vec!["深掘りする", "正面から扱う"]);
        let md2 = "この問題を深掘りしていきます。論点を深掘りします。\n";
        let d = run_with(&rule(&P09), md2, experimental());
        assert_eq!(matched(md2, &d), vec!["深掘りします"]);
    }

    #[test]
    fn p10_flags_dramatic_closers() {
        let md = "この一手こそが改善の本質に他なりません。これこそが仕事の醍醐味だ。\n";
        let d = run(&rule(&P10), md);
        assert_eq!(
            matched(md, &d),
            vec!["に他なりません", "これこそが仕事の醍醐味"]
        );
    }

    #[test]
    fn p11_flags_count_declarations() {
        let md = "採用した理由は3つあります。大きく分けて二つの型がある。以下の3点を守る。\n";
        let d = run(&rule(&P11), md);
        assert_eq!(
            matched(md, &d),
            vec!["理由は3つあります", "大きく分けて二つ", "以下の3点"]
        );
    }

    #[test]
    fn p12_merges_overlapping_translationese() {
        let md = "ツールを利用することによって、集計を自動化することができる。読むことができる。\n";
        let d = run(&rule(&P12), md);
        assert_eq!(
            matched(md, &d),
            vec!["することによって", "することができる", "ことができる"]
        );
        assert!(d.iter().all(|d| d.severity == Severity::Info));
    }

    #[test]
    fn p12_flags_conjugations_of_dekiru_after_koto_ga_or_wa() {
        // 元の定義 (こと + が/は + 「でき」で始まる動詞) に入る活用を拾う
        let md = "設定を変えることができない。前の版に戻すことはできません。\
                  手順を省くことができました。早めに気づくことができれば、傷は浅い。\
                  会議で話すことができて安心した。期限までに出すことができず、延期した。\
                  当日は参加することができなかった。期限を延ばすことは出来ない。\n";
        let d = run(&rule(&P12), md);
        assert_eq!(
            matched(md, &d),
            vec![
                "ことができない",
                "ことはできません",
                "ことができました",
                "ことができれば",
                "ことができて",
                "ことができず",
                "ことができなかった",
                "ことは出来ない",
            ]
        );
        assert!(
            d.iter()
                .all(|d| d.severity == Severity::Info && d.status == RuleStatus::Stable)
        );
    }

    #[test]
    fn p12_flags_the_continuative_form_without_the_comma() {
        // 連用中止形は読点を範囲に含めない。装飾をまたぐ一致は原文の記号ごと範囲に含める
        let md = "画面から設定を変えることができ、再起動も要らない。`git` で**戻すことはでき**ません。\n";
        let d = run(&rule(&P12), md);
        assert_eq!(matched(md, &d), vec!["ことができ", "ことはでき**ません"]);
    }

    #[test]
    fn p12_ignores_forms_outside_the_original_definition() {
        for md in [
            // 「も」は元の定義 (が/は) にない
            "話すこともできる。\n",
            // 「出来事」「出来高」は動詞ではない
            "そのことが出来事の発端になった。\n",
            "このことが出来高を押し上げた。\n",
            // 「こと」と「できる」のあいだに読点がある
            "大事なことは、できるところから始めることだ。\n",
            // 語順が逆
            "できることから順に片づける。\n",
            // 既定ではリストを見ない
            "- 設定を変えることはできない。\n",
        ] {
            assert!(run(&rule(&P12), md).is_empty(), "{md}");
        }
    }

    #[test]
    fn spans_follow_the_source_through_markdown() {
        // 装飾をまたぐ一致は、原文の装飾記号ごと範囲に含める
        let md = "`cargo` で設定する**ことによって**、手順を短くすることができる。\n";
        let d = run(&rule(&P12), md);
        assert_eq!(
            matched(md, &d),
            vec!["する**ことによって", "することができる"]
        );
    }

    #[test]
    fn matches_do_not_cross_sentences() {
        let md = "結論。として続ける。\n";
        assert!(run(&rule(&P01), md).is_empty());
    }

    #[test]
    fn metric_records_the_matched_phrase() {
        let md = "結論として良い。\n";
        let d = run(&rule(&P01), md);
        assert_eq!(
            d[0].metrics.get("matched"),
            Some(&crate::diagnostic::Metric::Text("結論として".into()))
        );
    }

    #[test]
    fn measured_ai_words_are_experimental_entries() {
        // 既定では出ず、--experimental で出る (いずれも情報)
        let md = "本記事では、適切な方法でデータを活用することが重要です。\n";
        let md2 = "予算を活用する。\n";
        assert!(run(&rule(&P02), md).is_empty());
        assert!(run(&rule(&P06), md).is_empty());
        let d = run_with(&rule(&P02), md, experimental());
        assert_eq!(matched(md, &d), vec!["本記事では"]);
        let d = run_with(&rule(&P06), md, experimental());
        assert_eq!(matched(md, &d), vec!["重要です"]);
        assert_eq!(d[0].status, RuleStatus::Experimental);
        assert_eq!(d[0].severity, Severity::Info);
        // P04・P05 はルール自体が実験的
        assert_eq!(matched(md, &run(&rule(&P05), md)), vec!["適切な"]);
        assert_eq!(matched(md2, &run(&rule(&P04), md2)), vec!["を活用する"]);
    }

    #[test]
    fn new_entries_do_not_duplicate_longer_matches() {
        // 実験的な正規表現と同じ位置の「本記事では」は 1 件にまとまる
        let md = "本記事では新機能を紹介します。\n";
        let d = run_with(&rule(&P02), md, experimental());
        assert_eq!(d.len(), 1);
        assert_eq!(matched(md, &d), vec!["本記事では新機能を紹介"]);
        // 「非常に重要です」は校正済みの「非常に重要」だけになる
        let md = "非常に重要です。\n";
        let d = run_with(&rule(&P06), md, experimental());
        assert_eq!(matched(md, &d), vec!["非常に重要"]);
        assert_eq!(d[0].status, RuleStatus::Stable);
    }

    #[test]
    fn p18_distinguishes_openers_and_closers() {
        let md = "ご質問ありがとうございます。以下に、手順をまとめました。\n\nお役に立てれば幸いです。他にご質問があればどうぞ。\n";
        let d = run(&rule(&P18), md);
        assert_eq!(
            matched(md, &d),
            vec![
                "ご質問ありがとうございます",
                "以下に、手順をまとめました",
                "お役に立てれば幸いです",
                "他にご質問があれば"
            ]
        );
        assert_eq!(d[0].severity, Severity::Warning);
        assert_eq!(d[1].severity, Severity::Warning);
        assert_eq!(d[2].severity, Severity::Info);
        assert!(d.iter().all(|d| d.status == RuleStatus::Experimental));
        assert_eq!(P18.meta.status, RuleStatus::Experimental);
    }

    #[test]
    fn p18_only_flags_mochiron_at_sentence_start() {
        let md = "もちろんです！手順を説明します。\n";
        assert_eq!(matched(md, &run(&rule(&P18), md)), vec!["もちろんです"]);
        let md = "それはもちろんですが、先に確認する。\n";
        assert!(run(&rule(&P18), md).is_empty());
    }

    #[test]
    fn p18_ignores_ordinary_text() {
        let md = "経費精算は、月末の 3 営業日前までに申請する。問い合わせは経理部へ送る。\n";
        assert!(run(&rule(&P18), md).is_empty());
    }

    #[test]
    fn p19_flags_hype_but_not_ordinary_usage() {
        let md = "この機能は承認業務を劇的に変える革命的な仕組みで、働き方を再定義する。\n";
        let d = run(&rule(&P19), md);
        assert_eq!(matched(md, &d), vec!["劇的に", "革命的", "働き方を再定義"]);
        assert!(d.iter().all(|d| d.severity == Severity::Info));
        for md in [
            "民主化運動の歴史を調べた。\n",
            "サブクラスでメソッドを再定義する。\n",
            "次世代の担い手を育てる。\n",
        ] {
            assert!(run(&rule(&P19), md).is_empty(), "{md}");
        }
    }
}
