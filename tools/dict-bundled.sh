#!/usr/bin/env bash
# 同梱辞書の準備と、固定情報の更新の入口。
#
# 使い方: tools/dict-bundled.sh [--update]
#
# 引数なしでは固定情報 (dict/bundled.json) に合う本体を準備する。追跡ファイルは書き換えない。
# --update のときだけ workspace の hasami のリリースから本体と目録を gh で取得し、
# 固定情報・dict/README.md・THIRD_PARTY_NOTICES.md を更新する。
#
# cargo は CARGO (例: "mise exec -- cargo")、cargo の追加の引数は CARGO_FLAGS (例: --locked) で渡す。
# 独立した workspace メンバーを起動するので、本体のビルドや辞書の有無に依存しない。
#
# macOS の bash 3.2 と BSD の sed でも動く書き方に限っている。jq は使わない。
set -euo pipefail

read -r -a cargo <<<"${CARGO:-cargo}"
read -r -a flags <<<"${CARGO_FLAGS:-}"
case "${1:-}" in
"")
    "${cargo[@]}" run --quiet ${flags[@]+"${flags[@]}"} -p noslop-xtask -- prepare
    exit 0
    ;;
--update) ;;
*) echo "使い方: tools/dict-bundled.sh [--update]" >&2; exit 1 ;;
esac

REPO="owayo/hasami"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

die() {
	echo "エラー: $*" >&2
	exit 1
}

# Cargo.toml の hasami の依存のタグ (重複を除き、1 行に 1 つ)
hasami_tags() {
	sed -n -e 's/^hasami[[:space:]]*=.*[[:space:]]tag[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' \
		"$ROOT/Cargo.toml" | sort -u
}

command -v gh >/dev/null 2>&1 || die "gh が見つかりません (https://cli.github.com)"

tag="$(hasami_tags)"
[ -n "$tag" ] || die "Cargo.toml に hasami のタグが見つかりません"
case "$tag" in
*"
"*) die "Cargo.toml の hasami のタグがそろっていません: $(printf '%s' "$tag" | tr '\n' ' ')" ;;
esac

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

gh release download "$tag" -R "$REPO" -p ipadic.hsd -p dictionaries.json -D "$tmp" ||
	die "hasami ${tag} の ipadic.hsd と dictionaries.json を取得できません"

"${cargo[@]}" run --quiet ${flags[@]+"${flags[@]}"} -p noslop-xtask -- update "$tmp" "$tag"
