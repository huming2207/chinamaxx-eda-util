//! CAD-independent assembly workflows shared by the desktop and command line apps.
pub mod board;
pub mod cache;
pub mod export;
pub mod library;
pub mod input;
pub mod project;
pub mod sexpr;
pub mod supplier;

use anyhow::{ensure, Result};

pub fn lcsc_id(s: &str) -> Result<String> {
    let s = s.trim().to_ascii_uppercase();
    ensure!(
        s.starts_with('C') && s.len() > 1 && s[1..].bytes().all(|b| b.is_ascii_digit()),
        "Invalid LCSC ID {s:?}; expected C followed by digits"
    );
    Ok(s)
}

pub fn finite(s: &str) -> Result<f64> {
    let n: f64 = s.parse()?;
    ensure!(n.is_finite(), "Non-finite coordinate: {s}");
    Ok(n)
}

pub fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn quoted(s: &str) -> String {
    format!(
        "\"{}\"",
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    )
}
