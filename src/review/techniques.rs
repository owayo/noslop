//! 文の形だけでは決まらない構成・伝達・推敲の検査。

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Prerequisite {
    Purpose,
    Audience,
    RequiredInformation,
}

#[derive(Debug, Serialize)]
pub struct Technique {
    pub id: &'static str,
    pub title: &'static str,
    pub check: &'static str,
    pub requires: &'static [Prerequisite],
}

pub const TECHNIQUES: &[Technique] = &[
    Technique {
        id: "purpose_coverage",
        requires: &[Prerequisite::Purpose, Prerequisite::RequiredInformation],
        title: "必要な情報",
        check: "目的とrequiredInformationを照らし、読者が必要な情報に到達できるか確認する。材料不足はunknown。必須でない情報まで不足としない。",
    },
    Technique {
        id: "lead",
        requires: &[Prerequisite::Purpose],
        title: "要点と説明の順序",
        check: "文書・節・段落の要点を先に知れるか。操作前に必要な前提が出るか。既知の背景や目的に必要な導入は残す。",
    },
    Technique {
        id: "relevance",
        requires: &[Prerequisite::Purpose],
        title: "段落の役割",
        check: "主題を支える根拠・背景・具体例・条件を区別する。目的と関係のない内容を根拠付きで示し、必要な説明を短文化のために落とさない。",
    },
    Technique {
        id: "spacing",
        requires: &[],
        title: "内容の区切り",
        check: "話題の切り替えが段落や空白で見えるか。文字の詰まった一続きの説明から必要な箇所を選べるか。折り返しや改行数を一律に指定しない。",
    },
    Technique {
        id: "hierarchy",
        requires: &[],
        title: "親子と並列の対応",
        check: "項目の包含関係と並列関係が構造と一致するか。予告・リスト・番号・条件を対応づける。子項目と独立項目を混ぜない。",
    },
    Technique {
        id: "grouping",
        requires: &[],
        title: "本文・例・条件の境界",
        check: "説明と例文、引用と書き手の主張、適用条件と操作が区別できるか。範囲が曖昧で誤読を生む位置を示す。",
    },
    Technique {
        id: "headings",
        requires: &[],
        title: "見出しから内容を選べるか",
        check: "読者が探す情報と見出し・題が対応するか。本文と見出しの内容が違わないか。短い文書に見出しを強制しない。",
    },
    Technique {
        id: "argument",
        requires: &[],
        title: "主張と根拠",
        check: "根拠の事実が示され、その根拠から結論へ進む前提が説明されているか。相関と因果、観測と推測、対象や比較基準を区別する。出典や数字の存在だけで支持としない。",
    },
    Technique {
        id: "audience",
        requires: &[Prerequisite::Audience],
        title: "読者が持つ知識",
        check: "audienceとknownTermsに照らして用語・前提が共有されているか。不明な読者像を仮定しない。説明が多すぎる箇所も確認する。",
    },
    Technique {
        id: "tone",
        requires: &[Prerequisite::Purpose],
        title: "評価・感情・事実の区別",
        check: "事実と書き手の評価・感情を読み分けられるか。目的に対して強すぎる断定や攻撃的な語調を示す。必要な主観・確信度・副詞の含みは保つ。",
    },
    Technique {
        id: "analogy",
        requires: &[Prerequisite::Purpose, Prerequisite::Audience],
        title: "例と比喩の働き",
        check: "抽象的な説明に必要な具体例や比喩があるか。比喩が対応を誤らせないか、比喩を実証された事実のように扱っていないか。全箇所へ例や比喩を足さない。",
    },
    Technique {
        id: "sentence_size",
        requires: &[],
        title: "一文に詰めた情報",
        check: "一文に独立した主張・長い修飾・括弧の説明を詰めていないか。分割しても主体・条件・否定・時制が保たれるか。一律の文字数は目標にしない。",
    },
    Technique {
        id: "boundaries",
        requires: &[],
        title: "意味の切れ目",
        check: "読点・区切り・並列表記によって修飾先や情報の関係を読み違えないか。必要な読点を数の都合で削らない。",
    },
    Technique {
        id: "ambiguity",
        requires: &[],
        title: "指示・条件・否定の範囲",
        check: "指示語や修飾語の参照先を候補とともに確認する。否定・比較・例外の対象、程度の基準が曖昧なら二つの解釈と根拠を示す。分かる主語を一律に補わない。",
    },
    Technique {
        id: "terminology",
        requires: &[],
        title: "定義と用語の対応",
        check: "複雑な説明の短い呼び名が定義され、同じ意味で使われるか。必要な定義が初出から遠くないか。用語・主題の必要な反復は残す。",
    },
    Technique {
        id: "redundancy",
        requires: &[],
        title: "削っても伝達が保たれるか",
        check: "意味の重複、不要な修飾・接続・語尾を示す。削ることで関係説明・確信度・敬意が失われないか確認する。長文の要旨再掲や注意の反復は残す。",
    },
    Technique {
        id: "flow",
        requires: &[],
        title: "語順と読み進め方",
        check: "語順により主体・時制・修飾先を途中で読み直す箇所がないか。音の反復が読みづらいかは文脈で判断し、同じ用語の使用自体は罰しない。",
    },
    Technique {
        id: "politeness",
        requires: &[Prerequisite::Purpose, Prerequisite::Audience],
        title: "相手と語調",
        check: "相手と目的に合う丁寧さか。尊敬・謙譲の対象や過剰な敬語によって主体が不明にならないか。常体や直接の依頼を一律に敬体へ変えない。",
    },
];
