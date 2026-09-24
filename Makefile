.PHONY: build release install test fmt fmt-check clippy check ci docs clean help

# Default target
.DEFAULT_GOAL := help

# Variables
BINARY_NAME := noslop
INSTALL_PATH ?= /usr/local/bin
CARGO ?= cargo
# make install でスキルを入れる AI エージェント (空にすると入れない: make install SKILL_TARGETS=)
SKILL_TARGETS ?= claude codex

## Build Commands

build: ## Build debug version
	$(CARGO) build

release: ## Build release version
	$(CARGO) build --release

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

## Development

test: ## Run tests
	$(CARGO) test

fmt: ## Format code
	$(CARGO) fmt --all

fmt-check: ## Check formatting
	$(CARGO) fmt --all -- --check

clippy: ## Run clippy (warnings are errors)
	$(CARGO) clippy --all-targets -- -D warnings

check: fmt-check clippy ## Run format check, clippy and cargo check
	$(CARGO) check

ci: fmt-check clippy test ## Run the same checks as CI (fmt, clippy, test)

# 手元の noslop.toml の有効・無効が一覧に混ざらないよう、設定ファイルは読まない
docs: ## Regenerate docs/rules.md from the built-in rule catalog
	@mkdir -p docs
	$(CARGO) run --quiet -- rules --format markdown --no-config > docs/rules.md

clean: ## Clean build artifacts
	$(CARGO) clean

## Help

help: ## Show this help message
	@echo "$(BINARY_NAME) Build Commands"
	@echo ""
	@echo "Usage: make [target]"
	@echo ""
	@echo "Targets:"
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-20s\033[0m %s\n", $$1, $$2}'
	@echo ""
	@echo "Toolchain:"
	@echo "  mise install && mise exec -- make <target>"
	@echo ""
	@echo "Release:"
	@echo "  Use GitHub Actions > Release > Run workflow"
