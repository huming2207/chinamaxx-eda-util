//! Supplier access. curl supplies platform TLS/proxy support; no Python runtime.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
const SEARCH: &str =
    "https://jlcpcb.com/api/overseas-pcb-order/v1/shoppingCart/smtGood/selectSmtComponentList";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Part {
    pub lcsc: String,
    pub name: String,
    pub model: String,
    pub brand: String,
    pub package: String,
    pub description: String,
    pub stock: u64,
    pub basic: bool,
    pub price: Option<f64>,
    pub datasheet: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Search {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<crate::cache::CacheInfo>,
    pub total: u64,
    pub results: Vec<Part>,
}

fn request(url: &str, payload: Option<&Value>) -> Result<Value> {
    let mut cmd = Command::new("curl");
    cmd.args([
        "--fail-with-body",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--proto-redir",
        "=https",
        "--connect-timeout",
        "10",
        "--max-time",
        "30",
        "--max-filesize",
        "16777216",
        "--user-agent",
        "ChinamaxxBOM/0.1",
    ]);
    if payload.is_some() {
        cmd.args([
            "--header",
            "Content-Type: application/json",
            "--header",
            "Origin: https://jlcpcb.com",
            "--header",
            "Referer: https://jlcpcb.com/parts",
            "--data-binary",
            "@-",
        ]);
    }
    let mut child = cmd
        .arg(url)
        .stdin(if payload.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Start curl; install curl for online supplier access")?;
    if let Some(p) = payload {
        child
            .stdin
            .take()
            .context("curl stdin")?
            .write_all(&serde_json::to_vec(p)?)?;
    }
    let out = child.wait_with_output()?;
    ensure!(
        out.status.success(),
        "Supplier request failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout)
        .context("Supplier returned invalid JSON (possibly a service error)")
}
fn string(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").into()
}
fn integer(v: &Value) -> u64 {
    v.as_u64().or_else(|| v.as_str()?.parse().ok()).unwrap_or(0)
}
pub fn parse_search(raw: Value) -> Result<Search> {
    if let Some(code) = raw.get("code") {
        ensure!(
            code.as_i64() == Some(200) || code.as_str() == Some("200"),
            "JLCPCB rejected search: {}",
            raw["message"]
        );
    }
    let info = raw
        .pointer("/data/componentPageInfo")
        .context("JLCPCB response is missing componentPageInfo")?;
    let items = info["list"]
        .as_array()
        .context("JLCPCB response is missing the parts list")?;
    let mut results = vec![];
    for p in items {
        let lcsc = crate::lcsc_id(&string(p, "componentCode"))?;
        results.push(Part {
            lcsc,
            name: string(p, "componentName"),
            model: string(p, "componentModelEn"),
            brand: string(p, "componentBrandEn"),
            package: string(p, "componentSpecificationEn"),
            description: string(p, "describe"),
            stock: integer(&p["stockCount"]),
            basic: p["componentLibraryType"] == "base",
            price: p
                .pointer("/componentPrices/0/productPrice")
                .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok())),
            datasheet: string(p, "dataManualUrl"),
        });
    }
    Ok(Search {
        cache: None,
        total: integer(&info["total"]),
        results,
    })
}
pub fn search(keyword: &str, page: u32, basic: bool, in_stock: bool) -> Result<Search> {
    ensure!(!keyword.trim().is_empty(), "Enter a search term");
    ensure!(page > 0, "Page is 1-based");
    let keyword = keyword
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let mut payload = json!({"keyword":keyword,"currentPage":page,"pageSize":50});
    if basic {
        payload["componentLibraryType"] = json!("base");
    }
    if in_stock {
        payload["presaleTypes"] = json!(["stock"]);
    }
    let key = format!(
        "jlc-search-v1:{SEARCH}:{}",
        serde_json::to_string(&payload)?
    );
    let (mut results, metadata) = crate::cache::Cache::for_user()?
        .get_or_fetch(&key, || parse_search(request(SEARCH, Some(&payload))?))?;
    results.cache = Some(metadata);
    results
        .results
        .retain(|p| (!basic || p.basic) && (!in_stock || p.stock > 0));
    Ok(results)
}
pub fn search_catalog(path: &Path, keyword: &str, basic: bool, in_stock: bool) -> Result<Search> {
    let raw: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let parts: Vec<Part> = if raw.is_array() {
        serde_json::from_value(raw)?
    } else {
        serde_json::from_value::<Search>(raw)?.results
    };
    let terms: Vec<_> = keyword
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let results: Vec<_> = parts
        .into_iter()
        .filter(|p| {
            let text = format!(
                "{} {} {} {} {} {}",
                p.lcsc, p.name, p.model, p.brand, p.package, p.description
            )
            .to_lowercase();
            (!basic || p.basic)
                && (!in_stock || p.stock > 0)
                && terms.iter().all(|t| text.contains(t))
        })
        .collect();
    Ok(Search {
        cache: None,
        total: results.len() as u64,
        results,
    })
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub lcsc: String,
    pub symbols: Vec<Value>,
    pub footprint: Value,
}
pub fn result(v: Value) -> Result<Value> {
    if v.get("success").and_then(Value::as_bool) == Some(false) {
        bail!("EasyEDA rejected request: {}", v["message"]);
    }
    Ok(v.get("result").cloned().unwrap_or(v))
}
fn easyeda(url: &str) -> Result<Value> {
    let (value, _) =
        crate::cache::Cache::for_user()?.get_or_fetch(&format!("easyeda-v1:{url}"), || {
            let value = result(request(url, None)?)?;
            ensure!(
                value.is_object() || value.is_array(),
                "EasyEDA returned no component data"
            );
            Ok(value)
        })?;
    Ok(value)
}
pub fn fetch(id: &str) -> Result<Bundle> {
    let lcsc = crate::lcsc_id(id)?;
    let ids = easyeda(&format!("https://easyeda.com/api/products/{lcsc}/svgs"))?;
    let ids = ids
        .as_array()
        .context("EasyEDA returned no component IDs")?;
    ensure!(
        ids.len() >= 2,
        "EasyEDA has no symbol/footprint pair for {lcsc}"
    );
    let mut data = vec![];
    for id in ids {
        let uuid = id["component_uuid"]
            .as_str()
            .context("Missing component UUID")?;
        ensure!(
            uuid.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'),
            "Invalid component UUID"
        );
        data.push(easyeda(&format!(
            "https://easyeda.com/api/components/{uuid}"
        ))?);
    }
    let footprint = data.pop().context("Missing footprint")?;
    Ok(Bundle {
        lcsc,
        symbols: data,
        footprint,
    })
}
