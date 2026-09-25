# noslop の開発用タスク。引数なしの `make` でターゲット一覧を表示する。
#
# ツールの版は mise.toml が正。mise があればコマンドを `mise exec --` 経由で呼ぶので、
# シェルで mise を activate していなくても (IDE や GUI から make を呼んでも)
# mise.toml の版で動く。mise を使わず PATH 上のツールで動かすなら SYSTEM_TOOLS=1 を
# 付ける (その場合、版の再現性は保証しない)。
#
# CI (.github/workflows/ci.yml) の Linux と macOS のジョブは make setup と make ci だけを
# 呼ぶ。検査を足すときは ci に足し、workflow に検査のコマンドを並べない。Windows のジョブ
# だけはランナーの make (mingw32-make) を避けて、ci と同じコマンドを直接呼んでいる。
#
# macOS 標準の GNU Make 3.81 で動く書き方に限っている
# (.ONESHELL / .SHELLFLAGS / $(file ...) / != は使わない)。

.PHONY: help setup build release run install uninstall test test-no-default-features lint clippy fmt fmt-check check docs docs-check ci clean

# Default target
.DEFAULT_GOAL := help

# Variables
BINARY_NAME := noslop
INSTALL_PATH ?= /usr/local/bin
# Cargo.lock をコミットしているので、依存の解決結果を CI・リリースとそろえる。
# Cargo.lock を更新したいとき (依存を足した直後など) は CARGO_FLAGS= で外す
CARGO_FLAGS ?= --locked
# make install でスキルを入れる AI エージェント (空にすると入れない: make install SKILL_TARGETS=)
SKILL_TARGETS ?= claude codex
# docs と docs-check が生成したルールの一覧を置く場所。生成に失敗したときに
# docs/rules.md を空にしないよう、いったんここに書く
RULES_MD_TMP := target/rules.md

# ---- Toolchain ----------------------------------------------------------------
# mise は PATH、よくある導入先の順に探す。GUI から起動した make はシェルの PATH を
# 引き継がないことがあるため。make MISE=/path/to/mise で明示もできる。
# mise が無い環境の振る舞いを試すときは MISE_CANDIDATES= で探す先を空にする。
MISE_CANDIDATES ?= $(HOME)/.local/bin/mise /opt/homebrew/bin/mise /usr/local/bin/mise
ifeq ($(SYSTEM_TOOLS),1)
RUN :=
else
ifndef MISE
MISE := $(firstword $(shell command -v mise 2>/dev/null) $(wildcard $(MISE_CANDIDATES)))
endif
ifeq ($(MISE),)
ifneq ($(filter-out help,$(or $(MAKECMDGOALS),help)),)
$(error mise not found. Install it from https://mise.jdx.dev, or add SYSTEM_TOOLS=1 to use the tools on PATH)
endif
endif
RUN := $(if $(MISE),$(MISE) exec --,)
endif

## Setup

setup: ## Install the toolchain (mise.toml) and fetch dependencies
	@if [ -n "$(MISE)" ]; then "$(MISE)" install; fi
	$(RUN) cargo fetch $(CARGO_FLAGS)

## Build Commands

build: ## Build debug version
	$(RUN) cargo build $(CARGO_FLAGS)

release: ## Build release version
	$(RUN) cargo build --release $(CARGO_FLAGS)

run: ## Run the debug build (pass arguments with ARGS="...")
	$(RUN) cargo run $(CARGO_FLAGS) -- $(ARGS)

## Installation

# 上書きコピーではなく一時ファイル + rename で置き換える。
# macOS はコード署名の検証結果をパス/inode 単位でキャッシュするため、実行中または
# 直前に実行されたバイナリへ cp で上書きすると、キャッシュ済みの CDHash と中身が
# 食い違って新しいバイナリが起動直後に SIGKILL される (exit 137)。
# 一時ファイルは rename が inode の差し替えになるよう、同じディレクトリに置く。
# スキル (skills/SKILL.md) は入れたばかりのバイナリで書き出すので、バイナリと版がそろう。
install: release ## Build release, install the binary and the skills (claude + codex)
	@mkdir -p "$(INSTALL_PATH)"
	cp "target/release/$(BINARY_NAME)" "$(INSTALL_PATH)/$(BINARY_NAME).new"
	mv -f "$(INSTALL_PATH)/$(BINARY_NAME).new" "$(INSTALL_PATH)/$(BINARY_NAME)"
	@for target in $(SKILL_TARGETS); do \
		"$(INSTALL_PATH)/$(BINARY_NAME)" skill-install "$$target" || exit 1; \
	done

# バイナリだけを消し、スキルは消さない。スキルの置き場所はエージェントごとに違い、
# 別の版で入れたものまで消してしまうため
uninstall: ## Remove the installed binary (the skills are kept)
	rm -f "$(INSTALL_PATH)/$(BINARY_NAME)"

## Development

test: ## Run tests
	$(RUN) cargo test $(CARGO_FLAGS)

# 辞書を同梱しないビルド (#[cfg(not(feature = "bundled-dict"))] の側) をコンパイルしてテストする。
# cargo は feature を切り替えるたびに target/debug/noslop を置き直すので、これを単独で
# 回した後の target/debug/noslop は辞書を同梱しない版になる (次に既定の feature で
# build・run・test すると、作り直さずに既定の版へ戻る)
test-no-default-features: ## Run tests without the bundled dictionary (--no-default-features)
	$(RUN) cargo test $(CARGO_FLAGS) --no-default-features

lint: ## Run clippy (warnings are errors)
	$(RUN) cargo clippy $(CARGO_FLAGS) --all-targets -- -D warnings

clippy: lint ## Alias of lint

fmt: ## Format code
	$(RUN) cargo fmt --all

fmt-check: ## Check formatting
	$(RUN) cargo fmt --all -- --check

check: fmt-check lint ## Run format check and clippy (no rewrite)

# 手元の noslop.toml の有効・無効が一覧に混ざらないよう、設定ファイルは読まない
docs: ## Regenerate docs/rules.md from the built-in rule catalog
	@mkdir -p target docs
	$(RUN) cargo run --quiet $(CARGO_FLAGS) -- rules --format markdown --no-config > $(RULES_MD_TMP)
	mv -f $(RULES_MD_TMP) docs/rules.md

# ルールの定義や説明文を変えたのに make docs を忘れた変更を落とす (docs/rules.md は書き換えない)
docs-check: ## Check that docs/rules.md is up to date (no rewrite)
	@mkdir -p target
	$(RUN) cargo run --quiet $(CARGO_FLAGS) -- rules --format markdown --no-config > $(RULES_MD_TMP)
	@diff -u docs/rules.md $(RULES_MD_TMP) || { echo "docs/rules.md is out of date. Run: make docs" >&2; exit 1; }

# CI の Linux と macOS のジョブはこれを呼ぶ。書き換えを含めない。
# 同梱しないビルドのテストを先に回し、既定の feature の test と docs-check を後に置く。
# こうすると make ci の後の target/debug/noslop が既定の版 (辞書を同梱) になる
ci: check test-no-default-features test docs-check ## Run the same checks as CI (fmt, clippy, tests, docs)

clean: ## Clean build artifacts
	$(RUN) cargo clean

## Help

help: ## Show this help message
	@echo "$(BINARY_NAME) Build Commands"
	@echo ""
	@echo "Usage: make [target]"
	@echo ""
	@echo "Targets:"
	@grep -E '^[a-zA-Z0-9_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-26s\033[0m %s\n", $$1, $$2}'
	@echo ""
	@echo "Toolchain:"
	@echo "  Versions are pinned in mise.toml. Run make setup first."
	@echo "  Commands run through mise exec; add SYSTEM_TOOLS=1 to use the tools on PATH."
	@echo ""
	@echo "Release:"
	@echo "  Use GitHub Actions > Release > Run workflow"
