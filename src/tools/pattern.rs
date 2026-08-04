//! Minimal, dependency-free pattern matching used by the structured tools.
//!
//! This module provides two small engines implemented purely on `std` so that
//! the `grep` and `glob` tools work even when ripgrep is not installed and
//! without pulling in the `regex`/`glob` crates:
//!
//! - [`regex_is_match`] — a backtracking regex matcher supporting literals,
//!   `.`, `*`, `+`, `?`, `^`, `$`, `[...]`, `[^...]`, `(...)` and `|`.
//! - [`glob_match`] — a path-segment glob matcher supporting `*`, `?`, `**`
//!   and `[...]`.

/// A node in the parsed regex AST.
#[derive(Debug, Clone)]
enum RegexNode {
    /// Matches nothing (empty sequence).
    Empty,
    /// Matches a single literal character.
    Char(char),
    /// Matches any single character (`.`).
    Any,
    /// Matches a single character in (or, if negated, not in) a class.
    Class {
        ranges: Vec<(char, char)>,
        negated: bool,
    },
    /// Matches only at the start of the text (`^`).
    AnchorStart,
    /// Matches only at the end of the text (`$`).
    AnchorEnd,
    /// Matches a sequence of nodes.
    Seq(Vec<RegexNode>),
    /// Matches any one of several alternatives (`a|b`).
    Alt(Vec<RegexNode>),
    /// Matches zero or more of the inner node (`a*`).
    Star(Box<RegexNode>),
    /// Matches one or more of the inner node (`a+`).
    Plus(Box<RegexNode>),
    /// Matches zero or one of the inner node (`a?`).
    Opt(Box<RegexNode>),
}

/// A recursive-descent parser for the supported regex subset.
struct RegexParser {
    chars: Vec<char>,
    pos: usize,
}

impl RegexParser {
    fn new(pattern: &str) -> Self {
        Self {
            chars: pattern.chars().collect(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn parse(&mut self) -> Result<RegexNode, String> {
        self.parse_alt()
    }

    fn parse_alt(&mut self) -> Result<RegexNode, String> {
        let mut alts = vec![self.parse_seq()?];
        while self.peek() == Some('|') {
            self.pos += 1;
            alts.push(self.parse_seq()?);
        }
        if alts.len() == 1 {
            Ok(alts.pop().unwrap())
        } else {
            Ok(RegexNode::Alt(alts))
        }
    }

    fn parse_seq(&mut self) -> Result<RegexNode, String> {
        let mut nodes = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            nodes.push(self.parse_piece()?);
        }
        match nodes.len() {
            0 => Ok(RegexNode::Empty),
            1 => Ok(nodes.pop().unwrap()),
            _ => Ok(RegexNode::Seq(nodes)),
        }
    }

    fn parse_piece(&mut self) -> Result<RegexNode, String> {
        let atom = self.parse_atom()?;
        match self.peek() {
            Some('*') => {
                self.pos += 1;
                Ok(RegexNode::Star(Box::new(atom)))
            }
            Some('+') => {
                self.pos += 1;
                Ok(RegexNode::Plus(Box::new(atom)))
            }
            Some('?') => {
                self.pos += 1;
                Ok(RegexNode::Opt(Box::new(atom)))
            }
            _ => Ok(atom),
        }
    }

    fn parse_atom(&mut self) -> Result<RegexNode, String> {
        let c = self.next().ok_or("unexpected end of pattern")?;
        match c {
            '.' => Ok(RegexNode::Any),
            '^' => Ok(RegexNode::AnchorStart),
            '$' => Ok(RegexNode::AnchorEnd),
            '[' => self.parse_class(),
            '(' => {
                let inner = self.parse_alt()?;
                if self.next() != Some(')') {
                    return Err("missing closing ')'".to_string());
                }
                Ok(inner)
            }
            '\\' => {
                let esc = self.next().ok_or("trailing backslash")?;
                Ok(RegexNode::Char(esc))
            }
            ')' => Err("unexpected ')'".to_string()),
            '|' => Err("unexpected '|'".to_string()),
            other => Ok(RegexNode::Char(other)),
        }
    }

    fn parse_class(&mut self) -> Result<RegexNode, String> {
        let mut ranges: Vec<(char, char)> = Vec::new();
        let mut negated = false;
        if self.peek() == Some('^') {
            negated = true;
            self.pos += 1;
        }
        // A literal ']' as the first character is allowed.
        let mut first = true;
        loop {
            let c = self.next().ok_or("unterminated character class")?;
            if c == ']' && !first {
                break;
            }
            first = false;
            if c == '\\' {
                let esc = self.next().ok_or("trailing backslash in class")?;
                ranges.push((esc, esc));
            } else if self.peek() == Some('-') && self.chars.get(self.pos + 1) != Some(&']') {
                // Range: c - next
                self.pos += 1; // consume '-'
                let end = self.next().ok_or("unterminated range")?;
                ranges.push((c, end));
            } else {
                ranges.push((c, c));
            }
        }
        Ok(RegexNode::Class { ranges, negated })
    }
}

/// Return every possible end position after matching `node` starting at `pos`.
fn match_node(node: &RegexNode, text: &[char], pos: usize) -> Vec<usize> {
    match node {
        RegexNode::Empty => vec![pos],
        RegexNode::Char(c) => {
            if pos < text.len() && text[pos] == *c {
                vec![pos + 1]
            } else {
                vec![]
            }
        }
        RegexNode::Any => {
            if pos < text.len() {
                vec![pos + 1]
            } else {
                vec![]
            }
        }
        RegexNode::Class { ranges, negated } => {
            if pos < text.len() {
                let c = text[pos];
                let in_class = ranges.iter().any(|(a, b)| c >= *a && c <= *b);
                if in_class != *negated {
                    vec![pos + 1]
                } else {
                    vec![]
                }
            } else {
                vec![]
            }
        }
        RegexNode::AnchorStart => {
            if pos == 0 {
                vec![pos]
            } else {
                vec![]
            }
        }
        RegexNode::AnchorEnd => {
            if pos == text.len() {
                vec![pos]
            } else {
                vec![]
            }
        }
        RegexNode::Seq(nodes) => {
            let mut ends = vec![pos];
            for n in nodes {
                let mut next = Vec::new();
                for e in ends {
                    next.extend(match_node(n, text, e));
                }
                ends = next;
                if ends.is_empty() {
                    break;
                }
            }
            ends
        }
        RegexNode::Alt(alts) => {
            let mut ends = Vec::new();
            for a in alts {
                ends.extend(match_node(a, text, pos));
            }
            ends
        }
        RegexNode::Star(inner) => {
            let mut ends = vec![pos];
            let mut frontier = vec![pos];
            let mut seen = std::collections::HashSet::new();
            while let Some(p) = frontier.pop() {
                if seen.insert(p) {
                    for e in match_node(inner, text, p) {
                        if !seen.contains(&e) {
                            ends.push(e);
                            frontier.push(e);
                        }
                    }
                }
            }
            ends
        }
        RegexNode::Plus(inner) => {
            let mut ends = Vec::new();
            for e in match_node(inner, text, pos) {
                ends.extend(match_node(&RegexNode::Star(inner.clone()), text, e));
            }
            ends
        }
        RegexNode::Opt(inner) => {
            let mut ends = vec![pos];
            ends.extend(match_node(inner, text, pos));
            ends
        }
    }
}

/// Return `true` if `text` contains a match for `pattern`.
///
/// The pattern is unanchored by default; use `^`/`$` to anchor explicitly.
pub fn regex_is_match(pattern: &str, text: &str) -> Result<bool, String> {
    let node = RegexParser::new(pattern).parse()?;
    let chars: Vec<char> = text.chars().collect();
    // Unanchored match: the pattern matches if it can start at any position.
    // Anchors (`^`/`$`) constrain the match within the pattern itself.
    for start in 0..=chars.len() {
        if !match_node(&node, &chars, start).is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Match a single path segment (no `/`) against a glob fragment supporting
/// `*`, `?` and `[...]`.
fn segment_match(pat: &[char], text: &[char]) -> bool {
    match (pat.first(), text.first()) {
        (None, None) => true,
        (Some('*'), _) => {
            segment_match(&pat[1..], text) || (!text.is_empty() && segment_match(pat, &text[1..]))
        }
        (Some('?'), Some(_)) => segment_match(&pat[1..], &text[1..]),
        (Some('['), _) => {
            // Parse a character class at the head of the pattern.
            let mut i = 1;
            let mut negated = false;
            if pat.get(i) == Some(&'^') {
                negated = true;
                i += 1;
            }
            let mut ranges: Vec<(char, char)> = Vec::new();
            let mut first = true;
            loop {
                match pat.get(i) {
                    Some(&']') if !first => {
                        i += 1;
                        break;
                    }
                    Some(&c) => {
                        first = false;
                        if pat.get(i + 1) == Some(&'-') && pat.get(i + 2) != Some(&']') {
                            let end = pat[i + 2];
                            ranges.push((c, end));
                            i += 3;
                        } else {
                            ranges.push((c, c));
                            i += 1;
                        }
                    }
                    None => return false, // unterminated class
                }
            }
            match text.first() {
                Some(&c) => {
                    let in_class = ranges.iter().any(|(a, b)| c >= *a && c <= *b);
                    if in_class != negated {
                        segment_match(&pat[i..], &text[1..])
                    } else {
                        false
                    }
                }
                None => false,
            }
        }
        (Some(a), Some(b)) if a == b => segment_match(&pat[1..], &text[1..]),
        _ => false,
    }
}

/// Match a path against a glob pattern.
///
/// Supports `*` and `?` within a segment, `**` across segments, and `[...]`
/// character classes. The pattern is matched against the whole path.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pat_segs: Vec<String> = pattern
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    let path_segs: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    match_segments(&pat_segs, &path_segs)
}

fn match_segments(pat: &[String], path: &[String]) -> bool {
    match (pat.first(), path.first()) {
        (None, None) => true,
        (Some(p), _) if p == "**" => {
            match_segments(&pat[1..], path) || (!path.is_empty() && match_segments(pat, &path[1..]))
        }
        (Some(p), Some(s)) => {
            let pchars: Vec<char> = p.chars().collect();
            let schars: Vec<char> = s.chars().collect();
            segment_match(&pchars, &schars) && match_segments(&pat[1..], &path[1..])
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regex_literal() {
        assert!(regex_is_match("hello", "say hello world").unwrap());
        assert!(!regex_is_match("xyz", "hello world").unwrap());
    }

    #[test]
    fn regex_dot_and_quantifiers() {
        assert!(regex_is_match("h.llo", "hello").unwrap());
        assert!(regex_is_match("he*llo", "heeeello").unwrap());
        assert!(regex_is_match("colou?r", "color").unwrap());
        assert!(regex_is_match("colou?r", "colour").unwrap());
        assert!(regex_is_match("a+", "aaa").unwrap());
    }

    #[test]
    fn regex_anchors() {
        assert!(regex_is_match("^start", "start here").unwrap());
        assert!(!regex_is_match("^start", "not start").unwrap());
        assert!(regex_is_match("end$", "the end").unwrap());
        assert!(!regex_is_match("end$", "the ending").unwrap());
    }

    #[test]
    fn regex_class_and_alt() {
        assert!(regex_is_match("[abc]at", "cat").unwrap());
        assert!(regex_is_match("[^abc]at", "hat").unwrap());
        assert!(!regex_is_match("[^abc]at", "cat").unwrap());
        assert!(regex_is_match("cat|dog", "a dog here").unwrap());
        assert!(regex_is_match("(ab)+", "ababab").unwrap());
    }

    #[test]
    fn regex_invalid_pattern_errors() {
        assert!(regex_is_match("[abc", "x").is_err());
        assert!(regex_is_match("(ab", "x").is_err());
    }

    #[test]
    fn glob_basic() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(!glob_match("*.rs", "main.txt"));
        assert!(glob_match("src/*.rs", "src/main.rs"));
        assert!(!glob_match("src/*.rs", "src/sub/main.rs"));
    }

    #[test]
    fn glob_double_star() {
        assert!(glob_match("**/*.rs", "src/sub/main.rs"));
        assert!(glob_match("**/*.rs", "main.rs"));
        assert!(glob_match("src/**", "src/a/b/c.rs"));
    }

    #[test]
    fn glob_question_and_class() {
        assert!(glob_match("file?.txt", "file1.txt"));
        assert!(!glob_match("file?.txt", "file12.txt"));
        assert!(glob_match("file[0-9].txt", "file5.txt"));
        assert!(!glob_match("file[0-9].txt", "filex.txt"));
    }
}
