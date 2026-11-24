//! 多言語対応。言語ファイル（JSON）は src/locales/*.json をビルドに埋め込み、
//! さらにデータフォルダの `locales/<コード>.json` があれば、その内容で上書き・追加できる。
use serde_json::{Map, Value};
use std::path::Path;

/// 同梱の言語。先頭の en が、訳が欠けたときの代替になる
pub const BUILTIN: &[(&str, &str)] = &[
    ("en", include_str!("../../src/locales/en.json")),
    ("ja", include_str!("../../src/locales/ja.json")),
    ("zh-CN", include_str!("../../src/locales/zh-CN.json")),
    ("zh-TW", include_str!("../../src/locales/zh-TW.json")),
    ("ko", include_str!("../../src/locales/ko.json")),
    ("es", include_str!("../../src/locales/es.json")),
    ("fr", include_str!("../../src/locales/fr.json")),
    ("de", include_str!("../../src/locales/de.json")),
];

fn parse(s: &str) -> Map<String, Value> {
    match serde_json::from_str::<Value>(s.trim_start_matches('\u{feff}')) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn safe_code(code: &str) -> bool {
    !code.is_empty() && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn user_file(dir: &Path, code: &str) -> Option<Map<String, Value>> {
    if !safe_code(code) {
        return None;
    }
    let s = std::fs::read_to_string(dir.join("locales").join(format!("{code}.json"))).ok()?;
    Some(parse(&s))
}

pub fn exists(dir: &Path, code: &str) -> bool {
    BUILTIN.iter().any(|(c, _)| *c == code) || user_file(dir, code).is_some()
}

/// en → 同梱の言語 → ユーザーの言語ファイル、の順に重ねた辞書
pub fn dict(dir: &Path, code: &str) -> Map<String, Value> {
    let mut m = parse(BUILTIN[0].1);
    if let Some((_, s)) = BUILTIN.iter().find(|(c, _)| *c == code) {
        m.extend(parse(s));
    }
    if let Some(u) = user_file(dir, code) {
        m.extend(u);
    }
    m
}

fn system_lang(dir: &Path) -> String {
    let loc = sys_locale::get_locale().unwrap_or_default().replace('_', "-");
    let low = loc.to_lowercase();
    let cand = if low.starts_with("zh") {
        if ["tw", "hk", "mo", "hant"].iter().any(|k| low.contains(k)) { "zh-TW".to_string() } else { "zh-CN".to_string() }
    } else {
        loc.clone()
    };
    if exists(dir, &cand) {
        return cand;
    }
    let base = low.split('-').next().unwrap_or("en").to_string();
    if exists(dir, &base) {
        return base;
    }
    "en".into()
}

/// 設定値（"auto" または言語コード）→ 実際に使う言語コード
pub fn resolve(setting: &str, dir: &Path) -> String {
    if setting.is_empty() || setting == "auto" || !exists(dir, setting) {
        system_lang(dir)
    } else {
        setting.to_string()
    }
}

/// 選択肢用の (コード, 表示名)。同梱 + ユーザー追加
pub fn list(dir: &Path) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = BUILTIN
        .iter()
        .map(|(c, s)| {
            let name = parse(s).get("_name").and_then(|v| v.as_str()).unwrap_or(c).to_string();
            (c.to_string(), name)
        })
        .collect();
    if let Ok(rd) = std::fs::read_dir(dir.join("locales")) {
        let mut extra: Vec<(String, String)> = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Some(code) = p.file_stem().and_then(|x| x.to_str()).map(str::to_string) else { continue };
            if !safe_code(&code) || out.iter().any(|(c, _)| *c == code) {
                continue;
            }
            let name = user_file(dir, &code)
                .and_then(|m| m.get("_name").and_then(|v| v.as_str()).map(str::to_string))
                .unwrap_or_else(|| code.clone());
            extra.push((code, name));
        }
        extra.sort();
        out.extend(extra);
    }
    out
}

pub fn tr(dir: &Path, setting: &str, key: &str, vars: &[(&str, &str)]) -> String {
    let code = resolve(setting, dir);
    let mut s = dict(dir, &code)
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or(key)
        .to_string();
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}
