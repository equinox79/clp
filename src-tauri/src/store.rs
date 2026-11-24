use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;
use unicode_normalization::UnicodeNormalization;

pub const PREVIEW_CHARS: usize = 200;
pub const NORM_CHARS: usize = 2000;
pub const LIMIT: usize = 50;

#[derive(Serialize, Clone)]
pub struct Row {
    pub id: i64,
    pub label: String,
    pub preview: String,
    pub pinned: bool,
    pub ts: i64,
}

/// 検索用正規化: NFKC（全角半角）+ 小文字 + カタカナ→ひらがな
pub fn normalize(s: &str) -> String {
    s.nfkc()
        .flat_map(|c| c.to_lowercase())
        .map(|c| {
            let u = c as u32;
            if (0x30A1..=0x30F6).contains(&u) {
                char::from_u32(u - 0x60).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

pub fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

pub fn make_preview(s: &str) -> String {
    let mut out = String::new();
    let mut prev_nl = false;
    for c in s.trim_start().chars() {
        if out.chars().count() >= PREVIEW_CHARS {
            break;
        }
        match c {
            '\r' => {}
            '\n' => {
                if !prev_nl {
                    out.push('⏎');
                }
                prev_nl = true;
            }
            '\t' => {
                out.push(' ');
                prev_nl = false;
            }
            _ => {
                out.push(c);
                prev_nl = false;
            }
        }
    }
    out
}

pub fn terms(query: &str) -> Vec<String> {
    normalize(query)
        .split_whitespace()
        .map(|s| s.to_string())
        .collect()
}

struct Item {
    id: i64,
    preview: String,
    norm: String,
    pinned: bool,
    last_used: i64,
}

pub struct Store {
    conn: Connection,
    items: Vec<Item>,
    max_items: usize,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl Store {
    pub fn open(path: &Path, max_items: usize) -> rusqlite::Result<Store> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS items(
                id INTEGER PRIMARY KEY,
                hash INTEGER NOT NULL UNIQUE,
                text TEXT NOT NULL,
                pinned INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                last_used_at INTEGER NOT NULL
             );",
        )?;
        let mut items = Vec::new();
        {
            let mut st = conn.prepare(
                "SELECT id, substr(text,1,?1), pinned, last_used_at FROM items",
            )?;
            let rows = st.query_map(params![NORM_CHARS as i64], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?;
            for r in rows {
                let (id, head, pinned, last_used) = r?;
                items.push(Item {
                    id,
                    preview: make_preview(&head),
                    norm: normalize(&head),
                    pinned: pinned != 0,
                    last_used,
                });
            }
        }
        let mut s = Store { conn, items, max_items };
        s.sort();
        Ok(s)
    }

    fn sort(&mut self) {
        self.items
            .sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.last_used.cmp(&a.last_used)));
    }

    pub fn insert(&mut self, text: &str) {
        let hash = fnv(text) as i64;
        let now = now_ms();
        let existing: Option<i64> = self
            .conn
            .query_row("SELECT id FROM items WHERE hash=?1", params![hash], |r| r.get(0))
            .ok();
        if let Some(id) = existing {
            self.touch(id);
            return;
        }
        if self
            .conn
            .execute(
                "INSERT INTO items(hash,text,pinned,created_at,last_used_at) VALUES(?1,?2,0,?3,?3)",
                params![hash, text, now],
            )
            .is_err()
        {
            return;
        }
        let id = self.conn.last_insert_rowid();
        let head: String = text.chars().take(NORM_CHARS).collect();
        self.items.push(Item {
            id,
            preview: make_preview(&head),
            norm: normalize(&head),
            pinned: false,
            last_used: now,
        });
        self.sort();
        self.prune();
    }

    pub fn set_max(&mut self, n: usize) {
        self.max_items = n;
        self.prune();
    }

    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// ピン留め以外を全削除し、削除件数を返す
    pub fn clear_unpinned(&mut self) -> usize {
        let n = self.items.iter().filter(|i| !i.pinned).count();
        let _ = self.conn.execute("DELETE FROM items WHERE pinned=0", []);
        self.items.retain(|i| i.pinned);
        n
    }

    fn prune(&mut self) {
        let unpinned = self.items.iter().filter(|i| !i.pinned).count();
        if unpinned <= self.max_items {
            return;
        }
        let excess = unpinned - self.max_items;
        let victims: Vec<i64> = self
            .items
            .iter()
            .rev()
            .filter(|i| !i.pinned)
            .take(excess)
            .map(|i| i.id)
            .collect();
        for id in victims {
            self.delete(id);
        }
    }

    pub fn touch(&mut self, id: i64) {
        let now = now_ms();
        let _ = self
            .conn
            .execute("UPDATE items SET last_used_at=?1 WHERE id=?2", params![now, id]);
        if let Some(it) = self.items.iter_mut().find(|i| i.id == id) {
            it.last_used = now;
        }
        self.sort();
    }

    pub fn toggle_pin(&mut self, id: i64) {
        if let Some(it) = self.items.iter_mut().find(|i| i.id == id) {
            it.pinned = !it.pinned;
            let _ = self.conn.execute(
                "UPDATE items SET pinned=?1 WHERE id=?2",
                params![it.pinned as i64, id],
            );
        }
        self.sort();
    }

    pub fn delete(&mut self, id: i64) {
        let _ = self.conn.execute("DELETE FROM items WHERE id=?1", params![id]);
        self.items.retain(|i| i.id != id);
    }

    pub fn get_text(&self, id: i64) -> Option<String> {
        self.conn
            .query_row("SELECT text FROM items WHERE id=?1", params![id], |r| r.get(0))
            .ok()
    }

    pub fn search(&self, query: &str) -> Vec<Row> {
        let ts = terms(query);
        self.items
            .iter()
            .filter(|i| ts.iter().all(|t| i.norm.contains(t.as_str())))
            .take(LIMIT)
            .map(|i| Row {
                id: i.id,
                label: String::new(),
                preview: i.preview.clone(),
                pinned: i.pinned,
                ts: i.last_used,
            })
            .collect()
    }
}
