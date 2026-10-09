#!/usr/bin/env bash
# 依存の hasami を新しいリリースに上げ、同梱の辞書も同じリリースのものにそろえる。
#
# 使い方: tools/hasami-update.sh [タグ]   (make hasami-update が呼ぶ。例: v26.10.101。省くと最新のリリース)
#
# 1. Cargo.toml の [workspace.dependencies] の hasami のタグを書き換え、
#    cargo update -p hasami で Cargo.lock を合わせる。古い版には戻さない
# 2. tools/dict-bundled.sh --update で、固定情報・同梱辞書・表示を同じリリースにそろえる
# 3. THIRD_PARTY_NOTICES.md にある例外表の NOTICE の写しが、新しい版の NOTICE と同じなら写しの版を
#    書き換える。NOTICE が変わっていれば、写しを直すよう知らせて終了コード 1 で終える
#
# 同じタグを指定すると 1 は飛ばし、2 と 3 だけを行う (依存の更新ツールがタグだけを上げたときなど)。
# 例外表の語が変わったか (文分割の結果が変わるか) は、make ci のテスト (src/segment.rs の版の固定) が確かめる。
#
# cargo は CARGO (例: "mise exec -- cargo")、cargo の追加の引数は CARGO_FLAGS で渡す (dict-bundled.sh も読む)。
# macOS の bash 3.2 と BSD の sed でも動く書き方に限っている。jq は使わない。
set -euo pipefail

REPO="owayo/hasami"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
NOTICES="$ROOT/THIRD_PARTY_NOTICES.md"

die() {
	echo "エラー: $*" >&2
	exit 1
}

# Cargo.toml の hasami の依存のタグ (重複を除き、1 行に 1 つ)
hasami_tags() {
	sed -n -e 's/^hasami[[:space:]]*=.*[[:space:]]tag[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' \
		"$ROOT/Cargo.toml" | sort -u
}

# hasami のリリースのファイルの中身 (raw)
hasami_file() {
	gh api -H "Accept: application/vnd.github.raw" "repos/$REPO/contents/$2?ref=$1"
}

# 正規表現の中で版の "." を文字どおりに読ませる
escape_dots() {
	printf '%s' "$1" | sed 's/\./\\./g'
}

command -v gh >/dev/null 2>&1 || die "gh が見つかりません (https://cli.github.com)"

current="$(hasami_tags)"
[ -n "$current" ] || die "Cargo.toml に hasami のタグが見つかりません"
case "$current" in
*"
"*) die "Cargo.toml の hasami のタグがそろっていません: $(printf '%s' "$current" | tr '\n' ' ')" ;;
esac

tag="${1:-}"
if [ -z "$tag" ]; then
	tag="$(gh release view -R "$REPO" --json tagName -q .tagName)" ||
		die "hasami の最新のリリースを調べられません"
fi
case "$tag" in
v[0-9]*) ;;
*) die "タグの形式が違います: ${tag} (v26.10.100 のように書く)" ;;
esac

read -r -a cargo <<<"${CARGO:-cargo}"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

# 1. 依存のタグと Cargo.lock
if [ "$tag" = "$current" ]; then
	echo "依存の hasami は ${tag} のままです"
else
	newest="$(printf '%s\n%s\n' "${current#v}" "${tag#v}" | sort -V | tail -n 1)"
	[ "$newest" = "${tag#v}" ] || die "古い版には戻しません (今は ${current}、指定は ${tag})"
	gh release view "$tag" -R "$REPO" --json tagName >/dev/null ||
		die "hasami のリリース ${tag} が見つかりません"
	sed -e '/^hasami[[:space:]]*=/s/tag[[:space:]]*=[[:space:]]*"'"$(escape_dots "$current")"'"/tag = "'"$tag"'"/' \
		"$ROOT/Cargo.toml" >"$tmp"
	cp "$tmp" "$ROOT/Cargo.toml"
	[ "$(hasami_tags)" = "$tag" ] || die "Cargo.toml の hasami のタグを ${tag} に書き換えられません"
	(cd "$ROOT" && "${cargo[@]}" update -p hasami)
	echo "依存の hasami を ${current} から ${tag} に上げました"

	ours="$(sed -n -e 's/^rust-version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$ROOT/Cargo.toml")"
	theirs="$(hasami_file "$tag" Cargo.toml | sed -n -e 's/^rust-version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p')" ||
		theirs=""
	if [ -n "$theirs" ] && [ "$theirs" != "$ours" ] &&
		[ "$(printf '%s\n%s\n' "$ours" "$theirs" | sort -V | tail -n 1)" = "$theirs" ]; then
		echo "注意: hasami ${tag} の rust-version は ${theirs} で、noslop の ${ours} より新しくなりました。Cargo.toml の rust-version をそろえてください"
	fi
fi

# 2. 同梱の辞書
"$ROOT/tools/dict-bundled.sh" --update

# 3. 例外表の NOTICE の写し (バッククォートは Markdown の文字で、展開しない)
# shellcheck disable=SC2016
noted="$(sed -n -e 's/.*以下は hasami \(v[0-9][0-9.]*\) の `src\/sentence\/builtin_exceptions\.NOTICE` の写しです.*/\1/p' "$NOTICES")"
[ -n "$noted" ] || die "THIRD_PARTY_NOTICES.md に例外表の NOTICE の写しの版が見つかりません"
if [ "$noted" != "$tag" ]; then
	notice_path="src/sentence/builtin_exceptions.NOTICE"
	before="$(hasami_file "$noted" "$notice_path")" || die "hasami ${noted} の NOTICE を取得できません"
	after="$(hasami_file "$tag" "$notice_path")" || die "hasami ${tag} の NOTICE を取得できません"
	if [ "$before" != "$after" ]; then
		echo "hasami ${tag} で例外表の NOTICE が変わりました。THIRD_PARTY_NOTICES.md の写し (今は ${noted} のもの) を新しい NOTICE に直し、版も ${tag} にしてください:" >&2
		echo "  gh api -H 'Accept: application/vnd.github.raw' 'repos/${REPO}/contents/${notice_path}?ref=${tag}'" >&2
		exit 1
	fi
	escaped="$(escape_dots "$noted")"
	sed -e 's/以下は hasami '"$escaped"' の `src/以下は hasami '"$tag"' の `src/' \
		-e 's/NOTICE` from hasami '"$escaped"' (/NOTICE` from hasami '"$tag"' (/' "$NOTICES" >"$tmp"
	cp "$tmp" "$NOTICES"
	if ! { grep -q "以下は hasami ${tag} の" "$NOTICES" && grep -q "NOTICE\` from hasami ${tag} (" "$NOTICES"; }; then
		die "THIRD_PARTY_NOTICES.md の例外表の NOTICE の写しの版を ${tag} に書き換えられません"
	fi
	echo "THIRD_PARTY_NOTICES.md の例外表の NOTICE の写しの版を ${tag} にしました (中身は ${noted} と同じ)"
fi
