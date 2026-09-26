//! Bash のコマンド行から gws の呼び出しを探し、引数を実行せずに読む (tree-sitter-bash)。
//!
//! `&&`・`;`・パイプ・サブシェル・コマンド置換・関数の中まで、プログラム名が `gws` の単純なコマンドを
//! 探す (`env VAR=値`・`command`・`exec`・`nohup`・`time` を前に付けたものも)。引数は、実行しなくても値が
//! 決まるものだけを読む。
//!
//! - クォートしていない語 (`\` のエスケープを解く)、シングルクォート、展開を含まないダブルクォート、
//!   `$'...'`、それらをつないだもの
//! - ダブルクォートの中の、ヒアドキュメントを `cat` で出すだけのコマンド置換 (`"$(cat <<'EOF' … EOF)"`)。
//!   区切りの語をクォートしていないヒアドキュメントは、本文に `$` とバッククォートがないときだけ。
//!   コマンド行の中で `cat` を関数やエイリアスとして定義し直していれば読まない
//!
//! 変数・コマンド置換・算術式・パス名やブレースの展開を含む引数は、値が決まらないものにする。クォートの
//! 中の展開だけなら 1 語になる ([`Arg::Unknown`])。クォートしていない展開は、単語分割とパス名の展開を
//! 受けて消えることも分かれることもある ([`Arg::Dynamic`])。コマンドは実行せず、コマンドが指すファイルも
//! 読まない。

use std::sync::LazyLock;

use tree_sitter::{Language, Node, Parser};

/// Bash の文法。tree-sitter 0.27 はノードの種類の名前を文法から借りるので、`static` に置く。
static LANGUAGE: LazyLock<Language> = LazyLock::new(|| tree_sitter_bash::LANGUAGE.into());

/// コマンド行の引数 1 つ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    /// 実行しなくても値が決まる引数 (クォートを外し、エスケープを解いたもの)。
    Static(String),
    /// 実行するとちょうど 1 つの引数になるが、値は実行しないと決まらない引数 (`"$DOC"` のように、
    /// 展開をクォートの中にだけ含むもの)。
    Unknown,
    /// クォートしていない変数・コマンド置換・パス名の展開などを含み、実行しないと値が決まらない引数。
    /// 空になって消えることも、複数の引数に分かれることもある。
    Dynamic,
}

impl Arg {
    /// 値が決まる引数の値。
    pub fn as_static(&self) -> Option<&str> {
        match self {
            Arg::Static(s) => Some(s),
            Arg::Unknown | Arg::Dynamic => None,
        }
    }
}

/// コマンド行にある gws の呼び出しの引数 (プログラム名より後ろ)。コマンド行に出てくる順に並べる。
///
/// 構文の誤りを含むコマンドは、誤りの箇所を含まない gws の呼び出しだけを読む。
pub fn gws_invocations(command: &str) -> Vec<Vec<Arg>> {
    let mut parser = Parser::new();
    if parser.set_language(&LANGUAGE).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(command, None) else {
        return Vec::new();
    };
    let reader = Reader {
        src: command,
        plain_cat: !redefines_cat(tree.root_node(), command),
    };
    let mut found = Vec::new();
    for node in preorder(tree.root_node()) {
        if node.kind() == "command"
            && !node.has_error()
            && let Some(args) = reader.gws_arguments(node)
        {
            found.push(args);
        }
    }
    found
}

/// 木のノードを行きがけ順に並べる (深い入れ子でもスタックを溢れさせないよう、再帰しない)。
fn preorder(root: Node) -> Vec<Node> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        out.push(node);
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    out
}

/// コマンド行の中で `cat` を関数 (`cat() { … }`) かエイリアス (`alias cat=…`) として定義し直しているか。
fn redefines_cat(root: Node, src: &str) -> bool {
    preorder(root).into_iter().any(|node| match node.kind() {
        "function_definition" => node
            .child_by_field_name("name")
            .is_some_and(|name| &src[name.byte_range()] == "cat"),
        "command" => {
            let is_alias = node
                .child_by_field_name("name")
                .is_some_and(|name| &src[name.byte_range()] == "alias");
            let mut cursor = node.walk();
            is_alias
                && node
                    .children_by_field_name("argument", &mut cursor)
                    .any(|arg| {
                        src[arg.byte_range()]
                            .trim_start_matches(['\'', '"'])
                            .starts_with("cat=")
                    })
        }
        _ => false,
    })
}

/// コマンドの引数 1 つと、`env` が環境変数の指定 (`NAME=値`) として読む形か。
struct Word {
    arg: Arg,
    /// 値が決まらなくても、原文が `NAME=` で始まれば環境変数の指定と分かる。
    assignment: bool,
}

/// 引数の値を読む道具 (コマンド行と、そこで `cat` を定義し直していないか)。
struct Reader<'a> {
    src: &'a str,
    /// `cat` がヒアドキュメントの本文をそのまま出すコマンドか (コマンド行で定義し直していないか)。
    plain_cat: bool,
}

impl<'a> Reader<'a> {
    fn text(&self, node: Node) -> &'a str {
        &self.src[node.byte_range()]
    }

    /// `command` が gws の呼び出しなら、gws に渡る引数。
    fn gws_arguments(&self, command: Node) -> Option<Vec<Arg>> {
        let name = command.child_by_field_name("name")?.named_child(0)?;
        let program = self.value(name, true)?;
        let words = self.words(command);
        let args = unwrap_wrappers(&program, &words)?;
        Some(args.iter().map(|w| w.arg.clone()).collect())
    }

    /// コマンドの引数。
    fn words(&self, command: Node) -> Vec<Word> {
        let mut cursor = command.walk();
        let nodes: Vec<Node> = command
            .children_by_field_name("argument", &mut cursor)
            .collect();
        let mut words = Vec::with_capacity(nodes.len());
        let mut i = 0;
        while i < nodes.len() {
            let node = nodes[i];
            let arg = if node.is_named() {
                match self.value(node, true) {
                    Some(value) => Arg::Static(value),
                    None if self.is_one_word(node) => Arg::Unknown,
                    None => Arg::Dynamic,
                }
            } else {
                // `$"..."` (翻訳する文字列) は `$` と文字列の 2 つに分かれるので、続く文字列ごと 1 つにする
                if node.kind() == "$"
                    && nodes
                        .get(i + 1)
                        .is_some_and(|next| next.start_byte() == node.end_byte())
                {
                    i += 1;
                }
                Arg::Dynamic
            };
            let assignment = starts_with_assignment(self.text(node))
                || arg
                    .as_static()
                    .is_some_and(|s| !s.starts_with('-') && s.contains('='));
            words.push(Word { arg, assignment });
            i += 1;
        }
        words
    }

    /// 引数のノードの値。実行しないと決まらないものは `None`。`leading` は語の先頭か (チルダの展開)。
    fn value(&self, node: Node, leading: bool) -> Option<String> {
        let text = self.text(node);
        match node.kind() {
            "word" => unquoted_word(text, leading),
            "number" => (node.named_child_count() == 0).then(|| text.to_string()),
            "raw_string" => text
                .strip_prefix('\'')?
                .strip_suffix('\'')
                .map(str::to_string),
            "string" => self.double_quoted(node),
            "ansi_c_string" => ansi_c(text),
            "concatenation" => self.concatenation(node),
            // クォートしていないコマンド置換は、結果が IFS で単語に分かれ、パス名の展開も受ける
            _ => None,
        }
    }

    /// 値の決まらない引数のノードが、実行するとちょうど 1 つの引数になるか (展開をクォートの中にだけ
    /// 含み、クォートしていない部分にパス名やブレースの展開の記号がない)。
    fn is_one_word(&self, node: Node) -> bool {
        match node.kind() {
            "string" | "raw_string" | "ansi_c_string" => true,
            "concatenation" => {
                let mut walker = node.walk();
                node.children(&mut walker).all(|child| match child.kind() {
                    "string" | "raw_string" | "ansi_c_string" => true,
                    "word" => unquoted_word(self.text(child), false).is_some(),
                    _ => false,
                })
            }
            _ => false,
        }
    }

    /// ダブルクォートの文字列。中の展開は、ヒアドキュメントを出すコマンド置換だけを読む。
    fn double_quoted(&self, node: Node) -> Option<String> {
        let text = self.text(node);
        if text.len() < 2 || !text.starts_with('"') || !text.ends_with('"') {
            return None;
        }
        let end = node.end_byte() - 1;
        // 文字列の中身は子のノードが覆いきらない (改行や末尾の空白が抜ける) ので、原文の範囲から
        // 展開の部分だけを差し替えて組み立てる
        let mut out = String::new();
        let mut cursor = node.start_byte() + 1;
        let mut walker = node.walk();
        for child in node.named_children(&mut walker) {
            if child.kind() == "string_content" {
                continue;
            }
            let replaced = self.substitution(child)?;
            if child.start_byte() < cursor || child.end_byte() > end {
                return None;
            }
            out.push_str(&unescape_double(&self.src[cursor..child.start_byte()]));
            out.push_str(&replaced);
            cursor = child.end_byte();
        }
        out.push_str(&unescape_double(&self.src[cursor..end]));
        Some(out)
    }

    /// つないだ語 (`--json='{...}'` など)。どの部分も値が決まるときだけ。
    fn concatenation(&self, node: Node) -> Option<String> {
        let mut out = String::new();
        let mut cursor = node.start_byte();
        let mut walker = node.walk();
        for child in node.children(&mut walker) {
            if !child.is_named() || child.start_byte() != cursor {
                return None;
            }
            out.push_str(&self.value(child, cursor == node.start_byte())?);
            cursor = child.end_byte();
        }
        (cursor == node.end_byte()).then_some(out)
    }

    /// ヒアドキュメントを `cat` で出すだけのコマンド置換 (`$(cat <<'EOF' … EOF)`) の値。コマンド置換と
    /// 同じく、末尾の改行を除く。ほかのコマンド置換は `None`。
    fn substitution(&self, node: Node) -> Option<String> {
        if node.kind() != "command_substitution" || !self.plain_cat {
            return None;
        }
        // `$(...)` の形だけ (バッククォートの中は `\` の扱いが違う)
        if node.child(0)?.kind() != "$(" {
            return None;
        }
        let mut walker = node.walk();
        let named: Vec<Node> = node.named_children(&mut walker).collect();
        let [statement] = named.as_slice() else {
            return None;
        };
        let body = self.cat_heredoc(*statement)?;
        Some(body.trim_end_matches('\n').to_string())
    }

    /// `cat <<'EOF' … EOF` の文の、ヒアドキュメントの本文。
    fn cat_heredoc(&self, statement: Node) -> Option<String> {
        if statement.kind() != "redirected_statement" || statement.named_child_count() != 2 {
            return None;
        }
        let body = statement.child_by_field_name("body")?;
        if !self.is_plain_cat(body) {
            return None;
        }
        let mut walker = statement.walk();
        let redirects: Vec<Node> = statement
            .children_by_field_name("redirect", &mut walker)
            .collect();
        let [redirect] = redirects.as_slice() else {
            return None;
        };
        self.heredoc(*redirect)
    }

    /// 引数なし (か `-` だけ) の `cat` か。
    fn is_plain_cat(&self, command: Node) -> bool {
        if command.kind() != "command" {
            return false;
        }
        let mut walker = command.walk();
        command.named_children(&mut walker).all(|child| {
            if child.kind() == "command_name" {
                child
                    .named_child(0)
                    .and_then(|n| self.value(n, true))
                    .is_some_and(|name| program_name(&name) == Some("cat"))
            } else {
                self.value(child, true).as_deref() == Some("-")
            }
        })
    }

    /// ヒアドキュメント (本文だけのもの) の本文。区切りの語をクォートしていなければ、`\` のエスケープを
    /// 解き、`$` とバッククォートを含むものは `None`。
    fn heredoc(&self, redirect: Node) -> Option<String> {
        if redirect.kind() != "heredoc_redirect" {
            return None;
        }
        let (mut start, mut body, mut end, mut dash) = (None, None, None, false);
        let mut walker = redirect.walk();
        for child in redirect.children(&mut walker) {
            match child.kind() {
                "<<" => {}
                "<<-" => dash = true,
                "heredoc_start" => start = Some(child),
                "heredoc_body" => body = Some(child),
                "heredoc_end" => end = Some(child),
                // 続けて書いたパイプ・コマンド・リダイレクトなど
                _ => return None,
            }
        }
        let (start, end) = (start?, end?);
        let (delimiter, quoted) = delimiter(self.text(start))?;
        let src = self.src;
        // 本文は、ヒアドキュメントを始めた行の次の行から、区切りの行の前まで
        let end_line = line_start(src, end.start_byte());
        let body_start = body.map_or(end_line, |b| line_start(src, b.start_byte()));
        if body_start <= start.end_byte() || body_start > end_line {
            return None;
        }
        // <<- は、本文と区切りの行の行頭のタブを除く
        let strip = |line: &'a str| -> &'a str {
            if dash {
                line.trim_start_matches('\t')
            } else {
                line
            }
        };
        // 区切りの行は区切りの語だけか (tree-sitter-bash は、行頭が区切りの語に一致すると後ろに続きが
        // あっても本文を閉じるが、bash は閉じない)
        let end_text = src[end_line..].split('\n').next().unwrap_or("");
        if strip(end_text) != delimiter {
            return None;
        }
        let mut text = String::new();
        for line in src[body_start..end_line].split_inclusive('\n') {
            let line = strip(line);
            // 本文の途中に区切りだけの行があれば、bash はそこで本文を閉じる
            if line.strip_suffix('\n').unwrap_or(line) == delimiter {
                return None;
            }
            text.push_str(line);
        }
        if quoted {
            Some(text)
        } else {
            unescape_heredoc(&text)
        }
    }
}

/// 原文が `NAME=` (シェルの変数名と `=`) で始まるか。
fn starts_with_assignment(raw: &str) -> bool {
    let Some((name, _)) = raw.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 前に付けたコマンド (`env`・`command` など) を外し、gws に渡る引数を返す。gws の呼び出しでなければ
/// `None`。
fn unwrap_wrappers<'w>(program: &str, words: &'w [Word]) -> Option<&'w [Word]> {
    let mut program = program;
    let mut rest = words;
    loop {
        rest = match program_name(program)? {
            "gws" => return Some(rest),
            "env" => skip_env_options(rest)?,
            "command" => skip_options(rest, &["-p"], &[])?,
            "exec" => skip_options(rest, &["-c", "-l", "-cl", "-lc"], &["-a"])?,
            "time" => skip_options(rest, &["-p"], &[])?,
            "nohup" => skip_options(rest, &[], &[])?,
            _ => return None,
        };
        let (next, tail) = rest.split_first()?;
        program = next.arg.as_static()?;
        rest = tail;
    }
}

/// パスを除いたプログラム名 (`/usr/local/bin/gws` → `gws`、Windows の `gws.exe` → `gws`)。
fn program_name(program: &str) -> Option<&str> {
    let name = program.rsplit(['/', '\\']).next()?;
    let lower = name.to_ascii_lowercase();
    Some(match lower.strip_suffix(".exe") {
        Some(stem) if !stem.is_empty() => &name[..stem.len()],
        _ => name,
    })
}

/// 前に付けたコマンドの選択肢を読み飛ばす。`flags` は値を取らない選択肢、`with_value` は次の引数を
/// 値に取る選択肢。知らない選択肢があれば `None` (何を実行するか分からない)。
fn skip_options<'w>(words: &'w [Word], flags: &[&str], with_value: &[&str]) -> Option<&'w [Word]> {
    let mut rest = words;
    while let Some((first, tail)) = rest.split_first() {
        let arg = first.arg.as_static()?;
        if arg == "--" {
            return Some(tail);
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        if flags.contains(&arg) {
            rest = tail;
        } else if with_value.contains(&arg) {
            rest = tail.get(1..)?;
        } else {
            return None;
        }
    }
    Some(rest)
}

/// `env` の選択肢と環境変数の指定 (`NAME=値`。値は決まらなくてもよい) を読み飛ばす。文字列を分割して
/// 実行する `-S` などは `None`。
fn skip_env_options(words: &[Word]) -> Option<&[Word]> {
    let mut rest = words;
    while let Some((first, tail)) = rest.split_first() {
        if first.assignment {
            rest = tail;
            continue;
        }
        let arg = first.arg.as_static()?;
        match arg {
            "--" => return Some(tail),
            "-" | "-i" | "--ignore-environment" | "-0" | "--null" | "-v" | "--debug" => {
                rest = tail;
            }
            "-u" | "--unset" | "-C" | "--chdir" | "-P" => rest = tail.get(1..)?,
            _ if arg.starts_with("--unset=") || arg.starts_with("--chdir=") => rest = tail,
            _ if arg.starts_with("-u") && arg.len() > 2 => rest = tail,
            _ if arg.starts_with('-') => return None,
            _ => break,
        }
    }
    Some(rest)
}

/// クォートしていない語。`\` のエスケープを解く。パス名の展開 (`*` `?` `[`)・ブレースの展開 (`{`)・
/// 語頭のチルダを含む語は `None`。
fn unquoted_word(text: &str, leading: bool) -> Option<String> {
    if leading && text.starts_with('~') {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                // 行の継続
                Some('\n') => {}
                Some(next) => out.push(next),
                None => return None,
            },
            '*' | '?' | '[' | '{' | '$' | '`' => return None,
            _ => out.push(c),
        }
    }
    Some(out)
}

/// ダブルクォートの中の `\` を解く (`\$` `` \` `` `\"` `\\` と行の継続だけ。ほかの `\` は残る)。
fn unescape_double(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('$' | '`' | '"' | '\\') => {
                    out.push(chars.next().unwrap_or('\\'));
                    continue;
                }
                Some('\n') => {
                    chars.next();
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
    }
    out
}

/// `$'...'` の文字列。エスケープを解いた結果が UTF-8 でなければ `None`。
fn ansi_c(text: &str) -> Option<String> {
    let body = text.strip_prefix("$'")?.strip_suffix('\'')?;
    let mut bytes = Vec::with_capacity(body.len());
    let mut chars = body.chars().peekable();
    let mut buf = [0u8; 4];
    while let Some(c) = chars.next() {
        if c != '\\' {
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        let Some(e) = chars.next() else {
            bytes.push(b'\\');
            break;
        };
        match e {
            'a' => bytes.push(0x07),
            'b' => bytes.push(0x08),
            'e' | 'E' => bytes.push(0x1b),
            'f' => bytes.push(0x0c),
            'n' => bytes.push(b'\n'),
            'r' => bytes.push(b'\r'),
            't' => bytes.push(b'\t'),
            'v' => bytes.push(0x0b),
            '\\' | '\'' | '"' | '?' => bytes.push(e as u8),
            '0'..='7' => {
                let mut n = e.to_digit(8)?;
                for _ in 0..2 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(d) => {
                            n = n * 8 + d;
                            chars.next();
                        }
                        None => break,
                    }
                }
                bytes.push(u8::try_from(n & 0xff).ok()?);
            }
            'x' => {
                let n = hex_digits(&mut chars, 2)?;
                bytes.push(u8::try_from(n).ok()?);
            }
            'u' | 'U' => {
                let n = hex_digits(&mut chars, if e == 'u' { 4 } else { 8 })?;
                let c = char::from_u32(n)?;
                bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            'c' => {
                let c = chars.next()?;
                bytes.push(u8::try_from(u32::from(c.to_ascii_uppercase()) ^ 0x40).ok()?);
            }
            _ => {
                bytes.push(b'\\');
                bytes.extend_from_slice(e.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    String::from_utf8(bytes).ok()
}

/// 16 進の数字を最大 `max` 桁読む。1 桁もなければ `None`。
fn hex_digits(chars: &mut std::iter::Peekable<std::str::Chars>, max: usize) -> Option<u32> {
    let mut n: u32 = 0;
    let mut read = 0;
    while read < max {
        let Some(d) = chars.peek().and_then(|c| c.to_digit(16)) else {
            break;
        };
        n = n.checked_mul(16)?.checked_add(d)?;
        chars.next();
        read += 1;
    }
    (read > 0).then_some(n)
}

/// ヒアドキュメントの区切りの語 (クォートを外したもの) と、クォートしていたか。
fn delimiter(text: &str) -> Option<(String, bool)> {
    let mut out = String::new();
    let mut quoted = false;
    let mut quote: Option<char> = None;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\'' | '"') => {
                quote = Some(c);
                quoted = true;
            }
            (Some(q), _) if c == q => quote = None,
            (None | Some('"'), '\\') => {
                quoted = true;
                out.push(chars.next()?);
            }
            _ => out.push(c),
        }
    }
    (quote.is_none() && !out.is_empty()).then_some((out, quoted))
}

/// 区切りの語をクォートしていないヒアドキュメントの本文の `\` を解く (`\$` `` \` `` `\\` と
/// 行の継続だけ)。`$` とバッククォートがあれば (展開される) `None`。
fn unescape_heredoc(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.peek() {
                Some('$' | '`' | '\\') => out.push(chars.next().unwrap_or('\\')),
                Some('\n') => {
                    chars.next();
                }
                _ => out.push('\\'),
            },
            '$' | '`' => return None,
            _ => out.push(c),
        }
    }
    Some(out)
}

/// `pos` を含む行の先頭のバイト位置。
fn line_start(src: &str, pos: usize) -> usize {
    src[..pos].rfind('\n').map_or(0, |i| i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Arg {
        Arg::Static(v.to_string())
    }

    fn only(command: &str) -> Vec<Arg> {
        let mut found = gws_invocations(command);
        assert_eq!(found.len(), 1, "{command}: {found:?}");
        found.remove(0)
    }

    #[test]
    fn quotes_and_escapes_are_resolved() {
        let args = only(
            r#"gws docs +write --document 'd 1' --text "改行を
含む \"本文\" \$HOME と \\ と \n" --x 語\ の\ 続き --y $'一\n二\x41\u3042' --z=12"#,
        );
        assert_eq!(
            args,
            vec![
                s("docs"),
                s("+write"),
                s("--document"),
                s("d 1"),
                s("--text"),
                s("改行を\n含む \"本文\" $HOME と \\ と \\n"),
                s("--x"),
                s("語 の 続き"),
                s("--y"),
                s("一\n二Aあ"),
                s("--z=12"),
            ]
        );
    }

    #[test]
    fn concatenations_join_static_parts() {
        let args = only(r#"gws x --json='{"a":1}' --p="a"'b'c --q=12"#);
        assert_eq!(args[1], s(r#"--json={"a":1}"#));
        assert_eq!(args[2], s("--p=abc"));
        assert_eq!(args[3], s("--q=12"));
    }

    #[test]
    fn expansions_in_quotes_are_one_unknown_word() {
        let args = only(r#"gws x "$HOME" "a${B}c" "$(cat file)" "価格 $1" --j="$X" 'a'"$Y"b end"#);
        assert_eq!(args[0], s("x"));
        assert!(
            args[1..args.len() - 1].iter().all(|a| *a == Arg::Unknown),
            "{args:?}"
        );
        assert_eq!(args.last(), Some(&s("end")), "{args:?}");
    }

    #[test]
    fn unquoted_expansions_are_dynamic() {
        // 単語分割・パス名やブレースの展開を受けて、消えることも分かれることもある
        let args = only(r#"gws x $C `date` $((1+2)) *.md a{b,c} ~/x --j=$X "$Y"* $"tr" end"#);
        assert_eq!(args[0], s("x"));
        assert!(
            args[1..args.len() - 1].iter().all(|a| *a == Arg::Dynamic),
            "{args:?}"
        );
        assert_eq!(args.last(), Some(&s("end")), "{args:?}");
    }

    #[test]
    fn quoted_heredocs_in_command_substitutions_are_static() {
        let command = "gws docs documents batchUpdate --json \"$(cat <<'EOF'\n{\"text\": \"$HOME と `x` と \\n\"}\n\nEOF\n)\" --q \"前$(cat <<\"END\"\n中\nEND\n)後\"";
        let args = only(command);
        assert_eq!(
            args[4],
            s("{\"text\": \"$HOME と `x` と \\n\"}"),
            "末尾の改行は除く: {args:?}"
        );
        assert_eq!(args[6], s("前中後"));

        // <<- は行頭のタブを除く。区切りの語を \ でクォートしてもよい
        let args = only("gws a --b \"$(cat <<-\\EOF\n\t\t一行目\n\t二行目\n\tEOF\n)\"");
        assert_eq!(args[2], s("一行目\n二行目"));
        // 空のヒアドキュメント
        let args = only("gws a --b \"$(cat <<'EOF'\nEOF\n)\"");
        assert_eq!(args[2], s(""));
    }

    #[test]
    fn unquoted_heredocs_are_static_only_without_expansions() {
        let plain = only("gws a --b \"$(cat <<EOF\n本文 \\$5 と \\\\ と \\n\nEOF\n)\"");
        assert_eq!(plain[2], s("本文 $5 と \\ と \\n"));
        let with_var = only("gws a --b \"$(cat <<EOF\n本文 $X\nEOF\n)\"");
        assert_eq!(with_var[2], Arg::Unknown);
        let with_backquote = only("gws a --b \"$(cat <<EOF\n本文 `x`\nEOF\n)\"");
        assert_eq!(with_backquote[2], Arg::Unknown);
    }

    #[test]
    fn only_plain_cat_heredocs_are_read() {
        for command in [
            // パイプでつなぐ・cat に選択肢・ほかのコマンド
            "gws a --b \"$(cat <<'EOF' | tr a b\nx\nEOF\n)\"",
            "gws a --b \"$(cat -n <<'EOF'\nx\nEOF\n)\"",
            "gws a --b \"$(printf '%s' x)\"",
            "gws a --b \"$(sed s/a/b/ <<'EOF'\nx\nEOF\n)\"",
        ] {
            assert_eq!(only(command)[2], Arg::Unknown, "{command}");
        }
        // クォートしないコマンド置換は、結果が IFS で単語に分かれる
        assert_eq!(
            only("gws a --b $(cat <<'EOF'\n一語\nEOF\n)")[2],
            Arg::Dynamic
        );
        assert_eq!(
            only("gws a --b=$(cat <<'EOF'\n一語\nEOF\n)")[1],
            Arg::Dynamic
        );
        // 行頭が区切りの語で始まるだけの行は、bash では本文の続き。tree-sitter-bash はそこで本文を
        // 閉じて読み違えるので、値を読まない (読み違えの補いで誤りを含む木になり、呼び出しごと飛ばす)
        let found = gws_invocations("gws a --b \"$(cat <<'EOF'\nx\nEOF more\nEOF\n)\"");
        assert!(
            found
                .iter()
                .all(|args| args.get(2).is_none_or(|v| v.as_static().is_none())),
            "{found:?}"
        );
        // cat - と、パスを付けた cat は読む
        assert_eq!(only("gws a --b \"$(cat - <<'EOF'\nx\nEOF\n)\"")[2], s("x"));
        assert_eq!(
            only("gws a --b \"$(/bin/cat <<'EOF'\nx\nEOF\n)\"")[2],
            s("x")
        );
    }

    #[test]
    fn a_redefined_cat_is_not_trusted() {
        let heredoc = "gws a --b \"$(cat <<'EOF'\n本文\nEOF\n)\"";
        for prefix in [
            "cat() { printf '別の値'; }\n",
            "function cat { printf '別の値'; }\n",
            "alias cat='tac'\n",
        ] {
            let args = only(&format!("{prefix}{heredoc}"));
            assert_eq!(args[2], Arg::Unknown, "{prefix}");
        }
        assert_eq!(only(heredoc)[2], s("本文"));
    }

    #[test]
    fn gws_is_found_across_lists_pipes_subshells_and_wrappers() {
        let command = "cd x && gws a 1; echo y | gws b 2 || (gws c 3) ; env -i FOO=1 command gws d 4\n\
                       x=$(gws e 5) ; if true; then /usr/local/bin/gws f 6; fi\n\
                       cat <<'EOF' | gws g 7\nbody\nEOF\n\
                       FOO=1 exec -a name gws h 8 && nohup gws i 9 & time -p gws j 10\n\
                       env TOKEN=\"$TOKEN\" OTHER=$X gws k 11";
        let found: Vec<String> = gws_invocations(command)
            .into_iter()
            .map(|args| {
                args.iter()
                    .map(|a| a.as_static().unwrap_or("?").to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        assert_eq!(
            found,
            vec![
                "a 1", "b 2", "c 3", "d 4", "e 5", "f 6", "g 7", "h 8", "i 9", "j 10", "k 11"
            ]
        );
    }

    #[test]
    fn other_commands_and_unknown_wrappers_are_ignored() {
        for command in [
            "echo gws docs +write --text x",
            "gwsx docs",
            "git commit -m 'gws'",
            "sudo gws docs +write --text x",
            "env -S 'gws docs' +write",
            "env $OPTS gws docs +write --text x",
            "command -v gws",
            "$GWS docs +write --text x",
            "bash -c 'gws docs +write --text x'",
        ] {
            assert_eq!(
                gws_invocations(command),
                Vec::<Vec<Arg>>::new(),
                "{command}"
            );
        }
    }

    #[test]
    fn broken_commands_are_skipped() {
        // 閉じていないクォートは引数にならない (値を読まない)
        assert_eq!(
            gws_invocations("gws a --text \"閉じない"),
            vec![vec![s("a"), s("--text")]]
        );
        // 誤りを含まないほうの gws は読む。閉じていないヒアドキュメントの値は読まない
        let found = gws_invocations("gws ok 1\ngws a --text \"$(cat <<'EOF'\n閉じない\n");
        assert_eq!(found.first(), Some(&vec![s("ok"), s("1")]), "{found:?}");
        assert!(
            found[1..]
                .iter()
                .all(|args| args.iter().all(|a| a.as_static() != Some("閉じない"))),
            "{found:?}"
        );
    }

    #[test]
    fn assignments_are_recognized_by_their_name() {
        assert!(starts_with_assignment("TOKEN=\"$X\""));
        assert!(starts_with_assignment("_a1=$(date)"));
        assert!(!starts_with_assignment("1A=x"));
        assert!(!starts_with_assignment("--flag=x"));
        assert!(!starts_with_assignment("\"A\"=x"));
        assert!(!starts_with_assignment("gws"));
    }

    #[test]
    fn program_names_strip_paths_and_exe() {
        assert_eq!(program_name("/opt/bin/gws"), Some("gws"));
        assert_eq!(program_name(r"C:\tools\gws.EXE"), Some("gws"));
        assert_eq!(program_name("gws.exe"), Some("gws"));
        assert_eq!(program_name(".exe"), Some(".exe"));
    }
}
