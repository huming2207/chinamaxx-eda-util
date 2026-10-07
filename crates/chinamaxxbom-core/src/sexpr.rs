//! A bounded S-expression reader retaining byte ranges for lossless field edits.
use anyhow::{bail, ensure, Result};
use std::ops::Range;

#[derive(Debug, Clone)]
pub struct Node {
    pub atom: Option<String>,
    pub children: Vec<Node>,
    pub span: Range<usize>,
}
impl Node {
    pub fn text(&self) -> &str {
        self.atom.as_deref().unwrap_or("")
    }
    pub fn at(&self, i: usize) -> &str {
        self.children.get(i).map(Self::text).unwrap_or("")
    }
    pub fn tag(&self) -> &str {
        self.at(0)
    }
    pub fn child(&self, tag: &str) -> Option<&Node> {
        self.children.iter().find(|n| n.tag() == tag)
    }
    pub fn all<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a Node> {
        self.children.iter().filter(move |n| n.tag() == tag)
    }
    pub fn value(&self, tag: &str) -> &str {
        self.child(tag).map(|n| n.at(1)).unwrap_or("")
    }
    pub fn has_atom(&self, a: &str) -> bool {
        self.children.iter().any(|n| n.text() == a)
    }
}

pub fn parse(s: &str) -> Result<Node> {
    fn skip(s: &[u8], i: &mut usize) {
        while *i < s.len() && s[*i].is_ascii_whitespace() {
            *i += 1;
        }
    }
    fn node(s: &str, i: &mut usize, depth: usize) -> Result<Node> {
        ensure!(depth < 128, "S-expression nesting exceeds 128");
        let b = s.as_bytes();
        skip(b, i);
        ensure!(*i < b.len(), "Unexpected end of S-expression");
        let start = *i;
        if b[*i] == b'(' {
            *i += 1;
            let mut children = vec![];
            loop {
                skip(b, i);
                ensure!(*i < b.len(), "Unclosed list at byte {start}");
                if b[*i] == b')' {
                    *i += 1;
                    break;
                }
                children.push(node(s, i, depth + 1)?);
            }
            Ok(Node {
                atom: None,
                children,
                span: start..*i,
            })
        } else if b[*i] == b'"' {
            *i += 1;
            let mut out = Vec::new();
            loop {
                ensure!(*i < b.len(), "Unclosed string at byte {start}");
                let c = b[*i];
                *i += 1;
                if c == b'"' {
                    break;
                }
                if c == b'\\' {
                    ensure!(*i < b.len(), "Trailing escape");
                    let e = b[*i];
                    *i += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'"' | b'\\' => out.push(e),
                        _ => {
                            out.push(b'\\');
                            out.push(e);
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            Ok(Node {
                atom: Some(String::from_utf8(out)?),
                children: vec![],
                span: start..*i,
            })
        } else {
            if b[*i] == b')' {
                bail!("Unexpected ')' at byte {i}");
            }
            while *i < b.len() && !b[*i].is_ascii_whitespace() && !b"()".contains(&b[*i]) {
                *i += 1;
            }
            Ok(Node {
                atom: Some(s[start..*i].into()),
                children: vec![],
                span: start..*i,
            })
        }
    }
    let mut i = 0;
    let n = node(s, &mut i, 0)?;
    skip(s.as_bytes(), &mut i);
    ensure!(i == s.len(), "Trailing S-expression content");
    Ok(n)
}
