use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const BUILTIN_PLUGINS: &str = include_str!("../search-plugins/builtin.json");

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_RESULTS_PER_PLUGIN: usize = 60;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    Json,
    Rss,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct JsonMapping {
    pub results: String,
    pub name: String,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub seeders: Option<String>,
    pub link: String,
    #[serde(default)]
    pub link_template: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SearchPlugin {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub browse_url: Option<String>,
    pub kind: PluginKind,
    #[serde(default)]
    pub json: Option<JsonMapping>,
    #[serde(default)]
    pub site: Option<String>,
    #[serde(default, skip_deserializing)]
    pub builtin: bool,
    #[serde(default, skip_deserializing)]
    pub enabled: bool,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub name: String,
    pub source: String,
    pub size: Option<u64>,
    pub seeders: Option<u64>,
    pub link: String,
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn dig<'a>(v: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .try_fold(v, |acc, key| acc.get(key))
}

fn as_u64(v: Option<&serde_json::Value>) -> Option<u64> {
    match v? {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn as_string(v: Option<&serde_json::Value>) -> Option<String> {
    match v? {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(a) => a.first().and_then(|x| x.as_str().map(str::to_string)),
        other => Some(other.to_string()),
    }
}

pub async fn load_plugins(dir: &Path, disabled: &[String]) -> Vec<SearchPlugin> {
    let mut plugins: Vec<SearchPlugin> =
        serde_json::from_str(BUILTIN_PLUGINS).unwrap_or_default();
    for p in plugins.iter_mut() {
        p.builtin = true;
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| serde_json::from_str::<SearchPlugin>(&s).ok())
            {
                Some(mut p) => {
                    p.builtin = false;
                    plugins.retain(|x| x.id != p.id);
                    plugins.push(p);
                }
                None => eprintln!("[qtorrent] skipping unreadable plugin: {}", path.display()),
            }
        }
    }

    for p in plugins.iter_mut() {
        p.enabled = !disabled.iter().any(|d| d == &p.id);
    }
    plugins.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    plugins
}

async fn run_plugin(
    client: reqwest::Client,
    plugin: SearchPlugin,
    query: String,
) -> Result<Vec<SearchResult>, String> {
    let url = if query.is_empty() {
        match plugin.browse_url.as_ref() {
            Some(u) => u.clone(),
            None => return Ok(Vec::new()),
        }
    } else {
        plugin.url.replace("{query}", &encode(&query))
    };
    let body = client
        .get(&url)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("{}: {e}", plugin.name))?
        .error_for_status()
        .map_err(|e| format!("{}: {e}", plugin.name))?
        .text()
        .await
        .map_err(|e| format!("{}: {e}", plugin.name))?;

    let results = match plugin.kind {
        PluginKind::Json => parse_json(&plugin, &body)?,
        PluginKind::Rss => parse_rss(&plugin, &body)?,
    };
    Ok(results.into_iter().take(MAX_RESULTS_PER_PLUGIN).collect())
}

fn parse_json(plugin: &SearchPlugin, body: &str) -> Result<Vec<SearchResult>, String> {
    let map = plugin
        .json
        .as_ref()
        .ok_or_else(|| format!("{}: plugin is kind=json but has no json mapping", plugin.name))?;
    let doc: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("{}: invalid JSON ({e})", plugin.name))?;

    let items = dig(&doc, &map.results)
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("{}: no results at '{}'", plugin.name, map.results))?;

    Ok(items
        .iter()
        .filter_map(|item| {
            let name = as_string(item.get(&map.name))?;
            let raw = as_string(item.get(&map.link))?;
            let link = match &map.link_template {
                Some(t) => t.replace("{value}", &raw),
                None => raw,
            };
            Some(SearchResult {
                name,
                source: plugin.name.clone(),
                size: map.size.as_deref().and_then(|k| as_u64(item.get(k))),
                seeders: map.seeders.as_deref().and_then(|k| as_u64(item.get(k))),
                link,
            })
        })
        .collect())
}

fn parse_rss(plugin: &SearchPlugin, body: &str) -> Result<Vec<SearchResult>, String> {
    let doc = roxmltree::Document::parse(body)
        .map_err(|e| format!("{}: invalid XML ({e})", plugin.name))?;

    let mut out = Vec::new();
    for item in doc
        .descendants()
        .filter(|n| n.has_tag_name("item") || n.has_tag_name("entry"))
    {
        let child_text = |tag: &str| {
            item.children()
                .find(|c| c.has_tag_name(tag))
                .and_then(|c| c.text())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };

        let name = match child_text("title") {
            Some(t) => t,
            None => continue,
        };

        let enclosure = item.children().find(|c| c.has_tag_name("enclosure"));
        let link = enclosure
            .and_then(|e| e.attribute("url"))
            .map(str::to_string)
            .or_else(|| child_text("link"))
            .or_else(|| {
                item.children()
                    .find(|c| c.has_tag_name("link"))
                    .and_then(|c| c.attribute("href"))
                    .map(str::to_string)
            });
        let link = match link {
            Some(l) => l,
            None => continue,
        };

        let size = enclosure
            .and_then(|e| e.attribute("length"))
            .and_then(|s| s.parse().ok())
            .or_else(|| child_text("size").and_then(|s| s.parse().ok()));

        let seeders = item
            .children()
            .find(|c| c.tag_name().name() == "seeders")
            .and_then(|c| c.text())
            .and_then(|s| s.trim().parse().ok());

        out.push(SearchResult {
            name,
            source: plugin.name.clone(),
            size,
            seeders,
            link,
        });
    }
    Ok(out)
}

pub async fn search(
    plugins: Vec<SearchPlugin>,
    query: String,
) -> (Vec<SearchResult>, Vec<String>) {
    let client = match reqwest::Client::builder()
        .user_agent("quantum-torrent")
        .build()
    {
        Ok(c) => c,
        Err(e) => return (Vec::new(), vec![format!("HTTP client: {e}")]),
    };

    let handles: Vec<_> = plugins
        .into_iter()
        .filter(|p| p.enabled)
        .map(|p| {
            let client = client.clone();
            let query = query.clone();
            tauri::async_runtime::spawn(run_plugin(client, p, query))
        })
        .collect();

    let mut results = Vec::new();
    let mut errors = Vec::new();
    for h in handles {
        match h.await {
            Ok(Ok(mut r)) => results.append(&mut r),
            Ok(Err(e)) => errors.push(e),
            Err(e) => errors.push(format!("search task failed: {e}")),
        }
    }

    results.sort_by(|a, b| {
        b.seeders
            .unwrap_or(0)
            .cmp(&a.seeders.unwrap_or(0))
            .then_with(|| b.size.unwrap_or(0).cmp(&a.size.unwrap_or(0)))
    });
    (results, errors)
}

pub fn plugins_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("search-plugins")
}
