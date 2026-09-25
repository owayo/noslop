# Third-Party Notices

## 日本語の文章作法に関する知見（MIT License）

noslop のルール体系、語句カタログ、辞書を使わない検出の近似の一部、および校正済みの閾値の一部は、coji が MIT License で公開している日本語の文章作法プロジェクトの知見と実装に由来します。noslop はそれらを Rust で書き直し、辞書を使わない形に組み替えています。説明文と例文は noslop のために書き起こしたものです。

Part of noslop's rule system, phrase catalog, dictionary-free detection heuristics and calibrated thresholds is derived from a Japanese writing-practice project published by coji under the MIT License. noslop reimplements them in Rust and restructures them to work without a dictionary. The explanations and examples were written for noslop.

```text
MIT License

Copyright (c) 2026 coji

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## 追加ルールの着想に用いた公開資料

- [Wikipedia: Signs of AI writing](https://en.wikipedia.org/wiki/Wikipedia:Signs_of_AI_writing) — 編集者による観察集の「Vague attributions and overgeneralization of opinions」「Outline-like conclusions about challenges and future prospects」「Rule of three」を、P21・R15・R16 の着想に用いています。日本語の検出条件・説明・例文は独自に作成しました。日本語コーパスでの校正結果としては扱いません。
- [Huang et al., RAP: A Metric for Balancing Repetition and Performance in Open-Source Large Language Models (NAACL 2025)](https://aclanthology.org/2025.naacl-long.69/) — 生成文の反復を品質上の問題として扱う R14 の参考資料です。論文のコードや評価スコアは利用せず、文・段落の文字列比較を独自に実装しています。字数の閾値は未校正です。

## hasami（MIT License）

noslop の文分割は、日本語の形態素解析器 [hasami](https://github.com/owayo/hasami) の辞書を使わない文分割（`hasami::sentence`）を使っています。配布するバイナリには hasami が静的にリンクされています。hasami は noslop と同じ作者によるもので、著作権表示とライセンス（MIT License、Copyright (c) 2026 Yohei）は noslop の [LICENSE](LICENSE) と同じです。

noslop splits sentences with the dictionary-free splitter (`hasami::sentence`) of hasami, a Japanese morphological analyzer, which is statically linked into the distributed binaries. hasami is by the same author as noslop; its copyright notice and license (MIT License, Copyright (c) 2026 Yohei) are the same as noslop's [LICENSE](LICENSE).

### 同梱の形態素解析の辞書（IPAdic）

noslop の既定のバイナリ（feature `bundled-dict`）には、hasami が mecab-ipadic 2.7.0-20070801 から作った形態素解析の辞書（`dict/ipadic.hsd`。hasami v26.9.105 の配布辞書で、出所と SHA-256 は [dict/README.md](dict/README.md)）が入っています。語彙・品詞・連接のデータは mecab-ipadic のもので、hasami が自分の形式（`.hsd`）へ変換し、次の点を変えています。未知語の文字の分類（記号・中黒・ひらがな・英数字の扱いと、空白の文字の書き間違いの訂正）、ダッシュ・波ダッシュ・マイナスの CP932 側の異体字 33 語の追加、単位の記号（`%`・`℃`・`㎏` など）を全角の「％」と同じ品詞の語として足したことです。

mecab-ipadic の条文は Nara Institute of Science and Technology License (2003)（SPDX: NAIST-2003）です。条文の全文は、下の「hasami に組み込まれた例外表の表示」の「1. mecab-ipadic」にあるものと同じです。

The default noslop binary (feature `bundled-dict`) includes a morphological-analysis dictionary built by hasami from mecab-ipadic 2.7.0-20070801 (`dict/ipadic.hsd`, the distributed dictionary of hasami v26.9.105; its origin and SHA-256 are in [dict/README.md](dict/README.md)). The lexicon, parts of speech and connection costs come from mecab-ipadic; hasami converted them to its own format (`.hsd`) and changed the following: the character classes for unknown words (symbols, the middle dot, hiragana and alphanumerics, and a corrected typo in the whitespace class), 33 added CP932-side variants of dashes, tildes and minus signs, and unit symbols (such as `%`, `℃` and `㎏`) added as words with the same part of speech as the full-width 「％」. mecab-ipadic is licensed under the Nara Institute of Science and Technology License (2003) (SPDX: NAIST-2003); the full text is the same as the one in section "1. mecab-ipadic" of the notice for hasami's exception table below.

### hasami に組み込まれた例外表の表示

hasami の文分割は、表層に文末記号を含む語（`Yahoo!ニュース`、`モーニング娘。` など約 1 万 9 千語）の例外表を組み込んでおり、noslop のバイナリにも含まれます。例外表は辞書データ（mecab-ipadic・mecab-ipadic-NEologd・SudachiDict）から作った派生データで、配布するときに添える表示を hasami が NOTICE にまとめています。以下は hasami v26.9.105 の `src/sentence/builtin_exceptions.NOTICE` の写しです（v26.9.101 から変わっていません）。NOTICE が求める Apache License 2.0 の全文は、このファイルの末尾にあります。

The splitter embeds an exception table of about 19,000 words that contain sentence-ending marks (such as `Yahoo!ニュース` and `モーニング娘。`), and the table is included in noslop's binaries. The table is derived from dictionary data (mecab-ipadic, mecab-ipadic-NEologd and SudachiDict), and hasami collects the notices required for redistribution in a NOTICE file. The following is a copy of `src/sentence/builtin_exceptions.NOTICE` from hasami v26.9.105 (unchanged since v26.9.101). The full text of the Apache License 2.0 that the NOTICE requires is at the end of this file.

```text
hasami: 文分割の組み込みの例外表の NOTICE
================================================================================

対象: hasami（https://github.com/owayo/hasami）の src/sentence/builtin_exceptions.txt

この NOTICE は、hasami の文分割の組み込みの例外表を含むものを配布するときに添える表示を
まとめたものである。例外表（と、表から作る照合の索引）は hasami のライブラリに埋め込まれ、文分割
（hasami::sentence）や形態素解析（Analyzer。入力を例外表の語の内側で割らないように前分割する）を
使うバイナリに入る。辞書ファイル（.hsd）を同梱しなくても入る。
このファイルをそのまま、または内容を配布物の NOTICE やサードパーティライセンスの一覧に写して使う。
hasami 自体のライセンス（MIT License。hasami の LICENSE）の表示は、これとは別に要る。


例外表について
--------------

例外表は、表層に文末記号（。！？!?‼⁇⁈⁉．｡）を含む語（「モーニング娘。」「Yahoo!ニュース」など）
の一覧で、文分割がこれらの語の内側で文を切らないために使う。hasami は、配布辞書
dict/ipadic-neologd-sudachi.hsd の全表層形から文末記号を含む語を規則で選び、表記を整えて
1 行 1 語で並べた（hasami export-sentence-exceptions。規則は表の先頭のコメントにある）。
元のデータから語を選び、表記を整えて表層形以外を除いた派生データである。

その辞書は次のデータから作られており、例外表の語はこれらのデータの表層形から作ったものである。

  1. mecab-ipadic 2.7.0-20070801                                  NAIST-2003
  2. mecab-ipadic-NEologd（seed）                                 Apache-2.0
  3. SudachiDict（raw 辞書 20260723 の small_lex・core_lex）      Apache-2.0
     SudachiDict は UniDic（BSD-3-Clause）と NEologd（mecab-unidic-neologd、Apache-2.0）の
     一部を含む

例外表に掛かるライセンスを SPDX の式で書くと NAIST-2003 AND Apache-2.0 AND BSD-3-Clause である
（hasami が語を選んで並べた部分は MIT）。
ソースごとの語数は hasami の THIRD_PARTY_LICENSES.md にある。表は抽出規則を変えて作り直すことが
あるが、元になるデータが変わらなければこの NOTICE はそのまま使える。

Apache License 2.0 の全文は https://www.apache.org/licenses/LICENSE-2.0.txt にある。
2・3 を含むものを配布するときは、この全文の写しを添える（Apache License 2.0 第 4 条 (a)）。


================================================================================
1. mecab-ipadic
================================================================================

https://taku910.github.io/mecab/
（条文は https://github.com/taku910/mecab の mecab-ipadic/COPYING）
ライセンス: Nara Institute of Science and Technology License (2003)（SPDX: NAIST-2003）

NAIST-2003 は、元の形でも改変したものでも、すべての写しに次の著作権表示とそれに続くすべての段落
（ICOT Free Software の条件と NO WARRANTY を含む）を含めることを求める。以下は COPYING の全文である。

Copyright 2000, 2001, 2002, 2003 Nara Institute of Science
and Technology.  All Rights Reserved.

Use, reproduction, and distribution of this software is permitted.
Any copy of this software, whether in its original form or modified,
must include both the above copyright notice and the following
paragraphs.

Nara Institute of Science and Technology (NAIST),
the copyright holders, disclaims all warranties with regard to this
software, including all implied warranties of merchantability and
fitness, in no event shall NAIST be liable for
any special, indirect or consequential damages or any damages
whatsoever resulting from loss of use, data or profits, whether in an
action of contract, negligence or other tortuous action, arising out
of or in connection with the use or performance of this software.

A large portion of the dictionary entries
originate from ICOT Free Software.  The following conditions for ICOT
Free Software applies to the current dictionary as well.

Each User may also freely distribute the Program, whether in its
original form or modified, to any third party or parties, PROVIDED
that the provisions of Section 3 ("NO WARRANTY") will ALWAYS appear
on, or be attached to, the Program, which is distributed substantially
in the same form as set out herein and that such intended
distribution, if actually made, will neither violate or otherwise
contravene any of the laws and regulations of the countries having
jurisdiction over the User or the intended distribution itself.

NO WARRANTY

The program was produced on an experimental basis in the course of the
research and development conducted during the project and is provided
to users as so produced on an experimental basis.  Accordingly, the
program is provided without any warranty whatsoever, whether express,
implied, statutory or otherwise.  The term "warranty" used herein
includes, but is not limited to, any warranty of the quality,
performance, merchantability and fitness for a particular purpose of
the program and the nonexistence of any infringement or violation of
any right of any third party.

Each user of the program will agree and understand, and be deemed to
have agreed and understood, that there is no warranty whatsoever for
the program and, accordingly, the entire risk arising from or
otherwise connected with the program is assumed by the user.

Therefore, neither ICOT, the copyright holder, or any other
organization that participated in or was otherwise related to the
development of the program and their respective officials, directors,
officers and other employees shall be held liable for any and all
damages, including, without limitation, general, special, incidental
and consequential damages, arising out of or otherwise in connection
with the use or inability to use the program or any product, material
or result produced or otherwise obtained by using the program,
regardless of whether they have been advised of, or otherwise had
knowledge of, the possibility of such damages at any time during the
project or thereafter.  Each user will be deemed to have agreed to the
foregoing by his or her commencement of use of the program.  The term
"use" as used herein includes, but is not limited to, the use,
modification, copying and distribution of the program and the
production of secondary products from the program.

In the case where the program, whether in its original form or
modified, was distributed or delivered to or received by a user from
any person, organization or entity other than ICOT, unless it makes or
grants independently of ICOT any specific warranty to the user in
writing, such person, organization or entity, will also be exempted
from and not be held liable to the user for any such damages as noted
above as far as the program is concerned.

================================================================================
2. mecab-ipadic-NEologd
================================================================================

https://github.com/neologd/mecab-ipadic-neologd
ライセンス: Apache License, Version 2.0（SPDX: Apache-2.0）

上流に NOTICE ファイルは無い。以下は上流の COPYING の全文（著作権表示とデータの出典の表示）である。

Copyright (C) 2015-2019 Toshinori Sato (@overlast)

      https://github.com/neologd/mecab-ipadic-neologd

    i. 本データは、株式会社はてなが提供するはてなキーワード一覧ファイル
       中の表記、及び、読み仮名の大半を使用している。

       はてなキーワード一覧ファイルの著作権は、株式会社はてなにある。

       はてなキーワード一覧ファイルの使用条件に基づき、また、
       データ使用の許可を頂いたことに対する感謝の意を込めて、
       以下に株式会社はてなおよびはてなキーワードへの参照をURLで示す。

       株式会社はてな : http://hatenacorp.jp/information/outline

       はてなキーワード :
       http://developer.hatena.ne.jp/ja/documents/keyword/misc/catalog

   ii. 本データは、日本郵便株式会社が提供する郵便番号データ中の表記、
       及び、読み仮名を使用している。

       日本郵便株式会社は、郵便番号データに限っては著作権を主張しないと
       述べている。

       日本郵便株式会社の郵便番号データに対する感謝の意を込めて、
       以下に日本郵便株式会社および郵便番号データへの参照をURLで示す。

       日本郵便株式会社 :
         http://www.post.japanpost.jp/about/profile.html

       郵便番号データ :
         http://www.post.japanpost.jp/zipcode/dl/readme.html

  iii. 本データは、スナフキん氏が提供する日本全国駅名一覧中の表記、及び
       読み仮名を使用している。

       日本全国駅名一覧の著作権は、スナフキん氏にある。

       スナフキん氏は 「このデータを利用されるのは自由ですが、その際に
       不利益を被ったりした場合でも、スナフキんは一切責任は負えません
       ことをご承知おき下さい」と述べている。

       スナフキん氏に対する感謝の意を込めて、
       以下に日本全国駅名一覧のコーナーへの参照をURLで示す。

       日本全国駅名一覧のコーナー :
         http://www5a.biglobe.ne.jp/~harako/data/station.htm

   iv. 本データは、工藤拓氏が提供する人名(姓/名)エントリデータ中の、
       漢字表記の姓・名とそれに対応する読み仮名を使用している。

       人名(姓/名)エントリデータは被災者・安否不明者の人名の
       表記揺れ対策として、Mozcの人名辞書を活用できるという
       工藤氏の考えによって提供されている。

       工藤氏に対する感謝の意を込めて、
       以下にデータ本体と経緯が分かる情報への参照をURLで示す。

       人名(姓/名)エントリデータ :
         http://chasen.org/~taku/software/misc/personal_name.zip

       上記データが提供されることになった経緯
         http://togetter.com/li/111529

    v. 本データは、Web上からクロールした大量の文書データから抽出した
       表記とそれに対応する読み仮名のデータを含んでいる。

       抽出した表記とそれに対応する読み仮名の組は、上記の i. から iv.
       の言語資源の組み合わせによって得られる組のみを採録した。

       Web 上に文書データを公開して下さっている皆様に感謝いたします。

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

      http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.

================================================================================
3. SudachiDict（UniDic と NEologd の一部を含む）
================================================================================

https://github.com/WorksApplications/SudachiDict
ライセンス: Apache License, Version 2.0（SPDX: Apache-2.0）。含まれる UniDic は BSD-3-Clause、
NEologd（mecab-unidic-neologd）は Apache-2.0。

以下は SudachiDict の README にあるライセンスの表示である。

   Copyright (c) 2017-2023 Works Applications Co., Ltd.

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.

This project includes UniDic and a part of NEologd.

SudachiDict には NOTICE という名前のファイルは無く、帰属表示は LEGAL（LEGAL NOTICE INFORMATION）に
ある。以下は LEGAL の全文である。例外表の元になったのは small_lex.csv と core_lex.csv で、
matrix.def.zip と notcore_lex.csv は使っていない。

LEGAL NOTICE INFORMATION
========================

- All the files in this distribution are covered under the Apache License
version 2.0. (see the file LICENSE-2.0.txt)


- src/main/text/small_lex.csv contains a part
  of UniDic (https://unidic.ninjal.ac.jp/).
- src/main/text/matrix.def.zip is a part of UniDic.

Copyright (c) 2011-2013, The UniDic Consortium
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are
met:

 * Redistributions of source code must retain the above copyright
   notice, this list of conditions and the following disclaimer.

 * Redistributions in binary form must reproduce the above copyright
   notice, this list of conditions and the following disclaimer in the
   documentation and/or other materials provided with the
   distribution.

 * Neither the name of the UniDic Consortium nor the names of its
   contributors may be used to endorse or promote products derived
   from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
"AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
OWNER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.


- src/main/text/core_lex.csv and src/main/text/notcore_lex.csv contain
a part of NEologd (https://github.com/neologd/mecab-unidic-neologd).

Copyright (C) 2015-2019 Toshinori Sato (@overlast)

      https://github.com/neologd/mecab-unidic-neologd

    i. 本データは、株式会社はてなが提供するはてなキーワード一覧ファイル
       中の表記、及び、読み仮名の大半を使用している。

       はてなキーワード一覧ファイルの著作権は、株式会社はてなにある。

       はてなキーワード一覧ファイルの使用条件に基づき、また、
       データ使用の許可を頂いたことに対する感謝の意を込めて、
       以下に株式会社はてなおよびはてなキーワードへの参照をURLで示す。

       株式会社はてな : http://hatenacorp.jp/information/outline

       はてなキーワード :
       http://developer.hatena.ne.jp/ja/documents/keyword/misc/catalog

   ii. 本データは、日本郵便株式会社が提供する郵便番号データ中の表記、
       及び、読み仮名を使用している。

       日本郵便株式会社は、郵便番号データに限っては著作権を主張しないと
       述べている。

       日本郵便株式会社の郵便番号データに対する感謝の意を込めて、
       以下に日本郵便株式会社および郵便番号データへの参照をURLで示す。

       日本郵便株式会社 :
         http://www.post.japanpost.jp/about/profile.html

       郵便番号データ :
         http://www.post.japanpost.jp/zipcode/dl/readme.html

  iii. 本データは、スナフキん氏が提供する日本全国駅名一覧中の表記、及び
       読み仮名を使用している。

       日本全国駅名一覧の著作権は、スナフキん氏にある。

       スナフキん氏は 「このデータを利用されるのは自由ですが、その際に
       不利益を被ったりした場合でも、スナフキんは一切責任は負えません
       ことをご承知おき下さい」と述べている。

       スナフキん氏に対する感謝の意を込めて、
       以下に日本全国駅名一覧のコーナーへの参照をURLで示す。

       日本全国駅名一覧のコーナー :
         http://www5a.biglobe.ne.jp/~harako/data/station.htm

   iv. 本データは、工藤拓氏が提供する人名(姓/名)エントリデータ中の、
       漢字表記の姓・名とそれに対応する読み仮名を使用している。

       人名(姓/名)エントリデータは被災者・安否不明者の人名の
       表記揺れ対策として、Mozcの人名辞書を活用できるという
       工藤氏の考えによって提供されている。

       工藤氏に対する感謝の意を込めて、
       以下にデータ本体と経緯が分かる情報への参照をURLで示す。

       人名(姓/名)エントリデータ :
         http://chasen.org/~taku/software/misc/personal_name.zip

       上記データが提供されることになった経緯
         http://togetter.com/li/111529

    v. 本データは、Web上からクロールした大量の文書データから抽出した
       表記とそれに対応する読み仮名のデータを含んでいる。

       抽出した表記とそれに対応する読み仮名の組は、上記の i. から iv.
       の言語資源の組み合わせによって得られる組のみを採録した。

       Web 上に文書データを公開して下さっている皆様に感謝いたします。

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

      http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

## 参考にしたもの（コードは含まない）

- [textlint-rule-preset-ai-writing](https://github.com/textlint-ja/textlint-rule-preset-ai-writing)（MIT）— AI が書いた文章に出やすい癖を textlint のルールとして集めたプリセットです。チャット応答の名残・誇張表現・コロンでの列挙の導入・見出しの装飾など、観点の洗い出しの参考にしました。noslop のルールと語句は独自に選び直したもので、プリセットのコードと語句の一覧は含みません。

  A textlint preset that collects habits common in AI-written text. It informed which aspects to look at (chat-reply leftovers, hype, colon-led lists, decorated headings and so on). noslop's rules and phrases were selected independently; no code or phrase lists from the preset are included.

## ライセンスの全文

### Apache License, Version 2.0（hasami の例外表の NOTICE の 2・3）

```text

                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

      To apply the Apache License to your work, attach the following
      boilerplate notice, with the fields enclosed by brackets "[]"
      replaced with your own identifying information. (Don't include
      the brackets!)  The text should be enclosed in the appropriate
      comment syntax for the file format. We also recommend that a
      file or class name and description of purpose be included on the
      same "printed page" as the copyright notice for easier
      identification within third-party archives.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
```
