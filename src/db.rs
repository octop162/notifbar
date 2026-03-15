// SQLite database layer (rusqlite)
// 通知履歴の永続化を担うモジュール。
// Database 構造体が接続を保持し、CRUD操作を提供する。

#![allow(dead_code)]

use rusqlite::{Connection, Result, params};

/// SQLite接続ラッパー。1インスタンスにつき1ファイルを管理する。
pub struct Database {
    conn: Connection,
}

/// notificationsテーブルの1行に対応するデータ構造。
#[derive(Debug, Clone)]
pub struct Notification {
    /// アプリ独自の自動採番ID（INSERT後に確定）
    pub id: Option<i64>,
    /// Windows通知ID。UNIQUE制約によりシステム側の重複を防ぐ。
    pub win_id: Option<i64>,
    /// 通知元アプリの表示名（AppInfo.DisplayInfo.DisplayName）
    pub app_name: String,
    /// トースト通知のタイトル
    pub title: Option<String>,
    /// トースト通知の本文
    pub body: Option<String>,
    /// トースト通知の起動URL（<toast launch="..."> 属性）
    pub launch_url: Option<String>,
    /// 通知の到着時刻（ISO 8601 文字列: "YYYY-MM-DDTHH:MM:SS"）
    pub arrived_at: String,
    /// 通知が削除された時刻（None = まだアクティブ）
    pub removed_at: Option<String>,
    /// 既読フラグ（false = 未読）
    pub read: bool,
}

impl Database {
    /// 指定パスのSQLiteファイルを開き、スキーマを初期化して返す。
    /// ファイルが存在しない場合は新規作成される。
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    /// テーブルが存在しなければ作成する（冪等）。
    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS notifications (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                win_id INTEGER UNIQUE,        -- Windows通知ID（重複防止）
                app_name TEXT NOT NULL,
                title TEXT,
                body TEXT,
                launch_url TEXT,              -- toast launch 属性
                arrived_at DATETIME NOT NULL,
                removed_at DATETIME,
                read INTEGER DEFAULT 0        -- 0: 未読, 1: 既読
            );",
        )?;
        Ok(())
    }

    /// 通知を1件挿入する。
    /// win_id が重複する場合は INSERT OR IGNORE により無視される。
    /// 戻り値は新規挿入行の rowid（重複時は 0）。
    pub fn insert(&self, n: &Notification) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO notifications (win_id, app_name, title, body, launch_url, arrived_at, removed_at, read)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![n.win_id, n.app_name, n.title, n.body, n.launch_url, n.arrived_at, n.removed_at, n.read as i64],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 全通知を到着時刻の降順（新しい順）で返す。
    pub fn query_all(&self) -> Result<Vec<Notification>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, win_id, app_name, title, body, launch_url, arrived_at, removed_at, read
             FROM notifications ORDER BY arrived_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Notification {
                id: row.get(0)?,
                win_id: row.get(1)?,
                app_name: row.get(2)?,
                title: row.get(3)?,
                body: row.get(4)?,
                launch_url: row.get(5)?,
                arrived_at: row.get(6)?,
                removed_at: row.get(7)?,
                // SQLite に boolean 型はないため INTEGER (0/1) で保存し変換する
                read: row.get::<_, i64>(8)? != 0,
            })
        })?;
        rows.collect()
    }

    /// アプリ独自 id で指定した通知を既読にする。
    /// UI側でユーザーが通知をクリックした際に呼び出す想定。
    pub fn mark_read(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE notifications SET read = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    /// notifications テーブルの全行を削除する。
    /// 設定画面の「DB クリア」ボタンから呼び出す。
    pub fn delete_all(&self) -> Result<()> {
        self.conn.execute_batch("DELETE FROM notifications;")?;
        Ok(())
    }

    /// Windows通知ID (win_id) に対応する行の removed_at を更新する。
    /// NotificationChanged イベントで通知削除を検知したときに呼び出す。
    pub fn set_removed_at(&self, win_id: i64, removed_at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE notifications SET removed_at = ?1 WHERE win_id = ?2",
            params![removed_at, win_id],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト用のインメモリDBを生成する（ファイルに書き出さない）。
    fn in_memory_db() -> Database {
        let conn = Connection::open_in_memory().unwrap();
        let db = Database { conn };
        db.migrate().unwrap();
        db
    }

    #[test]
    fn test_insert_and_query() {
        let db = in_memory_db();
        let n = Notification {
            id: None,
            win_id: Some(1001),
            app_name: "TestApp".to_string(),
            title: Some("Hello".to_string()),
            body: Some("World".to_string()),
            launch_url: None,
            arrived_at: "2026-03-15T00:00:00".to_string(),
            removed_at: None,
            read: false,
        };
        db.insert(&n).unwrap();
        let all = db.query_all().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].app_name, "TestApp");
        assert_eq!(all[0].title.as_deref(), Some("Hello"));
        assert!(!all[0].read);
    }

    #[test]
    fn test_mark_read() {
        let db = in_memory_db();
        let n = Notification {
            id: None,
            win_id: Some(2001),
            app_name: "App".to_string(),
            title: None,
            body: None,
            launch_url: None,
            arrived_at: "2026-03-15T00:00:00".to_string(),
            removed_at: None,
            read: false,
        };
        db.insert(&n).unwrap();
        let all = db.query_all().unwrap();
        let id = all[0].id.unwrap();
        db.mark_read(id).unwrap();
        let all = db.query_all().unwrap();
        assert!(all[0].read);
    }

    #[test]
    fn test_set_removed_at() {
        let db = in_memory_db();
        let n = Notification {
            id: None,
            win_id: Some(3001),
            app_name: "App".to_string(),
            title: None,
            body: None,
            launch_url: None,
            arrived_at: "2026-03-15T00:00:00".to_string(),
            removed_at: None,
            read: false,
        };
        db.insert(&n).unwrap();
        db.set_removed_at(3001, "2026-03-15T01:00:00").unwrap();
        let all = db.query_all().unwrap();
        assert_eq!(all[0].removed_at.as_deref(), Some("2026-03-15T01:00:00"));
    }

    #[test]
    fn test_delete_all() {
        let db = in_memory_db();
        for win_id in [5001i64, 5002, 5003] {
            db.insert(&Notification {
                id: None,
                win_id: Some(win_id),
                app_name: "App".to_string(),
                title: None,
                body: None,
                launch_url: None,
                arrived_at: "2026-03-15T00:00:00".to_string(),
                removed_at: None,
                read: false,
            })
            .unwrap();
        }
        assert_eq!(db.query_all().unwrap().len(), 3);
        db.delete_all().unwrap();
        assert!(db.query_all().unwrap().is_empty());
    }

    #[test]
    fn test_insert_ignore_duplicate_win_id() {
        let db = in_memory_db();
        let n = Notification {
            id: None,
            win_id: Some(4001),
            app_name: "App".to_string(),
            title: None,
            body: None,
            launch_url: None,
            arrived_at: "2026-03-15T00:00:00".to_string(),
            removed_at: None,
            read: false,
        };
        db.insert(&n).unwrap();
        db.insert(&n).unwrap(); // win_id重複 → INSERT OR IGNORE で無視される
        let all = db.query_all().unwrap();
        assert_eq!(all.len(), 1);
    }
}
