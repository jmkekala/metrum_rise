// SPDX-License-Identifier: GPL-2.0-only

//! The gate that keeps Rayon entries inside the extension's pools (`CODE-33`).
//!
//! The extension disables Rayon's global pool (`CODE-17`), so a Rayon entry that runs on a thread
//! outside the extension's pools (Godot's main thread, the simulation thread, a test thread)
//! panics with "The global thread pool has not been initialized." where it starts. That panic
//! is the only runtime check and it fires only on executed paths, so this test reads the crate's
//! non-test source and requires every Rayon entry to lie lexically inside a pool-entry closure.
//!
//! - Rayon entries: a `.par_*` method call or `.into_par_iter()` (the traits of
//!   `rayon::prelude`), and the `rayon::` free functions that use the current registry, the
//!   global one outside a pool: `join`, `scope`, `spawn`, `broadcast`, `current_num_threads`,
//!   their variants, and the `rayon::iter` constructors. Each counts once where it is written;
//!   the adaptors and drivers chained on it (`with_min_len`, `for_each`, `collect`) do not.
//!   `current_thread_index`, `yield_now` and types and traits are neutral. Any other `rayon::`
//!   path, and a `use` that imports an entry (so that its calls lose the `rayon::` prefix the
//!   scan reads), fails the gate until it is classified in `classify`.
//! - Pool-entry closures: the argument list of `thread_pools::install`, `install_background` and
//!   `spawn` under any path prefix, and of the two wrappers that run their work in a pool:
//!   `PureJob::spawn` / `PureJob::unstarted` (a pure job's work always runs inside
//!   `install_background`) and `async_jobs.spawn` (`AsyncJobs::spawn` queues it with
//!   `thread_pools::spawn`). `thread_pools.rs` is the pool implementation and is not scanned.
//! - Not scanned: items under a cfg that excludes them from a non-test build (`#[cfg(test)]`,
//!   `#[cfg(all(test, ..))]`), `#[test]` functions, and every module mounted under such a cfg.
//!   The walk follows `mod` declarations from `lib.rs`, so a test-only file is skipped whatever
//!   its name.
//!
//! Bare entries are counted per function (`file::Type::function`, nested functions and
//! `macro_rules!` definitions by their own name). `bare_entries.txt` lists the functions that
//! had them when the gate landed: they rely on a caller already in a pool, which was true of every
//! path the audit found but is checked only by the panic. The list is shrink-only; [`WAIVERS`]
//! counts the three sites that branch on whether they run in a pool by design.
//!
//! Blind spots (lexical, by design): a parallel iterator or a closure built inside a pool-entry
//! closure and driven outside it, or built outside and passed in by name (the second is reported
//! as bare), trait-object and closure dispatch, and `macro_rules!` bodies, which count once at
//! their definition and not where they expand.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

// The functions with bare Rayon entries, as `count key` lines; see the file's header.
const ALLOWLIST: &str = include_str!("bare_entries.txt");
const ALLOWLIST_PATH: &str = "rust/src/thread_pools/bare_entries.txt";

// Sites that run in or out of a pool by design, correct only through their caller's coupling:
// `(key, bare entries, reason)`. A count change means the coupling changed: recheck it.
const WAIVERS: &[(&str, usize, &str)] = &[
    (
        "simulation/network/graph/data.rs::RegionGraph::clone_on_pool",
        8,
        "its one caller, the road tool publication (road_tool_context.rs), takes the copy only \
         when `copy_graph` selects the branch that runs inside `thread_pools::install`",
    ),
    (
        "nodes/sim/render/building_lod.rs::Chunk::update_due",
        2,
        "parallel only with `parallel` set, which building_lod/spatial.rs passes only from inside \
         its `thread_pools::install` closure",
    ),
    (
        "simulation/network/render/road/mod.rs::splice_chunks",
        1,
        "runs its `into_par_iter` only when `rayon::current_thread_index()` says it is on a pool \
         worker, sequentially otherwise",
    ),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Ident,
    Punct,
    Literal,
    Lifetime,
}

#[derive(Clone, Copy, Debug)]
struct Token<'a> {
    kind: Kind,
    text: &'a str,
    line: u32,
}

fn is_ident_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic() || byte >= 0x80
}

fn is_ident_continue(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric() || byte >= 0x80
}

// Splits Rust source into identifiers, punctuation, literals and lifetimes, dropping comments.
// Literal contents (strings, raw strings, byte and C strings, chars) are opaque, so a quoted
// `par_iter` or brace never counts. `::`, `->` and `=>` are single tokens; `>>` stays two `>`.
fn lex(src: &str) -> Vec<Token<'_>> {
    let bytes = src.as_bytes();
    let n = bytes.len();
    let mut tokens = Vec::new();
    let (mut i, mut line) = (0, 1u32);
    let lines_in =
        |from: usize, to: usize| bytes[from..to].iter().filter(|&&b| b == b'\n').count() as u32;
    // End (exclusive) of a quoted string whose opening quote is at `quote`.
    let string_end = |quote: usize| {
        let mut j = quote + 1;
        while j < n && bytes[j] != b'"' {
            j += if bytes[j] == b'\\' { 2 } else { 1 };
        }
        (j + 1).min(n)
    };
    // End (exclusive) of a raw string whose hashes or quote start at `from`.
    let raw_end = |from: usize| {
        let hashes = bytes[from..].iter().take_while(|&&b| b == b'#').count();
        let body = from + hashes + 1;
        let closing = format!("\"{}", "#".repeat(hashes));
        src[body..]
            .find(&closing)
            .map_or(n, |at| body + at + closing.len())
    };
    // A char literal or a lifetime whose quote is at `quote`.
    let quoted = |quote: usize| -> (usize, Kind) {
        if bytes.get(quote + 1) == Some(&b'\\') {
            let mut j = quote + 3;
            while j < n && bytes[j] != b'\'' {
                j += 1;
            }
            return ((j + 1).min(n), Kind::Literal);
        }
        let width = src[quote + 1..].chars().next().map_or(1, char::len_utf8);
        if bytes.get(quote + 1 + width) == Some(&b'\'') {
            return (quote + 2 + width, Kind::Literal);
        }
        let mut j = quote + 1;
        while j < n && is_ident_continue(bytes[j]) {
            j += 1;
        }
        (j, Kind::Lifetime)
    };
    while i < n {
        let byte = bytes[i];
        let next = bytes.get(i + 1).copied();
        let (end, kind) = if byte == b'\n' {
            line += 1;
            i += 1;
            continue;
        } else if byte.is_ascii_whitespace() {
            i += 1;
            continue;
        } else if byte == b'/' && next == Some(b'/') {
            i = src[i..].find('\n').map_or(n, |at| i + at);
            continue;
        } else if byte == b'/' && next == Some(b'*') {
            let (mut j, mut depth) = (i + 2, 1);
            while j < n && depth > 0 {
                if bytes[j..].starts_with(b"/*") {
                    depth += 1;
                    j += 2;
                } else if bytes[j..].starts_with(b"*/") {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            line += lines_in(i, j);
            i = j;
            continue;
        } else if byte == b'"' {
            (string_end(i), Kind::Literal)
        } else if byte == b'\'' {
            quoted(i)
        } else if is_ident_start(byte) {
            let mut j = i;
            while j < n && is_ident_continue(bytes[j]) {
                j += 1;
            }
            let after = bytes.get(j).copied();
            let raw_follows = after == Some(b'"')
                || (after == Some(b'#') && bytes[j..].iter().find(|&&b| b != b'#') == Some(&b'"'));
            match (&src[i..j], after) {
                ("r" | "br" | "cr", _) if raw_follows => (raw_end(j), Kind::Literal),
                ("b" | "c", Some(b'"')) => (string_end(j), Kind::Literal),
                ("b", Some(b'\'')) => (quoted(j).0, Kind::Literal),
                // A raw identifier, `r#type`.
                ("r", Some(b'#')) if bytes.get(j + 1).is_some_and(|&b| is_ident_start(b)) => {
                    let mut k = j + 1;
                    while k < n && is_ident_continue(bytes[k]) {
                        k += 1;
                    }
                    tokens.push(Token {
                        kind: Kind::Ident,
                        text: &src[j + 1..k],
                        line,
                    });
                    i = k;
                    continue;
                }
                _ => (j, Kind::Ident),
            }
        } else if byte.is_ascii_digit() {
            let mut j = i;
            while j < n
                && (is_ident_continue(bytes[j])
                    || (bytes[j] == b'.' && bytes.get(j + 1).is_some_and(u8::is_ascii_digit)))
            {
                j += 1;
            }
            (j, Kind::Literal)
        } else {
            let two = bytes.get(i..i + 2);
            let width = if matches!(two, Some(b"::" | b"->" | b"=>")) {
                2
            } else {
                src[i..].chars().next().map_or(1, char::len_utf8)
            };
            (i + width, Kind::Punct)
        };
        tokens.push(Token {
            kind,
            text: &src[i..end],
            line,
        });
        line += lines_in(i, end);
        i = end;
    }
    tokens
}

// Index of the matching closer for every `(`, `[` and `{`.
fn closers(tokens: &[Token], file: &str) -> Vec<usize> {
    let mut close = vec![usize::MAX; tokens.len()];
    let mut open = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if token.kind != Kind::Punct {
            continue;
        }
        match token.text {
            "(" | "[" | "{" => open.push(index),
            ")" | "]" | "}" => {
                let start = open.pop().unwrap_or_else(|| {
                    panic!("{file}:{}: unbalanced `{}`", token.line, token.text)
                });
                close[start] = index;
            }
            _ => {}
        }
    }
    assert!(open.is_empty(), "{file}: unclosed bracket");
    close
}

// A cfg predicate under a non-test build: `Some(false)` only when it certainly excludes the item.
fn cfg_value(tokens: &[Token], close: &[usize], at: usize) -> (Option<bool>, usize) {
    let name = tokens[at].text;
    if tokens.get(at + 1).is_some_and(|t| t.text == "(") {
        let end = close[at + 1];
        let mut values = Vec::new();
        let mut i = at + 2;
        while i < end {
            let (value, next) = cfg_value(tokens, close, i);
            values.push(value);
            i = next + usize::from(tokens[next].text == ",");
        }
        let value = match name {
            "not" => values[0].map(|v| !v),
            "all" if values.contains(&Some(false)) => Some(false),
            "all" => values.iter().all(Option::is_some).then_some(true),
            "any" if values.contains(&Some(true)) => Some(true),
            "any" => values.iter().all(Option::is_some).then_some(false),
            _ => None,
        };
        return (value, end + 1);
    }
    if tokens.get(at + 1).is_some_and(|t| t.text == "=") {
        return (None, at + 3);
    }
    (if name == "test" { Some(false) } else { None }, at + 1)
}

// Whether the attribute whose `[` is at `open` keeps its item out of a non-test build.
fn excludes(tokens: &[Token], close: &[usize], open: usize) -> bool {
    let body = &tokens[open + 1..close[open]];
    match body {
        [only] => only.text == "test",
        [cfg, paren, ..] if cfg.text == "cfg" && paren.text == "(" => {
            cfg_value(tokens, close, open + 3).0 == Some(false)
        }
        _ => false,
    }
}

// The last token of the item or statement that starts at `at`, after its attributes.
fn item_end(tokens: &[Token], close: &[usize], mut at: usize) -> usize {
    while tokens[at].text == "#" {
        at = close[at + 1] + 1;
    }
    let mut first = at;
    while matches!(
        tokens[first].text,
        "pub" | "unsafe" | "async" | "default" | "extern"
    ) {
        first += 1;
        if tokens[first].text == "(" {
            first = close[first] + 1;
        } else if tokens[first].kind == Kind::Literal {
            first += 1;
        }
    }
    let keyword = tokens[first].text;
    let item = matches!(
        keyword,
        "fn" | "mod" | "impl" | "struct" | "enum" | "union" | "trait" | "macro_rules"
    ) || (keyword == "const" && tokens[first + 1].text == "fn");
    if tokens[at].text == "{" {
        return close[at];
    }
    let mut i = at;
    while i < tokens.len() {
        match tokens[i].text {
            "(" | "[" => i = close[i],
            "{" if item => return close[i],
            // A match arm with a block body, or a braced macro call, ends at the block.
            "{" if matches!(tokens[i - 1].text, "=>" | "!") => return close[i],
            "{" => i = close[i],
            ";" => return i,
            "," if !item => return i,
            ")" | "]" | "}" => return i - 1,
            _ => {}
        }
        i += 1;
    }
    panic!(
        "line {}: cannot find the end of a cfg-excluded item",
        tokens[at].line
    );
}

#[derive(Default)]
struct FileScan {
    // `mod name;` declarations: name and the inline modules around it.
    modules: Vec<(String, Vec<String>)>,
    // Bare entries: key, line, label.
    bare: Vec<(String, u32, String)>,
    problems: Vec<String>,
}

// A named body: `(name, open brace, close brace, is a function)`.
type Scope = (String, usize, usize, bool);

fn scan_file(rel: &str, src: &str) -> FileScan {
    let tokens = lex(src);
    let close = closers(&tokens, rel);
    let text = |i: usize| tokens.get(i).map_or("", |t: &Token| t.text);
    let mut scan = FileScan::default();
    let mut scopes: Vec<Scope> = Vec::new(); // fns, impls, traits, inline mods, macros
    let mut inline_mods: Vec<(String, usize, usize)> = Vec::new();
    let mut regions: Vec<(usize, usize)> = Vec::new();
    let mut entries: Vec<(usize, String)> = Vec::new();
    let body_after = |mut i: usize| -> Option<usize> {
        while i < tokens.len() {
            match text(i) {
                "(" | "[" => i = close[i],
                "{" => return Some(i),
                ";" => return None,
                _ => {}
            }
            i += 1;
        }
        None
    };
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        match (token.kind, token.text) {
            (Kind::Punct, "#") if text(i + 1) == "[" => {
                if excludes(&tokens, &close, i + 1) {
                    i = item_end(&tokens, &close, i) + 1;
                    continue;
                }
                i = close[i + 1];
            }
            (Kind::Punct, "#") if text(i + 1) == "!" && text(i + 2) == "[" => {
                assert!(
                    !excludes(&tokens, &close, i + 2),
                    "{rel}:{}: inner cfg attributes are not supported by the gate",
                    token.line
                );
                i = close[i + 2];
            }
            (Kind::Ident, "mod") if tokens.get(i + 1).is_some_and(|t| t.kind == Kind::Ident) => {
                let name = text(i + 1).to_owned();
                if text(i + 2) == ";" {
                    let around = inline_mods
                        .iter()
                        .filter(|(_, open, end)| *open < i && i < *end)
                        .map(|(name, ..)| name.clone())
                        .collect();
                    scan.modules.push((name, around));
                } else if text(i + 2) == "{" {
                    inline_mods.push((name.clone(), i + 2, close[i + 2]));
                    scopes.push((name, i + 2, close[i + 2], false));
                }
            }
            (Kind::Ident, "fn") if tokens.get(i + 1).is_some_and(|t| t.kind == Kind::Ident) => {
                if let Some(open) = body_after(i + 2) {
                    scopes.push((text(i + 1).to_owned(), open, close[open], true));
                }
            }
            (Kind::Ident, "macro_rules") if text(i + 1) == "!" => {
                if let Some(open) = body_after(i + 3) {
                    scopes.push((format!("{}!", text(i + 2)), open, close[open], true));
                }
            }
            (Kind::Ident, "impl" | "trait")
                if i == 0
                    || matches!(
                        text(i.wrapping_sub(1)),
                        "}" | ";" | "{" | "]" | "unsafe" | "default" | "pub" | ")" | "auto"
                    ) =>
            {
                if let Some(open) = body_after(i + 1) {
                    let name = if token.text == "trait" {
                        text(i + 1).to_owned()
                    } else {
                        impl_type(&tokens[i + 1..open])
                    };
                    scopes.push((name, open, close[open], false));
                }
            }
            (Kind::Ident, "install" | "install_background" | "spawn")
                if text(i.wrapping_sub(1)) == "::"
                    && text(i.wrapping_sub(2)) == "thread_pools"
                    && text(i + 1) == "(" =>
            {
                regions.push((i + 1, close[i + 1]));
            }
            (Kind::Ident, "spawn" | "unstarted")
                if text(i.wrapping_sub(1)) == "::"
                    && text(i.wrapping_sub(2)) == "PureJob"
                    && text(i + 1) == "(" =>
            {
                regions.push((i + 1, close[i + 1]));
            }
            (Kind::Ident, "spawn")
                if text(i.wrapping_sub(1)) == "."
                    && text(i.wrapping_sub(2)) == "async_jobs"
                    && text(i + 1) == "(" =>
            {
                regions.push((i + 1, close[i + 1]));
            }
            (Kind::Ident, name)
                if text(i.wrapping_sub(1)) == "."
                    && (name.starts_with("par_") || name == "into_par_iter")
                    && matches!(text(i + 1), "(" | "::") =>
            {
                entries.push((i, format!(".{name}")));
            }
            (Kind::Ident, "rayon") if text(i + 1) == "::" && text(i.wrapping_sub(1)) != "::" => {
                let (paths, next) = rayon_paths(&tokens, &close, i);
                let in_use = statement_start(&tokens, i).is_some_and(|s| {
                    text(s) == "use"
                        || (text(s) == "pub" && tokens[s..i].iter().any(|t| t.text == "use"))
                });
                for path in paths {
                    match (classify(&path), in_use) {
                        (Class::Neutral, _) => {}
                        (Class::Entry, false) => entries.push((i, format!("rayon::{path}"))),
                        (Class::Entry, true) => scan.problems.push(format!(
                            "{rel}:{}: `use rayon::..{path}` imports a Rayon entry: call it by its \
                             path (`rayon::{path}(..)`) so the gate sees every call",
                            token.line,
                        )),
                        (Class::Unknown, _) => scan.problems.push(format!(
                            "{rel}:{}: `rayon::{path}` is not classified by the gate: add it to \
                             `classify` in rust/src/thread_pools/entry_gate.rs as an entry (it \
                             reaches the current registry, the disabled global one outside a pool) \
                             or as neutral, and name it in the gate's module documentation",
                            token.line,
                        )),
                    }
                }
                i = next;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    for (at, label) in entries {
        if regions.iter().any(|&(open, end)| open < at && at < end) {
            continue;
        }
        scan.bare
            .push((scope_key(rel, &scopes, at), tokens[at].line, label));
    }
    scan
}

// `file::Mod::Type::outer::inner` for the position `at`: the scopes around it, outermost first.
fn scope_key(rel: &str, scopes: &[Scope], at: usize) -> String {
    let mut around: Vec<&Scope> = scopes.iter().filter(|s| s.1 < at && at < s.2).collect();
    around.sort_by_key(|s| s.1);
    if !around.iter().any(|s| s.3) {
        return format!("{rel}::<top level>");
    }
    let names: Vec<&str> = around.iter().map(|s| s.0.as_str()).collect();
    format!("{rel}::{}", names.join("::"))
}

// The type an `impl` header implements, from the tokens between `impl` and its body.
fn impl_type(header: &[Token]) -> String {
    let mut start = 0;
    let mut depth = 0i32;
    if header.first().is_some_and(|t| t.text == "<") {
        for (index, token) in header.iter().enumerate() {
            depth += match token.text {
                "<" => 1,
                ">" => -1,
                _ => 0,
            };
            if depth == 0 {
                start = index + 1;
                break;
            }
        }
    }
    let mut name = "";
    depth = 0;
    for token in &header[start..] {
        match token.text {
            "<" => depth += 1,
            ">" => depth -= 1,
            "where" if depth == 0 => break,
            "for" if depth == 0 => name = "",
            "dyn" | "mut" => {}
            text if depth == 0 && token.kind == Kind::Ident => name = text,
            _ => {}
        }
    }
    name.to_owned()
}

// The first token of the statement or item holding `at`.
fn statement_start(tokens: &[Token], at: usize) -> Option<usize> {
    (0..at)
        .rev()
        .find(|&i| matches!(tokens[i].text, ";" | "{" | "}"))
        .map(|i| i + 1)
}

// The paths a `rayon::` reference at `at` names, and the token after it. A group import,
// `rayon::{A, b}`, names each of its leaves by the leaf alone (`A`, `b`), so a function imported
// from a submodule through a group reads as unknown and is reported.
fn rayon_paths(tokens: &[Token], close: &[usize], at: usize) -> (Vec<String>, usize) {
    let text = |i: usize| tokens.get(i).map_or("", |t| t.text);
    let mut path = Vec::new();
    let mut j = at + 1;
    while text(j) == "::" && tokens.get(j + 1).is_some_and(|t| t.kind == Kind::Ident) {
        path.push(text(j + 1));
        j += 2;
    }
    if text(j) != "::" || text(j + 1) != "{" {
        return (vec![path.join("::")], j);
    }
    let end = close[j + 1];
    let leaves = (j + 2..end)
        .filter(|&k| tokens[k].kind == Kind::Ident && text(k + 1) != "::" && text(k - 1) != "as")
        .map(|k| text(k).to_owned())
        .collect();
    (leaves, end + 1)
}

#[derive(Debug, PartialEq, Eq)]
enum Class {
    Entry,
    Neutral,
    Unknown,
}

// A `rayon::` path, `a::b`, without its `rayon::` prefix.
fn classify(path: &str) -> Class {
    let path: Vec<&str> = path.split("::").collect();
    match path.as_slice() {
        // Free functions that run work on, or read, the current registry: the global one, which
        // the extension disabled, on a thread outside every pool.
        [
            "join"
            | "join_context"
            | "scope"
            | "scope_fifo"
            | "in_place_scope"
            | "in_place_scope_fifo"
            | "spawn"
            | "spawn_fifo"
            | "broadcast"
            | "spawn_broadcast"
            | "current_num_threads",
        ] => Class::Entry,
        // Parallel-iterator constructors (`repeat_n`, `split`, `walk_tree`, ...), driven nearby.
        ["iter", function] if function.starts_with(|c: char| c.is_ascii_lowercase()) => {
            Class::Entry
        }
        // Read only the calling worker, never a registry.
        ["current_thread_index" | "yield_now" | "yield_local" | "max_num_threads"] => {
            Class::Neutral
        }
        // Traits in a glob or group import.
        ["prelude", ..] => Class::Neutral,
        // Types and traits: `ThreadPoolBuilder`, `iter::ParallelIterator`.
        [.., last] if last.starts_with(|c: char| c.is_ascii_uppercase()) => Class::Neutral,
        _ => Class::Unknown,
    }
}

// The non-test `.rs` files of the crate, by walking `mod` declarations from `lib.rs`, with their
// scans.
fn scan_crate(src: &Path) -> BTreeMap<String, FileScan> {
    let mut scans = BTreeMap::new();
    let mut queue = vec![src.join("lib.rs")];
    while let Some(path) = queue.pop() {
        let rel = path
            .strip_prefix(src)
            .expect("modules lie under src")
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        // The pool implementation itself reaches Rayon's registries directly.
        let scan = if rel == "thread_pools.rs" {
            let mut scan = scan_file(&rel, &source);
            scan.bare.clear();
            scan
        } else {
            scan_file(&rel, &source)
        };
        let parent = path.parent().expect("a file has a directory");
        let children = match path.file_name().and_then(|name| name.to_str()) {
            Some("lib.rs" | "mod.rs") => parent.to_path_buf(),
            _ => parent.join(path.file_stem().expect("a module file has a stem")),
        };
        for (name, around) in &scan.modules {
            let dir: PathBuf = around.iter().fold(children.clone(), |dir, m| dir.join(m));
            let file = dir.join(format!("{name}.rs"));
            let nested = dir.join(name).join("mod.rs");
            queue.push(match (file.is_file(), nested.is_file()) {
                (true, false) => file,
                (false, true) => nested,
                _ => panic!(
                    "{rel}: cannot resolve `mod {name};` under {}",
                    dir.display()
                ),
            });
        }
        scans.insert(rel, scan);
    }
    scans
}

fn parse_allowlist() -> BTreeMap<String, usize> {
    let mut listed = BTreeMap::new();
    for (index, line) in ALLOWLIST.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (count, key) = line
            .split_once(' ')
            .unwrap_or_else(|| panic!("{ALLOWLIST_PATH}:{}: expected `count key`", index + 1));
        let count = count
            .parse()
            .unwrap_or_else(|_| panic!("{ALLOWLIST_PATH}:{}: bad count", index + 1));
        assert!(
            listed.insert(key.trim().to_owned(), count).is_none(),
            "{ALLOWLIST_PATH}:{}: {key} listed twice",
            index + 1
        );
    }
    listed
}

// Every bare entry as `key -> lines`, sorted.
fn bare_entries(scans: &BTreeMap<String, FileScan>) -> BTreeMap<&str, Vec<u32>> {
    let mut bare: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    for scan in scans.values() {
        for (key, line, _) in &scan.bare {
            bare.entry(key).or_default().push(*line);
        }
    }
    bare
}

fn lines(lines: &[u32]) -> String {
    let lines: Vec<String> = lines.iter().map(u32::to_string).collect();
    format!(
        "line{} {}",
        if lines.len() == 1 { "" } else { "s" },
        lines.join(", ")
    )
}

#[test]
fn rayon_entries_stay_inside_the_extension_pools() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let scans = scan_crate(&src);
    let bare = bare_entries(&scans);
    let listed = parse_allowlist();
    let waived: BTreeMap<&str, (usize, &str)> = WAIVERS
        .iter()
        .map(|&(key, count, reason)| (key, (count, reason)))
        .collect();
    assert_eq!(waived.len(), WAIVERS.len(), "a waiver is listed twice");

    let mut problems: Vec<String> = scans
        .values()
        .flat_map(|scan| scan.problems.clone())
        .collect();
    for key in waived.keys() {
        assert!(
            !listed.contains_key(*key),
            "{key} is both waived and listed in {ALLOWLIST_PATH}"
        );
    }
    for (&key, at) in &bare {
        let found = at.len();
        let file = key.split("::").next().unwrap_or(key);
        let at = lines(at);
        if let Some(&(count, reason)) = waived.get(key) {
            if found != count {
                problems.push(format!(
                    "{key}: {found} bare Rayon entries ({file} {at}), its waiver counts {count} \
                     ({reason}). Recheck that coupling: wrap any new region in \
                     `crate::thread_pools::install(|| ..)`, then set the waiver's count in \
                     rust/src/thread_pools/entry_gate.rs to what remains."
                ));
            }
            continue;
        }
        match listed.get(key) {
            None => problems.push(format!(
                "{key}: {found} Rayon entr{} outside the extension's pools ({file} {at}). Wrap \
                 the parallel region in `crate::thread_pools::install(|| ..)` \
                 (`install_background` for background-pool work); on a pool worker it runs \
                 inline. Do not add the function to {ALLOWLIST_PATH}: the list is shrink-only.",
                if found == 1 { "y" } else { "ies" }
            )),
            Some(&count) if found > count => problems.push(format!(
                "{key}: {found} bare Rayon entries ({file} {at}), {count} allowed. Wrap the new \
                 region in `crate::thread_pools::install(|| ..)`; the listed count covers only the \
                 entries that predate the gate."
            )),
            Some(&count) if found < count => problems.push(format!(
                "{key}: {found} bare Rayon entries, {count} listed. Set its line in \
                 {ALLOWLIST_PATH} to `{found} {key}`: the list is shrink-only."
            )),
            Some(_) => {}
        }
    }
    for (key, count) in &listed {
        if !bare.contains_key(key.as_str()) {
            problems.push(format!(
                "{key}: no bare Rayon entries left, {count} listed. Remove its line from \
                 {ALLOWLIST_PATH}: the list is shrink-only."
            ));
        }
    }
    for (key, (count, reason)) in &waived {
        if !bare.contains_key(key) {
            problems.push(format!(
                "{key}: no bare Rayon entries left, its waiver counts {count} ({reason}). Remove \
                 the waiver from rust/src/thread_pools/entry_gate.rs."
            ));
        }
    }
    if problems.is_empty() {
        return;
    }
    problems.sort();
    let mut message = format!(
        "{} Rayon pool-entry problem(s). A Rayon entry outside the extension's pools panics with \
         \"The global thread pool has not been initialized.\" on the thread that reaches it \
         (CODE-17, CODE-33); see docs/debug.md#shutdown.\n",
        problems.len()
    );
    for problem in &problems {
        let _ = writeln!(message, "- {problem}");
    }
    message.push_str(
        "A listed function stays correct only while every caller is already inside a pool; that \
         is why today's are listed until each enters the pool itself. A function only ever called \
         from inside a pool may stay listed, with that reason in a comment above its line in the \
         allowlist.",
    );
    panic!("{message}");
}

#[test]
fn the_lexer_keeps_literals_and_comments_opaque() {
    let src = r####"
        fn a() { let s = "x.par_iter() {"; let r = r#"y.par_iter() }"#; }
        // z.par_iter() {
        /* w.par_iter() /* nested } */ */
        fn b<'a>(c: &'a [u8]) -> char { b'{'; '}'; '\''; c.par_iter(); 'x' }
        #[cfg(test)]
        fn c() { v.par_iter(); }
        #[cfg(all(test, debug_assertions))]
        mod d { fn e() { v.par_iter(); } }
        #[cfg(not(test))]
        fn f() { crate::thread_pools::install(|| v.par_iter()); rayon::join(|| (), || ()); }
        impl<T> Drop for Wrapper<T> { fn drop(&mut self) { rayon::current_num_threads(); } }
    "####;
    let scan = scan_file("x.rs", src);
    let found: Vec<(&str, u32, &str)> = scan
        .bare
        .iter()
        .map(|(key, line, label)| (key.as_str(), *line, label.as_str()))
        .collect();
    assert_eq!(
        found,
        [
            ("x.rs::b", 5, ".par_iter"),
            ("x.rs::f", 11, "rayon::join"),
            ("x.rs::Wrapper::drop", 12, "rayon::current_num_threads"),
        ]
    );
    assert!(scan.problems.is_empty(), "{:?}", scan.problems);
}
