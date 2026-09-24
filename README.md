<h1 align="center">noslop</h1>

<p align="center">
  <strong>A fast linter that flags AI-generated "slop" patterns in Japanese prose, with no dictionary required</strong>
</p>

<p align="center">
  <a href="https://github.com/owayo/noslop/actions/workflows/ci.yml">
    <img alt="CI" src="https://github.com/owayo/noslop/actions/workflows/ci.yml/badge.svg?branch=main">
  </a>
  <a href="https://github.com/owayo/noslop/releases/latest">
    <img alt="Version" src="https://img.shields.io/github/v/release/owayo/noslop">
  </a>
  <a href="LICENSE">
    <img alt="License" src="https://img.shields.io/github/license/owayo/noslop">
  </a>
</p>

<p align="center">
  English | <a href="README.ja.md">日本語</a>
</p>

---

## Overview

noslop points at the habits that LLM-written Japanese tends to show in minutes, blog posts and technical documents: stock phrases such as `と言えるでしょう`, repeated "not X but Y" contrasts (`〜ではなく`), a flat rhythm where every sentence has about the same length, and textbook-style layouts built from headings and bold text alone.

noslop is not a detector that decides "an AI wrote this". Writers rarely notice their own habits, and reading a long document by eye always misses some. noslop lists every suspicious spot deterministically; whether to fix or keep each one is the writer's call. When you decide to keep something, you can record the reason in a suppression comment.

noslop works without a morphological-analysis dictionary. It judges text by character classes, phrase patterns and sentence-length statistics, so it ships as a single binary, starts instantly and lints large repositories in parallel. When a dictionary for the morphological analyzer [hasami](https://github.com/owayo/hasami) is available, rules that count parts of speech (chains of 「の」 and kanji runs) are judged more precisely.

## Features

- **Dictionary optional**: runs as a single binary without loading any dictionary; with a hasami dictionary (`.hsd`), chains of 「の」 (P16) and kanji runs (P15) are judged by part of speech ([Morphological-analysis dictionary](#morphological-analysis-dictionary-optional))
- **Calibrated thresholds**: only phrases and thresholds whose false-positive rates were measured on human and model-generated documents (7 models) are enabled by default; uncalibrated checks run only when you opt in as experimental rules
- **Two lanes**: AI "slop" (`slop`) and reading-load hints (`readability`) are reported separately; only calibrated slop rules feed the naturalness score
- **Markdown-aware**: skips code blocks, inline code, URLs and front matter at the top of the document (YAML `---` or TOML `+++`), and tells headings, lists, tables and quotes apart. A `---` further down is read as a thematic break or a heading underline, so no text is dropped
- **Bracket-aware sentence splitting**: uses the dictionary-free splitter of the morphological analyzer [hasami](https://github.com/owayo/hasami); it never splits at a full stop inside 「」 or （）, an unclosed bracket does not swallow the following sentences, and words that contain sentence-ending marks such as `Yahoo!ニュース` stay whole
- **Records your decisions**: write why you keep a flagged spot, e.g. `<!-- noslop-disable-next-line P01 -- quoted remark -->`
- **CI-friendly output**: colored text, JSON with a stable schema and GitHub Actions annotations; by default it never fails the job
- **Hands findings back to AI agents**: a revision brief that groups what to fix by rule, as Markdown, JSON or TOON (the same data in fewer tokens), plus an MCP server (`noslop mcp`), a Claude Code hook (`noslop hook claude-code`) and a skill (`noslop skill-install`) let the agent that wrote the text review it
- **Compares revisions**: `noslop diff` shows findings introduced by a rewrite, numbers and names that disappeared, and edits applied uniformly across the whole document
- **Calibrates on your own corpus**: `noslop calibrate` measures each rule's false-positive and detection rates on human and generated documents and suggests thresholds

## Requirements

- **OS**: macOS, Linux, Windows
- **Rust**: 1.88+ (for building from source)

## Installation

### Binary Download

Download the file for your OS from [Releases](https://github.com/owayo/noslop/releases).

| OS | File |
|----|------|
| Linux (x86_64) | `noslop-linux-amd64` |
| macOS (Apple Silicon) | `noslop-darwin-arm64` |
| macOS (Intel) | `noslop-darwin-amd64` |
| Windows (x86_64) | `noslop-windows-amd64.exe` |

Every release includes `SHA256SUMS`; use it to verify the download.

### cargo

```bash
cargo install --git https://github.com/owayo/noslop
```

### From Source

```bash
git clone https://github.com/owayo/noslop
cd noslop
make install   # installs to /usr/local/bin (override with INSTALL_PATH)
```

After installing the binary, `make install` uses it to install the skills for Claude Code and Codex CLI (`~/.claude/skills/noslop/SKILL.md` and `~/.codex/skills/noslop/SKILL.md`; see [Using noslop with AI Agents](#using-noslop-with-ai-agents)). Choose the agents with `SKILL_TARGETS` (`make install SKILL_TARGETS=claude`, or `make install SKILL_TARGETS=` to skip the skills).

## Usage

### Commands

| Command | Description |
|---------|-------------|
| `noslop check [PATH]...` | Lint files or directories (alias: `lint`). Defaults to the current directory |
| `noslop diff <BEFORE> <AFTER>` | Compare two revisions: new findings, lost facts and uniform rewrites |
| `noslop rules` | List the rules |
| `noslop explain <RULE>` | Explain a rule: what it looks at, why it matters, how to fix it, examples and evidence |
| `noslop init` | Create a `noslop.toml` template |
| `noslop mcp` | Run as an MCP server over stdio so AI agents can call the linter |
| `noslop hook claude-code` | Act as a Claude Code PostToolUse hook and hand back findings for the edited file |
| `noslop skill-install <claude\|codex>` | Install the noslop skill for Claude Code or Codex CLI |
| `noslop calibrate --human <PATH> --ai <PATH>` | Measure false-positive and detection rates and thresholds on a corpus |

```bash
# Lint one file
noslop check docs/intro.md

# Lint every Markdown and text file under a directory (respects .gitignore, .ignore and .noslopignore)
noslop check .

# Read from stdin (e.g. pipe an AI draft straight in)
pbpaste | noslop check - --stdin-filename draft.md

# Produce a revision brief to hand to an AI or an editor
noslop check draft.md --format brief | pbcopy

# The same brief as JSON, or as TOON (fewer tokens for an LLM)
noslop check draft.md --report brief --format json
noslop check draft.md --report brief --format toon

# Use the thresholds for technical writing and enable experimental rules
noslop check docs --genre tech --experimental

# Ignore specific rules
noslop check draft.md --ignore-rules P01,R03

# Compare two revisions (pipe the committed version through stdin to compare with HEAD)
noslop diff draft-v1.md draft-v2.md
git show HEAD:docs/intro.md | noslop diff - docs/intro.md --stdin-filename docs/intro.md

# Read a rule's explanation
noslop explain R01
```

When a directory is given, files with the extensions `md` / `markdown` / `txt` are linted (configurable). Files passed explicitly are always linted, whatever their extension. If a directory you pass yields no files at all, a warning goes to stderr (a `.gitignore` line that matches files, such as `dir/**`, also excludes the contents of a directory you pass explicitly; write `dir/` instead).

### `check` Options

| Option | Short | Description |
|--------|-------|-------------|
| `--report <KIND>` | | What to output: `full` (every finding; default) / `brief` (a revision brief that groups what to fix by rule) |
| `--format <FORMAT>` | `-f` | Output format. The full report: `text` (default) / `json` / `toon` / `github` (alias `github-actions`). The brief: `markdown` (default) / `json` / `toon`. `brief` is shorthand for `--report brief --format markdown` |
| `--genre <GENRE>` | | Genre: `general` (default) / `tech` / `business` / `essay`; aliases `blog`→essay, `minutes`→business |
| `--ignore-rules <IDS>` | | Rules to ignore (comma-separated IDs or names) |
| `--enable-rules <IDS>` | | Rules to enable in addition (e.g. a single experimental rule) |
| `--only-rules <IDS>` | | Run only these rules |
| `--experimental` | | Enable every experimental rule and phrase |
| `--config <PATH>` | | Use this configuration file |
| `--no-config` | | Do not read any configuration file |
| `--fail-on <LEVEL>` | | Exit with 1 when a finding at or above this severity exists: `never` (default) / `info` / `warning` / `error` |
| `--stdin-filename <NAME>` | | Display name when reading stdin (`-`); its extension selects the format |
| `--show-suppressed` | | Also show findings kept by suppression comments |
| `--no-readability` | | Do not report the readability lane |
| `--include <KINDS>` | | Also apply phrase rules to lists, tables and quotes: `lists` / `tables` / `quotes` / `all` (comma-separated) |
| `--line-breaks <MODE>` | | How line breaks inside a paragraph are treated: `space` (default) / `sentence` |
| `--dict <PATH>` | | Use this morphological-analysis dictionary (a hasami `.hsd`); it becomes required |
| `--no-dict` | | Do not use a morphological-analysis dictionary; judge with the dictionary-free approximations |
| `--color <WHEN>` | | Coloring: `auto` (default) / `always` / `never`; `NO_COLOR` is honored |
| `--quiet` | `-q` | Hide files without findings and the summary |
| `--brief-limit <N>` | | Maximum number of spots listed per rule in the `brief` format (default 5) |
| `--help` | `-h` | Show help |
| `--version` | `-V` | Show version |

### `diff` Options

`noslop diff <BEFORE> <AFTER>` accepts the same rule-selection and parsing options as `check` (`--genre`, `--ignore-rules`, `--enable-rules`, `--only-rules`, `--experimental`, `--no-readability`, `--include`, `--line-breaks`, `--dict`, `--no-dict`, `--config`, `--no-config`). Either side can be `-` to read from stdin.

| Option | Short | Description |
|--------|-------|-------------|
| `--format <FORMAT>` | `-f` | Output format: `text` (default) / `json` / `toon` |
| `--stdin-filename <NAME>` | | Display name when reading stdin (`-`); its extension selects the format |
| `--color <WHEN>` | | Coloring: `auto` (default) / `always` / `never` |

### `rules` / `explain` / `init`

| Command | Option | Description |
|---------|--------|-------------|
| `noslop rules` | `-f, --format <text\|json\|markdown>` | Output format; `markdown` includes each rule's explanation (used to generate `docs/rules.md`) |
| | `--genre <GENRE>` / `--experimental` | Used to decide whether each rule is enabled by default |
| | `--config <PATH>` / `--no-config` | Whether the enabled column reflects a configuration file |
| `noslop explain <RULE>` | | Show a rule's metadata, the current values of its thresholds and its explanation, by ID or name |
| `noslop init` | `--force` | Create a `noslop.toml` template in the current directory (`--force` overwrites an existing one) |
| `noslop mcp` | `--config <PATH>` / `--no-config` | Configuration file to use. Tools and registration are described in [docs/integrations.md](docs/integrations.md) |
| `noslop hook claude-code` | `--brief-limit <N>` / `--include-readability` / `--experimental` / `--genre <GENRE>` / `--whole-file` | Spots returned per rule (default 3), whether to include readability hints, and whether to look at the whole file instead of the changed lines. See [docs/integrations.md](docs/integrations.md) |
| `noslop skill-install <claude\|codex>` | `--dir <DIR>` | Where to put the skill (default `~/.claude/skills` or `~/.codex/skills`; use `.claude/skills` or similar for a project). Writes `noslop/SKILL.md`, overwriting an existing one |
| `noslop calibrate` | `--human <PATH>` / `--ai <PATH>` (required, repeatable), `--genre`, `--target-fp`, `--holdout`, `--min-detection`, `--no-experimental`, `-f, --format <text\|json\|markdown>` | How the corpus is measured. The procedure is in [docs/calibration.md](docs/calibration.md) |

### Exit Codes

| Code | Meaning |
|------|---------|
| `0` | Finished (with or without findings; `diff` exits 0 even when it lists concerns) |
| `1` | A finding at or above the `--fail-on` severity exists (`check` only) |
| `2` | Argument, configuration or I/O error (if some files cannot be read, results for the readable files are still printed and the exit code is 2) |

With the default `--fail-on never`, the exit code stays 0 even when there are findings. noslop points at suspicions; it is not designed to stop builds by counting them.

## Rules

Rules come in three families.

| Family | ID | What it looks at |
|--------|----|------------------|
| Phrase patterns | `P` | Stock phrases and wording inside a sentence |
| Rhythm & statistics | `R` | Aggregates across sentences and paragraphs: sentence-length variance, repeated endings and openings, paragraph layout |
| Structure | `S` | Markdown layout: bold text, bullet lists, headings |

Each rule has a lane and a status.

- **Lane** — `slop` detects AI habits and feeds the naturalness score. `readability` points at reading load (overlong sentences, double negatives, ...). It is unrelated to AI-likeness, so it never enters the score.
- **Status** — `stable` rules had their false-positive rates measured on a corpus and are enabled by default. `experimental` rules are uncalibrated, or rely on dictionary-free approximations that fall outside the calibration conditions; they run only when enabled with `--experimental` or in the configuration, and they never enter the score. Phrase rules also carry a status per phrase.

| ID | Name | Checks | Lane | Status |
|----|------|--------|------|--------|
| P01 | `AI_CONCLUSION` | Pushy conclusions and wrap-up phrases | slop | stable |
| P02 | `AI_PREFACE` | Stock prefaces and announcements | slop | stable |
| P03 | `AI_CONJUNCTION` | Empty connectives | slop | stable |
| P04 | `REDUNDANT_VERB` | Wordy verb phrases | readability | experimental |
| P05 | `REDUNDANT_MODIFIER` | Vague modifiers | slop | experimental |
| P06 | `OVER_EMPHASIS` | Excessive emphasis | slop | stable |
| P07 | `HEDGING` | Hedges and disclaimers | slop | stable |
| P08 | `EMPTY_ADJECTIVE` | Empty adjectives | slop | stable |
| P09 | `EMPTY_VERB` | Verbs that only announce the work | slop | stable |
| P10 | `DRAMATIC_CLOSER` | Theatrical closing lines | slop | experimental |
| P11 | `COUNT_DECLARATION` | Presentation-style "there are three reasons" | slop | experimental |
| P12 | `TRANSLATIONESE` | Translationese | slop | stable |
| P13 | `INANIMATE_SUBJECT` | Inanimate subject with a transitive verb | slop | stable |
| P14 | `EM_DASH` | Em-dash parentheticals | slop | experimental |
| P15 | `KANJI_RUN` | Long runs of kanji | readability | stable |
| P16 | `NO_CHAIN` | Chains of 「の」 | readability | stable |
| P17 | `DOUBLE_NEGATIVE` | Double negatives | readability | stable |
| P18 | `CHAT_RESIDUE` | Leftovers of a chat reply, such as 「ご質問ありがとうございます」 | slop | experimental |
| P19 | `HYPE` | Hype without evidence | slop | experimental |
| P20 | `COLON_CONTINUATION` | A predicate followed by a colon to introduce a list (「以下の通りです：」) | slop | experimental |
| R01 | `LOW_BURSTINESS` | Monotonous sentence lengths | slop | stable |
| R02 | `REPETITIVE_ENDING` | Repeated sentence endings | readability | experimental |
| R03 | `LONG_SENTENCE` | Overlong sentences | readability | stable |
| R04 | `BURIED_LIST` | Enumerations buried in one sentence | readability | experimental |
| R05 | `ANTITHESIS_REPETITION` | Repeated "not X but Y" contrasts | slop | stable |
| R06 | `NO_NOMINAL_ENDING` | No noun-ending sentences in a long document | slop | stable |
| R07 | `UNIFORM_PARAGRAPHS` | Paragraphs with near-identical sentence counts | slop | stable |
| R08 | `REPEATED_SENTENCE_LEAD` | Repeated sentence openings | slop | experimental |
| R09 | `PARAGRAPH_LEAD_CONJUNCTION` | Conjunctions at paragraph starts | slop | experimental |
| R10 | `CLEFT_BECAUSE` | "It is X. Because Y." constructions | slop | experimental |
| R11 | `SELF_ANSWER` | Asking a question and answering it yourself | slop | experimental |
| R12 | `OVERCORRECTION` | Over-corrected uniformity (mechanical long/short alternation, too many noun endings) | slop | experimental |
| R13 | `COMMA_PROFILE` | A habit of heavy comma use (commas per sentence) | slop | experimental |
| S01 | `BOLD_DENSITY` | Heavy use of bold | slop | experimental |
| S02 | `BULLET_RATIO` | Over-reliance on bullet lists | slop | experimental |
| S03 | `BOILERPLATE_HEADING` | Boilerplate headings such as 「まとめ」「おわりに」 | slop | experimental |
| S04 | `NUMBERED_PHASES` | Numbered stages such as 「フェーズ1」 | slop | experimental |
| S05 | `EMOJI_DENSITY` | Heavy use of emoji and decorative symbols | slop | experimental |
| S06 | `BOLD_LABEL_LIST` | The `**Label**: description` pattern | slop | experimental |
| S07 | `HEADING_TEMPLATE` | Headings poured into one template (`X: Y`, questions or numbers) | slop | experimental |
| S08 | `HEADING_EMPHASIS` | Bold or emoji inside headings | slop | experimental |
| S09 | `STRUCTURE_DENSITY` | Too many headings and bullet items for the amount of text | slop | experimental |
| S10 | `LABEL_STYLE` | Paragraphs or items that open with an emoji or a `Label:` | slop | experimental |

Run `noslop rules` for the current list and `noslop explain <ID>` for details. Every rule's explanation (what it looks at, why it matters, how to fix it, examples and evidence; in Japanese) is collected in [docs/rules.md](docs/rules.md). Rule IDs never change meaning once published.

By default, phrase rules look only at prose paragraphs, because calibration was done on prose. To include lists, tables and quotes, change `[scope]` in the configuration. Rhythm and statistics rules always aggregate prose paragraphs only.

## Configuration

The configuration file is `noslop.toml` (or `.noslop.toml`). noslop searches from the current directory upwards and uses the first one it finds. `--config` selects a file directly, and `--no-config` disables configuration files. Command-line options take precedence over the file.

```toml
genre = "tech"
fail_on = "never"

[files]
exclude = ["CHANGELOG.md", "vendor/**"]

[scope]
lists = true            # also apply phrase rules to bullet items

[morphology]
mode = "required"       # always use a morphological-analysis dictionary (auto / required / off)

[rules.P04]
enabled = true          # enable a single experimental rule

[rules.R03]
severity = "warning"    # change the severity
max_chars = 100         # change the threshold

[[custom]]              # team-specific banned phrases
id = "X01"
name = "BANNED_HONORIFIC"
pattern = "ユーザー様"
message = "「ユーザー様」ではなく「利用者」と書く"
severity = "warning"
```

Every option is described in [examples/noslop.toml](examples/noslop.toml). Changing a threshold breaks the calibration assumptions, so leave a note explaining why.

## Suppression Comments

When you have read a finding and decided not to change the text in that context, record the decision in the document. The comments work in both Markdown and plain text.

| Comment | Scope |
|---------|-------|
| `<!-- noslop-disable-next-line P01 -- reason -->` | The line after the comment |
| `<!-- noslop-disable-line R03 -- reason -->` | The line containing the comment |
| `<!-- noslop-disable P05 -- reason -->` … `<!-- noslop-enable P05 -->` | Between the two comments (to the end of the document without `enable`) |
| `<!-- noslop-disable-file R01 -- reason -->` | The whole document |

- List rules by ID or name, separated by commas or spaces. Omit them to target every rule.
- Write the reason after `--`. It is kept in the JSON output as `suppressed.reason`.
- Unknown rules produce a warning.
- Comments inside code blocks are not treated as suppressions, so you can document the syntax itself.

```markdown
<!-- noslop-disable-next-line P01 -- quoted remark, keep verbatim -->
> 「結論として、この方式が最適と言えるでしょう」と担当者は述べた。
```

A suppression without a reason abandons the decision. Even a short reason such as "part of a proper noun", "quotation" or "natural for this genre" is enough.

## Output Formats

### text (default)

```text
📄 docs/meeting.md
  3:1  情報  P03 AI_CONJUNCTION
    「このように」は前の内容を機械的に束ねる接続です（人間の文章にもよく出るため、弱い手掛かりとして扱っています）
    │ このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。
    │ ^^^^^^^^^^
    💡 接続語に頼らず、前の内容の具体的な事実や疑問から次の話へつないでください
  3:35  警告  P01 AI_CONCLUSION
    「と言えるだろう」は結論を定型句で押し付ける締めです
    │ このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。
    │                                                                     ^^^^^^^^^^^^^^
    💡 定型句を外して言い切るか、結論を支える事実や数値を書いてください

✖ 2 件の指摘 (重大 0・警告 1・情報 1) — 1 ファイルを検査
```

Messages are in Japanese. Lines and columns are 1-based, and columns count characters. Documents with at least 100 characters get a naturalness score next to the file name (e.g. `自然度 56/100 (要修正)`). Findings from experimental entries are marked `[実験的]`.

### json

A stable schema for machines. The essentials (the example below is the P01 finding extracted from the JSON of the same document as the text example above):

- Top level: `schemaVersion`, `tool` (`name`, `version`), `columnUnit`, `settings` (`genre`, `experimental`, `failOn`, `morphology`), `files`, `summary`, `errors`
- `settings.morphology` is the judging method: `requested` (`auto` / `required` / `off`), `method` (`dictionary` / `surface`), `dictionary` (the `name` and `path` of the dictionary used, or `null`) and `reason` (why no dictionary was used: `disabled` / `not-found` / `not-needed`)
- Per file: `path`, `format` (`markdown` / `text`), `characters`, `sentences`, `score` (`value`, `band`, `label`, `formula`; `null` for documents under 100 characters), `diagnostics`, `warnings`
- Per finding: `ruleId`, `ruleName`, `severity`, `lane`, `status`, `message`, `hint`, `range`, `context`, `excerpt`, `related`, `metrics`, `fingerprint`, `suppressed`
- `range` and `context` have `start` and `end`, each with `line`, `column` (1-based, counted in Unicode scalar values as `columnUnit` says) and `offset` (UTF-8 byte offset)
- `metrics` holds rule-specific values. Phrase rules put the matched dictionary entry in `item` (a regular-expression entry is written as `/pattern/`) and the matched text in `matched`
- `fingerprint` is a 16-digit identifier that does not depend on line numbers, so results can be matched against a previous run. A repeated sentence yields the same value, so match as a multiset
- Suppressed findings are kept; `suppressed` records `reason` and `line` (the line of the suppression comment)
- Files that could not be read go to `errors` with `path` and `message`
- Arrays are ordered by path, position and rule ID

```json
{
  "schemaVersion": 1,
  "tool": { "name": "noslop", "version": "0.1.0" },
  "columnUnit": "unicode-scalar",
  "settings": { "genre": "general", "experimental": false, "failOn": "never" },
  "files": [
    {
      "path": "docs/meeting.md",
      "format": "markdown",
      "characters": 56,
      "sentences": 2,
      "score": null,
      "diagnostics": [
        {
          "ruleId": "P01",
          "ruleName": "AI_CONCLUSION",
          "severity": "warning",
          "lane": "slop",
          "status": "stable",
          "message": "「と言えるだろう」は結論を定型句で押し付ける締めです",
          "hint": "定型句を外して言い切るか、結論を支える事実や数値を書いてください",
          "range": {
            "start": { "line": 3, "column": 35, "offset": 133 },
            "end": { "line": 3, "column": 42, "offset": 154 }
          },
          "context": {
            "start": { "line": 3, "column": 1, "offset": 31 },
            "end": { "line": 3, "column": 43, "offset": 157 }
          },
          "excerpt": "このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。",
          "related": [],
          "metrics": { "item": "と言えるだろう", "matched": "と言えるだろう" },
          "fingerprint": "bd78baa3ef438ee5",
          "suppressed": null
        }
      ],
      "warnings": []
    }
  ],
  "summary": {
    "files": 1,
    "filesWithDiagnostics": 1,
    "diagnostics": 2,
    "bySeverity": { "error": 0, "warning": 1, "info": 1 },
    "byLane": { "slop": 2, "readability": 0, "custom": 0 },
    "suppressed": 0,
    "errors": 0
  },
  "errors": []
}
```

### github

Emits GitHub Actions workflow commands, which annotate the pull request diff. Severities map as `error`→`error`, `warning`→`warning`, `info`→`notice`. Suppressed findings are not emitted.

```text
::warning file=docs/meeting.md,line=3,col=35,endLine=3,endColumn=42,title=P01 AI_CONCLUSION::「と言えるだろう」は結論を定型句で押し付ける締めです%0A💡 定型句を外して言い切るか、結論を支える事実や数値を書いてください
```

### brief

Emits a revision brief in Markdown (in Japanese) to hand to an AI agent or an editor. It opens with the rules of revision (keep claims, numbers and proper nouns; do not add facts that are not in the source; do not apply the same edit everywhere; findings may be kept; do not aim at the count or the score; re-run only once), then groups the findings by rule with why they are suspicious, the direction of a fix and the locations. It never prescribes replacement wording.

```markdown
## docs/meeting.md

- 自然度 (参考): 本文が短いため算出していません
- 指摘: AI 臭さ (校正済み) 2 件 / AI 臭さ (実験的) 0 件 / 独自ルール 0 件 / 読みやすさ 0 件

### 優先して見る箇所

#### 1. P01 AI_CONCLUSION — 結論の押し付け・まとめ口調 (警告 1 件)

- なぜ疑わしいか: 要約の定型として大量に学習された言い回しで、生成された文章ほど段落の終わりに現れます。…
- 直し方の方向: 定型句を外して言い切るか、結論を支える事実や数値を書いてください
- 該当箇所:
  - L3: 「と言えるだろう」は結論を定型句で押し付ける締めです
    - 原文: このように、定例会議を減らしたことはチーム全体にとって良い変化だったと言えるだろう。
```

The full structure of the brief is described in [docs/integrations.md](docs/integrations.md).

### Brief as JSON / TOON (`--report brief --format json|toon`)

The same brief comes as JSON for programs and as [TOON](https://github.com/toon-format/spec) (Token-Oriented Object Notation) for handing it to an LLM in fewer tokens. Both carry the same data, with two tables per file:

- `rules` — the rules with findings, in the order to look at them: `ruleId`, `ruleName`, `title`, `lane`, `maxSeverity`, `experimentalOnly`, `count`, `omittedCount` (findings beyond `--brief-limit`), `why` (why it looks suspicious) and `hint` (the direction of a fix)
- `occurrences` — the locations, referring to `rules` by `ruleId`: `line`, `column` (1-based, counted in characters), `message` and `excerpt` (the sentence with the finding; `null` for findings without one)

The top level also has `settings` (`genre`, `experimental`, `method` — the judging method, `dictionary` or `surface` — and `dictionary`, the name of the dictionary used; local paths are left out), `revisionRules`, `editorialQuestions`, `note` (a caveat when there are no findings), `cleanFiles`, `warnings` and `errors`. The naturalness score, metrics, fingerprints and suppressed findings are left out; use the full report to track findings. The schema version is `schemaVersion`, and `kind` is `brief`.

```toon
files[1]:
  - path: docs/meeting.md
    counts:
      stableSlop: 2
      experimentalSlop: 0
      custom: 0
      readability: 0
    rules[2]{ruleId,ruleName,title,lane,maxSeverity,experimentalOnly,count,omittedCount,why,hint}:
      P01,AI_CONCLUSION,結論の押し付け・まとめ口調,slop,warning,false,1,0,要約の定型として…,定型句を外して言い切るか、結論を支える事実や数値を書いてください
      P03,AI_CONJUNCTION,空疎な接続,slop,info,false,1,0,前の段落で何が述べられたかに…,接続語に頼らず、前の内容の具体的な事実や疑問から次の話へつないでください
    occurrences[2]{ruleId,line,column,message,excerpt}:
      P01,3,35,「と言えるだろう」は結論を定型句で押し付ける締めです,このように、定例会議を減らしたことは…と言えるだろう。
      P03,3,1,「このように」は前の内容を機械的に束ねる接続です…,このように、定例会議を減らしたことは…と言えるだろう。
```

Because the rules and locations become one-row-per-item tables, TOON is the shortest encoding of the brief. For the bundled [examples/ai-smelly.md](examples/ai-smelly.md), the brief took 2,770 tokens (`o200k_base`) as TOON, 2,980 as Markdown and 3,137 / 3,802 as compact / pretty JSON.

### toon

`--format toon` emits the full report (the same data as `--format json`) and the result of `noslop diff` as TOON. It is 20–30% shorter than the pretty-printed JSON, but longer than compact JSON, because much of it cannot be laid out as tables (each finding has its own `metrics` keys, for example). To hand findings to an LLM, use the TOON brief (`--report brief --format toon`).

TOON is encoded by the official Rust implementation [toon-format](https://github.com/toon-format/toon-rust) (spec v3.0), without a trailing newline. The output has been checked to decode, in strict mode, to the same data as the JSON with the spec v4.1 reference implementation (`@toon-format/toon` 4.1.1).

## Using noslop in GitHub Actions

By default noslop only adds annotations and the job succeeds. Add `--fail-on warning` (or another level) only if you want findings to fail the job.

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
          NOSLOP_VERSION: v26.9.100   # replace with the release tag you use
        run: |
          base="https://github.com/owayo/noslop/releases/download/${NOSLOP_VERSION}"
          curl -fsSL -o noslop-linux-amd64 "${base}/noslop-linux-amd64"
          curl -fsSL -o SHA256SUMS "${base}/SHA256SUMS"
          grep ' noslop-linux-amd64$' SHA256SUMS | sha256sum --check --strict -
          install -D -m 0755 noslop-linux-amd64 "$HOME/.local/bin/noslop"
          echo "$HOME/.local/bin" >> "$GITHUB_PATH"
      - name: Lint Japanese prose
        run: noslop check docs --format github
```

Pin the release tag and verify the checksum before use. Downloading from `releases/latest` changes the version under you.

## Using noslop with AI Agents

noslop can hand its findings back to the AI agent that wrote the text. Every method passes where and why to look again, together with the constraints of revision, and says explicitly not to aim at reducing the count.

| Method | When to use it |
|--------|----------------|
| `noslop check --format brief` | Paste the result into an AI chat or send it to an editor (as JSON or TOON: `--report brief --format json\|toon`) |
| `noslop skill-install <claude\|codex>` | Teach the agent to review Japanese text with noslop after writing or revising it (a skill) |
| `noslop mcp` | Let agents such as Claude Code or Codex CLI call `check`, `diff` (compare a rewrite with the original), `explain` and `rules` themselves. `check` returns the brief as Markdown (default), JSON or TOON |
| `noslop hook claude-code` | Right after Claude Code writes a file, hand back only the findings that touch the changed lines |

```bash
# Install the skill (~/.claude/skills/noslop/SKILL.md, ~/.codex/skills/noslop/SKILL.md)
noslop skill-install claude
noslop skill-install codex

# Register the MCP server
claude mcp add noslop -- noslop mcp
codex mcp add noslop -- noslop mcp
```

With the skill installed, requests such as "polish this into natural Japanese" or "remove the AI feel" make the agent fetch what to fix with `noslop check --report brief --format toon` and check its rewrite with `noslop diff`. The skill itself is [skills/SKILL.md](skills/SKILL.md) (in Japanese).

Add the hook to `.claude/settings.json` or a similar settings file ([examples/claude-code-settings.json](examples/claude-code-settings.json)).

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Write|Edit|MultiEdit",
        "hooks": [{ "type": "command", "command": "noslop hook claude-code", "timeout": 30 }]
      }
    ]
  }
}
```

Supported MCP protocol versions, what the hook returns and its limits are described in [docs/integrations.md](docs/integrations.md) (in Japanese).

## Comparing Revisions

`noslop diff <BEFORE> <AFTER>` lints both revisions with the same settings and lists three kinds of concerns. It does not grade the rewrite; it is a checklist for the writer to confirm that a rewrite aimed at fewer findings or a better score did not create new problems.

- **Finding changes** — findings introduced by the rewrite, findings that survived a rewritten sentence with the same phrase, persisting and resolved findings, and findings kept with a suppression comment. Findings are matched by the line-independent `fingerprint`
- **Fact changes** — numbers, dates, words that look like proper nouns, quotations and URLs that disappeared or appeared. Numbers that were not in the original carry a warning to check for unsourced figures
- **Uniform rewrites** — large swings in commas per sentence or in the share of noun endings, headings forced into one template, lists turned into prose (or the reverse) wholesale, mechanical long/short alternation and other signs that one transformation was applied across the whole document

With `--format json` or `--format toon`, `hasConcerns` tells whether there is anything to look at. The exit code stays 0 even when there are concerns.

## Naturalness Score

The text and JSON outputs include a per-file naturalness score (0–100; higher means less AI-like).

```text
deduction = (error × 8 + warning × 4 + info × 0.5) × (1000 / max(characters, 1000))
score     = max(100 − deduction, 20)
```

- Only unsuppressed findings from `stable` rules in the `slop` lane count. Readability, experimental and custom rules are excluded.
- Characters are counted over the whole source, including Markdown syntax and line breaks. Each finding weighs more in a short document; in a long document the density matters.
- Documents with fewer than 100 characters of text get no score.

| Score | Band | Reading |
|-------|------|---------|
| 90–100 | 自然 (natural) | Almost no findings; whatever remains can be left to the writer's taste |
| 70–89 | 軽微 (minor) | A few habits remain, but not enough to catch a reader's attention |
| 50–69 | 要修正 (needs work) | Noticeable habits in wording or structure; worth fixing |
| 20–49 | 濃厚 (heavy) | Strong signals in several families; rewriting is faster than patching |

The score is a rough guide. Adding rules changes it, so compare only values with the same `score.formula` (currently `v1`) in JSON. Do not use it as a CI gate.

## Genres

Legitimate conventions differ by genre. Setting `--genre` (or `genre` in the configuration) switches thresholds and turns off rules that conflict with those conventions.

| Genre | Alias | Main differences |
|-------|-------|------------------|
| `general` | | Default. Conservative thresholds that favor no genre |
| `tech` | | Repeated contrasts become `error` at 4.5% of sentences instead of 3%. Missing noun endings are judged from 3,000 characters. Repeated openings need 7 occurrences |
| `business` | `minutes` | Turns off the bold, bullet, boilerplate-heading, numbered-stage, heading-template, structure-density and label-style rules (S01–S04, S07, S09, S10). Missing noun endings are judged from 3,000 characters. Repeated openings need 7 occurrences |
| `essay` | `blog` | Overlong sentences start at 110 characters instead of 90. Missing noun endings are judged from 1,500 characters. Repeated openings need 5 occurrences |

## Morphological-Analysis Dictionary (Optional)

noslop works without a dictionary. When a dictionary (`.hsd`) for the morphological analyzer [hasami](https://github.com/owayo/hasami) is available, rules that count parts of speech are judged precisely, under the same conditions as the original calibration.

| Rule | With a dictionary | Without a dictionary |
|---|---|---|
| P16 chains of 「の」 | Counts the case particle 「の」 and treats particles at most two morphemes apart as one chain | Counts only 「の」 between words made of kanji, katakana or alphanumerics, so it misses chains through words with hiragana (「の家の大きな犬の」) and chains that end in a hiragana word (「魂の安静のため」) |
| P15 kanji runs | Excludes runs that contain proper nouns (era names, personal names and so on) by part of speech | Excludes only runs that end with common institution suffixes (委員会, 株式会社, ...) |

Compared with the original detector, which counts with morphological analysis, on 382 local documents:

| | Without | With (ipadic) |
|---|---:|---:|
| P16 share of the original's findings that were caught | 39% | 90% |
| P16 agreement of the flagged spots | 0.38 | 0.84 |
| P15 agreement of the flagged spots | 0.69 | 0.81 |

Other rules do not change with a dictionary, and sentence splitting never uses one. Noun endings (R06) keep the dictionary-free estimate because the per-document verdicts were identical with and without a dictionary.

### Installing a dictionary

The hasami repository ships prebuilt dictionaries through Git LFS. `ipadic.hsd` (IPAdic only, 17 MB) is recommended. `ipadic-neologd.hsd` and `ipadic-neologd-sudachi.hsd` include NEologd and know more words, but they sometimes analyze common phrases such as 「どうでしょう」 and 「作りました」 as a single proper noun (hasami [#1](https://github.com/owayo/hasami/issues/1)–[#3](https://github.com/owayo/hasami/issues/3)).

```bash
git lfs install
git clone https://github.com/owayo/hasami
mkdir -p ~/.local/share/hasami
cp hasami/dict/ipadic.hsd ~/.local/share/hasami/
```

A dictionary placed in `~/.local/share/hasami/` is used automatically (`HASAMI_DICT` can point to one directly). When several are present, `ipadic-neologd-sudachi`, `ipadic-neologd` and `ipadic` are preferred in that order, so set `dictionary` in the configuration to use ipadic.

### Choosing how the dictionary is used

| Setting | Behavior |
|---|---|
| `mode = "auto"` (default) | Use a dictionary when one is found; otherwise judge with the dictionary-free approximations |
| `mode = "required"` | Always use one; a missing dictionary is a configuration error (exit code 2). Useful to keep CI runs consistent |
| `mode = "off"` / `--no-dict` | Never use one |
| `dictionary = "<path>"` / `--dict <path>` | Use this dictionary. `--dict` also implies `required`. A relative path in the configuration is resolved from the configuration file's directory |

```toml
[morphology]
mode = "auto"
dictionary = "~/.local/share/hasami/ipadic.hsd"
```

The method used appears on the text summary line (for example 「辞書あり (ipadic)」), in `settings.morphology` of the JSON report and in `settings.method` and `settings.dictionary` of the revision brief. When no rule that uses the dictionary runs (for example with `--no-readability`, or when `--only-rules` leaves them out), noslop does not look for a dictionary.

## How the Rules Were Calibrated

The rules and thresholds enabled by default had their false-positive rates checked on human documents and documents generated by 7 models (71–103 human and 81–381 AI documents). The main decisions:

- **Monotonous sentence lengths (R01)** — burstiness `(σ−μ)/(σ+μ)` is a monotone transform of the coefficient of variation, and measuring length in characters or in morae discriminates about equally well. At the document level, `-0.38` flags 1.4% of human documents and about 58% of AI documents; `-0.24` flagged 32% of human documents, so it is not used. Paragraphs contain too few sentences for stable statistics, so the whole document is judged (20 or more prose sentences). Documents with 10–19 sentences are reported as info only below a stricter `-0.45`.
- **Comma counts** — flagging sentences with four or more commas fired on 62% of real documents, and almost all of them were plain enumerations. noslop does not count commas; it points at buried enumerations instead (R04).
- **Long sentences (R03)** — length is not evidence of AI (about 1% of AI documents are caught), but it is a useful reading-load hint, so the readability lane uses 90 characters (roughly the 91st percentile of the corpus).
- **Redundancy dictionaries (P04, P05)** — broad dictionaries of wordy expressions fired on 25.5% of well-written human documents. They stay experimental.
- **Stock phrases** — words humans used more than models (「最後に」「まさに」) were removed, and words with a steady human baseline (「重要なのは」「このように」 and a few others) were lowered to info.
- **Repeated contrasts (R05)** — deciding severity by the raw count fires strongly even at a thin rate in long documents. Severity follows the ratio to the total number of sentences: below 2% info, 2–3% warning, 3% or more error.
- **Noun endings (R06)** — the premise that "many noun endings look like AI" turned out to be backwards; humans use them more. noslop flags a long document that contains none at all, as info.
- **Structural habits (S01–S10)** — not quantitatively calibrated yet, so all are experimental.
- **Rules added later (P18–P20, R11–R13)** — chat-reply leftovers, hype, self-answered questions, over-corrected uniformity and comma habits have provisional thresholds, so they are experimental.

Dictionary-free approximations (estimating noun endings, detecting enumerations, ...) do not reproduce the original calibration conditions. Rules that rely on them stay experimental or info-level until they are recalibrated. With a morphological-analysis dictionary, chains of 「の」 (P16) and kanji runs (P15) are counted by part of speech, as in the original calibration.

To measure the rules on your own documents, run `noslop calibrate --human <human documents> --ai <generated documents>`. It reports each rule's false-positive and detection rates, the rates when a threshold moves, experimental rules that qualify for promotion and calibrated rules that need review (it never rewrites thresholds or statuses). How to collect a corpus, how to generate documents ([tools/corpus](tools/corpus)) and the promotion criteria are described in [docs/calibration.md](docs/calibration.md) (in Japanese).

## Development

Toolchain versions are pinned in `mise.toml` for [mise](https://mise.jdx.dev/).

```bash
mise install              # install the Rust toolchain from mise.toml

mise exec -- make build   # debug build
mise exec -- make test    # run tests
mise exec -- make check   # format check and clippy
mise exec -- make ci      # the same checks as CI (fmt, clippy, test)
mise exec -- make release # release build
```

You can drop `mise exec --` in a shell where `mise activate` is set up.

## Roadmap

- LSP support (live diagnostics in editors)
- Extending the dictionary-based judgment to buried enumerations (R04) and repeated sentence openings (R08)
- Recalibrating the dictionary-free approximations on corpora measured with `noslop calibrate` and promoting experimental rules to stable
- Comparing against a previous JSON result in CI and reporting only new findings (baseline)

## License

[MIT](LICENSE)

Part of noslop's rule system, phrase catalog and thresholds is derived from an MIT-licensed project on Japanese writing practice. The copyright notice and full license text are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Sentences are split with the dictionary-free splitter of [hasami](https://github.com/owayo/hasami) (MIT), a Japanese morphological analyzer. hasami embeds an exception table (a list of words that contain sentence-ending marks) extracted from dictionary data (SudachiDict and others); its sources and copyright notices are also in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

Some of the aspects noslop looks at were also informed by [textlint-rule-preset-ai-writing](https://github.com/textlint-ja/textlint-rule-preset-ai-writing) (MIT). No code or phrase lists from it are included.
