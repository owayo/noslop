# 同梱の形態素解析の辞書

`ipadic.hsd` は、形態素解析器 [hasami](https://github.com/owayo/hasami) の辞書です。noslop は既定の feature `bundled-dict` でこれをバイナリに埋め込み、品詞で数えるルール (P15・P16) に使います。

| 項目 | 値 |
|---|---|
| 出所 | hasami v26.9.102 の `dict/ipadic.hsd` (Git LFS) |
| 元のデータ | mecab-ipadic 2.7.0-20070801 (辞書のメタデータ `sources=ipadic@61b90ba6e669`) |
| 大きさ | 18,117,590 バイト |
| SHA-256 | `fcc46edf116800e06ca41508ef332a8af128fe6185a4875a861d0b061bc53593` |
| ライセンス | NAIST-2003 (条文は [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md)) |

NEologd の語彙を含む辞書 (ipadic-neologd など) は大きく、ありふれた表現を固有名詞にする誤りもあるため同梱しません。使いたい場合は `--dict` か設定の `dictionary` で指定します。

## 更新の手順

1. hasami の新しいタグで `dict/ipadic.hsd` を取り出す (`git lfs install` のうえで clone する)
2. このディレクトリの `ipadic.hsd` を置き換え、上の表を書き換える
3. `src/morph.rs` のテストに固定した辞書の大きさ・語数・出典を書き換える
4. 手元の文書で P15・P16 の指摘の差分を確かめてから `mise exec -- make ci` を通す
