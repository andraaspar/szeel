//! Reformats scene source: one node per line, four-space indents, single
//! spaces between entries. Comments are kept where they are, and so is one
//! blank line wherever the source had any between two lines.
//!
//! This works on tokens rather than on a parsed [`KdlDocument`], because the
//! `kdl` crate drops comments that follow a `}` when it prints a document.
//! The result is still parsed and checked against the source, so a bug here
//! can never change what a scene means.

use std::fmt;

use kdl::{KdlDocument, KdlError};

const INDENT: &str = "    ";

#[derive(Debug)]
pub enum FormatError {
    /// The source has syntax errors.
    Syntax(KdlError),
    /// Formatting would have changed the scene; a bug in the formatter.
    Changed,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::Syntax(_) => f.write_str("the scene has syntax errors"),
            FormatError::Changed => f.write_str("formatting would change the scene"),
        }
    }
}

impl std::error::Error for FormatError {}

/// Reformats `src`.
pub fn format(src: &str) -> Result<String, FormatError> {
    let before = normalized(src).map_err(FormatError::Syntax)?;
    let out = print(&lex(src));
    match normalized(&out) {
        Ok(after) if after == before => Ok(out),
        _ => Err(FormatError::Changed),
    }
}

/// `src` printed with all formatting and comments removed, for comparing
/// what two sources mean.
fn normalized(src: &str) -> Result<String, KdlError> {
    let mut doc: KdlDocument = src.parse()?;
    doc.autoformat_no_comments();
    Ok(doc.to_string())
}

#[derive(Debug, PartialEq)]
enum Token<'a> {
    /// A value, name, property or type annotation, strings included.
    Word(&'a str),
    /// `//` to the end of the line.
    LineComment(&'a str),
    /// `/* */`, possibly spanning lines.
    BlockComment(&'a str),
    /// `/-`, which comments out the next node, entry or children block.
    SlashDash,
    /// `\`, which continues a node on the next line.
    Continue,
    Open,
    Close,
    Semicolon,
    Newline,
}

/// Splits `src` into tokens, dropping spaces. Never fails: what it cannot
/// make sense of ends up in a `Word`, and the parse check catches it.
fn lex(src: &str) -> Vec<Token<'_>> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let rest = &src[i..];
        let c = rest.chars().next().unwrap();
        let start = i;
        if c == '\n' || c == '\r' && b.get(i + 1) != Some(&b'\n') {
            out.push(Token::Newline);
            i += 1;
        } else if c.is_whitespace() || c == '\u{feff}' {
            i += c.len_utf8();
        } else if rest.starts_with("//") {
            i += rest.find(['\n', '\r']).unwrap_or(rest.len());
            out.push(Token::LineComment(src[start..i].trim_end()));
        } else if rest.starts_with("/*") {
            i = block_comment_end(src, i);
            out.push(Token::BlockComment(&src[start..i]));
        } else if rest.starts_with("/-") {
            out.push(Token::SlashDash);
            i += 2;
        } else {
            match c {
                '{' => out.push(Token::Open),
                '}' => out.push(Token::Close),
                ';' => out.push(Token::Semicolon),
                '\\' => out.push(Token::Continue),
                _ => {
                    i = word_end(src, i);
                    out.push(Token::Word(&src[start..i]));
                    continue;
                }
            }
            i += 1;
        }
    }
    out
}

/// The end of the (possibly nested) block comment starting at `i`.
fn block_comment_end(src: &str, mut i: usize) -> usize {
    let mut depth = 0;
    while i < src.len() {
        let rest = &src[i..];
        if rest.starts_with("/*") {
            depth += 1;
            i += 2;
        } else if rest.starts_with("*/") {
            depth -= 1;
            i += 2;
            if depth == 0 {
                break;
            }
        } else {
            i += rest.chars().next().unwrap().len_utf8();
        }
    }
    i
}

/// The end of the word starting at `i`: up to whitespace or a character that
/// starts another token, skipping over any strings inside it (such as in
/// `key="a b"` or `(type)"a b"`).
fn word_end(src: &str, mut i: usize) -> usize {
    while i < src.len() {
        let rest = &src[i..];
        let c = rest.chars().next().unwrap();
        if c.is_whitespace() || matches!(c, '{' | '}' | ';' | '\\') || rest.starts_with('/') {
            break;
        }
        let hashes = rest.bytes().take_while(|&b| b == b'#').count();
        if rest[hashes..].starts_with('"') {
            i = string_end(src, i + hashes, hashes);
        } else {
            i += c.len_utf8().max(hashes);
        }
    }
    i
}

/// The end of the string whose opening quote is at `i`, raw if it was
/// preceded by `hashes` `#`s.
fn string_end(src: &str, i: usize, hashes: usize) -> usize {
    let quotes = if src[i..].starts_with("\"\"\"") { "\"\"\"" } else { "\"" };
    let close = format!("{quotes}{}", "#".repeat(hashes));
    let mut j = i + quotes.len();
    while j < src.len() {
        let rest = &src[j..];
        if rest.starts_with(&close) {
            return j + close.len();
        }
        if hashes == 0 && rest.starts_with('\\') {
            j += 1;
            if j >= src.len() {
                break;
            }
        }
        j += src[j..].chars().next().unwrap().len_utf8();
    }
    src.len()
}

/// Lays tokens out one node per line.
fn print(tokens: &[Token<'_>]) -> String {
    let mut p = Printer::default();
    for (k, t) in tokens.iter().enumerate() {
        // After `{` or `}`, only a comment may share the line.
        let comment = matches!(t, Token::LineComment(_) | Token::BlockComment(_));
        if p.after_open && !comment {
            p.open_block();
        }
        if p.after_close && !comment && *t != Token::Newline {
            p.end_line();
        }
        p.after_close = false;
        match *t {
            Token::Word(w) | Token::BlockComment(w) => p.push(w),
            Token::SlashDash => {
                p.push("/-");
                p.glue = true;
            }
            Token::Continue => {
                p.push("\\");
                p.continued = true;
            }
            Token::LineComment(c) => {
                p.push(c);
                if p.after_open {
                    p.open_block();
                } else {
                    p.end_line();
                }
            }
            Token::Open => {
                p.push("{");
                p.after_open = true;
            }
            Token::Close => {
                p.end_line();
                p.depth = p.depth.saturating_sub(1);
                p.blank = false;
                p.push("}");
                p.after_close = true;
            }
            Token::Semicolon => p.end_line(),
            Token::Newline => {
                // Two newlines in a row make a blank line. Whether one is
                // printed is up to `end_line`.
                if k > 0 && tokens[k - 1] == Token::Newline {
                    p.blank = true;
                }
                if p.continued {
                    p.end_line();
                    p.continuation = true;
                    p.continued = false;
                } else {
                    p.end_line();
                }
            }
        }
    }
    if p.after_open {
        p.open_block();
    }
    p.end_line();
    p.out
}

#[derive(Default)]
struct Printer {
    out: String,
    line: String,
    depth: usize,
    /// A blank line is due before the next line.
    blank: bool,
    /// The last line ended with `{`, so no blank line may follow it.
    opened: bool,
    /// The line ends with `\`, so its newline doesn't end the node.
    continued: bool,
    /// This line continues the node on the line before.
    continuation: bool,
    /// The next token joins the last without a space.
    glue: bool,
    /// The line ends with `{`; the block's contents start on the next.
    after_open: bool,
    after_close: bool,
}

impl Printer {
    fn push(&mut self, s: &str) {
        let tight = self.glue || self.line.ends_with('=') || s.starts_with('=');
        if !self.line.is_empty() && !tight {
            self.line.push(' ');
        }
        self.line.push_str(s);
        self.glue = false;
    }

    /// Ends the line holding `{` and indents what follows it.
    fn open_block(&mut self) {
        self.end_line();
        self.depth += 1;
        self.opened = true;
        self.after_open = false;
    }

    fn end_line(&mut self) {
        if self.line.is_empty() {
            return;
        }
        if self.blank && !self.opened && !self.out.is_empty() {
            self.out.push('\n');
        }
        let indent = self.depth + usize::from(self.continuation);
        for _ in 0..indent {
            self.out.push_str(INDENT);
        }
        self.out.push_str(&self.line);
        self.out.push('\n');
        self.line.clear();
        self.blank = false;
        self.opened = false;
        self.continuation = false;
        self.glue = false;
    }
}

#[cfg(test)]
mod tests {
    use super::format;

    #[test]
    fn expands_compact_blocks() {
        let src = "path { node 0 0; node 1 1; gradient 1 { stop 0 0 0 0; stop 2 1 0 0; } }\n";
        assert_eq!(
            format(src).unwrap(),
            "path {\n    node 0 0\n    node 1 1\n    gradient 1 {\n        stop 0 0 0 0\n        stop 2 1 0 0\n    }\n}\n"
        );
    }

    #[test]
    fn fixes_indentation_and_spacing() {
        let src = "image   width = 10 height=10\n  loop{\n node 0 0\n\t\t\tnode 1   1\n  }";
        assert_eq!(
            format(src).unwrap(),
            "image width=10 height=10\nloop {\n    node 0 0\n    node 1 1\n}\n"
        );
    }

    #[test]
    fn keeps_comments_and_one_blank_line() {
        let src = "\n\nimage width=10 height=10\n\n\n\n// Background.\npath {\n\n    // First.\n    node 0 0 // origin\n\n    node 1 1 /* a */; node 2 2\n\n} // end\n\n\n";
        assert_eq!(
            format(src).unwrap(),
            "image width=10 height=10\n\n// Background.\npath {\n    // First.\n    node 0 0 // origin\n\n    node 1 1 /* a */\n    node 2 2\n} // end\n"
        );
    }

    #[test]
    fn leaves_strings_and_slashdash_alone() {
        let src = "a \"x  { y; }\" #\"raw \" // \"# (t)\"q\"\n/-b { c }\nd /-1 2\n";
        assert_eq!(
            format(src).unwrap(),
            "a \"x  { y; }\" #\"raw \" // \"# (t)\"q\"\n/-b {\n    c\n}\nd /-1 2\n"
        );
    }

    #[test]
    fn keeps_comments_after_braces_on_their_line() {
        let src = "a { // x\n\n  b { /* y */ c }\n}\n";
        assert_eq!(format(src).unwrap(), "a { // x\n    b { /* y */\n        c\n    }\n}\n");
    }

    #[test]
    fn indents_continued_lines() {
        let src = "path {\nnode 1 \\\n2\n}\n";
        assert_eq!(format(src).unwrap(), "path {\n    node 1 \\\n        2\n}\n");
    }

    #[test]
    fn is_idempotent_on_examples() {
        for entry in std::fs::read_dir("examples").unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "kdl") {
                let once = format(&std::fs::read_to_string(&path).unwrap()).unwrap();
                assert_eq!(format(&once).unwrap(), once, "{}", path.display());
            }
        }
    }

    #[test]
    fn rejects_syntax_errors() {
        assert!(matches!(format("path {"), Err(super::FormatError::Syntax(_))));
    }
}
