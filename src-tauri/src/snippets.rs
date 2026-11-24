use crate::store::{make_preview, normalize, terms, Row, LIMIT};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

pub const MAX_SNIPPETS: usize = 30;

const SAMPLE: &str = r#"# 定型文（最大30個）。保存すると次回の呼び出し時に反映されます。
# 本文で使える変数: {date} {time} {datetime} {clipboard}
# body は複数行にしたい場合 """ ... """ で囲みます。

[[snippet]]
title = "挨拶"
body = "お世話になっております。"

[[snippet]]
title = "今日の日付"
body = "{date}"

[[snippet]]
title = "署名"
body = """
--
氏名
"""
"#;

#[derive(Deserialize, Serialize)]
struct File {
    #[serde(default)]
    snippet: Vec<Snip>,
}

#[derive(Deserialize, Serialize)]
struct Snip {
    title: String,
    body: String,
}

pub struct Snippet {
    pub title: String,
    pub body: String,
    norm: String,
}

pub struct Snippets {
    pub path: PathBuf,
    mtime: Option<SystemTime>,
    pub list: Vec<Snippet>,
    error: Option<String>,
}

impl Snippets {
    /// エディタ用。書式エラー中は Err（上書きで内容を壊さないため）
    pub fn dto(&self) -> Result<Vec<crate::SnipDto>, String> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        Ok(self
            .list
            .iter()
            .map(|s| crate::SnipDto { title: s.title.clone(), body: s.body.clone() })
            .collect())
    }

    pub fn save(&mut self, items: Vec<(String, String)>) -> Result<(), String> {
        let f = File {
            snippet: items
                .into_iter()
                .take(MAX_SNIPPETS)
                .map(|(title, body)| Snip { title, body })
                .collect(),
        };
        let body = toml::to_string(&f).map_err(|e| e.to_string())?;
        let header = "# 定型文（最大30個）。設定画面の「定型文」から編集できます。\n# 本文で使える変数: {date} {time} {datetime} {clipboard}\n\n";
        std::fs::write(&self.path, format!("{header}{body}")).map_err(|e| e.to_string())?;
        self.reload();
        Ok(())
    }

    pub fn new(path: PathBuf) -> Snippets {
        if !path.exists() {
            let _ = std::fs::write(&path, SAMPLE);
        }
        let mut s = Snippets { path, mtime: None, list: vec![], error: None };
        s.reload();
        s
    }

    fn reload(&mut self) {
        self.mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        let text = std::fs::read_to_string(&self.path).unwrap_or_default();
        self.error = None;
        match toml::from_str::<File>(&text) {
            Ok(f) => {
                self.list = f
                    .snippet
                    .into_iter()
                    .take(MAX_SNIPPETS)
                    .map(|s| Snippet {
                        norm: normalize(&format!("{} {}", s.title, s.body)),
                        title: s.title,
                        body: s.body,
                    })
                    .collect();
            }
            Err(e) => {
                // 書式エラー時は直前の内容を保持し、エラーを1件目に出す
                eprintln!("snippets.toml parse error: {e}");
                self.error = Some(e.to_string());
                self.list = vec![Snippet {
                    title: "⚠ snippets.toml の書式エラー".into(),
                    body: e.to_string(),
                    norm: String::new(),
                }];
            }
        }
    }

    pub fn reload_if_changed(&mut self) {
        let m = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if m != self.mtime {
            self.reload();
        }
    }

    pub fn search(&self, query: &str) -> Vec<Row> {
        let ts = terms(query);
        self.list
            .iter()
            .enumerate()
            .filter(|(_, s)| ts.iter().all(|t| s.norm.contains(t.as_str())))
            .take(LIMIT)
            .map(|(i, s)| Row {
                id: i as i64,
                label: s.title.clone(),
                preview: make_preview(&s.body),
                pinned: false,
                ts: 0,
            })
            .collect()
    }

    pub fn get(&self, idx: i64) -> Option<&Snippet> {
        self.list.get(idx as usize)
    }
}

pub fn expand(body: &str, clipboard: &str) -> String {
    let now = chrono::Local::now();
    body.replace("{datetime}", &now.format("%Y-%m-%d %H:%M").to_string())
        .replace("{date}", &now.format("%Y-%m-%d").to_string())
        .replace("{time}", &now.format("%H:%M").to_string())
        .replace("{clipboard}", clipboard)
}
