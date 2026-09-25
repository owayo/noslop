#!/usr/bin/env bash
# 配布辞書の目録 (dict/catalog.json) を hasami のリリースに合わせる。
#
# 使い方: tools/dict-catalog.sh [タグ]   (例: tools/dict-catalog.sh v26.9.110。省くと最新のリリース)
#
# hasami のリリースに添付された dictionaries.json を、そのまま dict/catalog.json に写す。
# build.rs がこれを検証して、取得元のタグ・URL・辞書ごとの大きさと SHA-256 を作る。
# 目録の hasami_version がタグと合わない、古い版に戻す、のどちらかなら失敗にする。
# GITHUB_OUTPUT があれば changed=true|false と tag=<タグ> を書く (Release のワークフローが使う)。
#
# macOS の bash 3.2 と BSD の sed・sort でも動く書き方に限っている。jq は使わない。
set -euo pipefail

REPO="owayo/hasami"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CATALOG="$ROOT/dict/catalog.json"

die() {
	echo "エラー: $*" >&2
	exit 1
}

# 目録の hasami_version (整形されていてもいなくても、最初の 1 つ)
catalog_version() {
	sed -n -e '/"hasami_version"/{' \
		-e 's/.*"hasami_version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
		-e 'q' -e '}' "$1"
}

command -v gh >/dev/null 2>&1 || die "gh が見つかりません (https://cli.github.com)"

tag="${1:-}"
if [ -z "$tag" ]; then
	tag="$(gh release view -R "$REPO" --json tagName -q .tagName)" ||
		die "hasami の最新のリリースを調べられません"
fi
case "$tag" in
v[0-9]*) ;;
*) die "タグの形式が違います: ${tag} (v26.9.105 のように書く)" ;;
esac

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

gh release download "$tag" -R "$REPO" -p dictionaries.json -O "$tmp" --clobber ||
	die "hasami ${tag} の dictionaries.json を取得できません"

version="$(catalog_version "$tmp")"
[ -n "${version}" ] || die "hasami ${tag} の dictionaries.json に hasami_version がありません"
[ "v$version" = "$tag" ] ||
	die "hasami ${tag} の dictionaries.json の hasami_version が v${version} で、タグと合いません"

changed=false
if [ -f "$CATALOG" ] && cmp -s "$tmp" "$CATALOG"; then
	echo "配布辞書の目録は hasami ${tag} のままです (変更なし)"
else
	if [ -f "$CATALOG" ]; then
		current="$(catalog_version "$CATALOG")"
		if [ -n "$current" ] && [ "$current" != "$version" ]; then
			newest="$(printf '%s\n%s\n' "$current" "$version" | sort -V | tail -n 1)"
			[ "$newest" = "$version" ] ||
				die "目録を古い版に戻そうとしています (今は v${current}、取得したのは ${tag})"
		fi
	fi
	mkdir -p "$(dirname "$CATALOG")"
	cp "$tmp" "$CATALOG"
	changed=true
	echo "配布辞書の目録を hasami ${tag} に更新しました (dict/catalog.json)"
fi

if [ -n "${GITHUB_OUTPUT:-}" ]; then
	{
		echo "changed=${changed}"
		echo "tag=${tag}"
	} >>"$GITHUB_OUTPUT"
fi
