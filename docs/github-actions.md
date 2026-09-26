# GitHub Actions で使う

[README に戻る](../README.md)

## GitHub Actions で使う

既定では注釈を付けるだけで、ジョブは成功します。指摘でジョブを落としたい場合だけ `--fail-on warning` などを付けてください。

```yaml
name: noslop

on:
  pull_request:

permissions:
  contents: read

jobs:
  noslop:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v7
      - name: Install noslop
        env:
          NOSLOP_VERSION: v26.9.106   # 使うリリースのタグに置き換える
        run: |
          base="https://github.com/owayo/noslop/releases/download/${NOSLOP_VERSION}"
          asset="noslop-x86_64-unknown-linux-gnu.tar.gz"
          curl -fsSL -o "${asset}" "${base}/${asset}"
          curl -fsSL -o SHA256SUMS "${base}/SHA256SUMS"
          grep " ${asset}\$" SHA256SUMS | sha256sum --check --strict -
          mkdir -p "$HOME/.local/bin"
          tar -xzf "${asset}" -C "$HOME/.local/bin" noslop
          echo "$HOME/.local/bin" >> "$GITHUB_PATH"
      - name: Lint Japanese prose
        run: noslop check docs --format github
```

リリースのタグを固定し、チェックサムを確かめてから使ってください。`releases/latest` から取る形にすると、使う版が知らないうちに変わります。v26.9.105 までのリリースの添付は、生のバイナリ（`noslop-linux-amd64` など）です。
