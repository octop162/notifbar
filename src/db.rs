// SQLite database layer (rusqlite)

#![allow(dead_code)]

use rusqlite::{Connection, Result, params};

pub struct Database {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: Option<i64>,
    pub win_id: Option<i64>,
    pub app_name: String,
    pub title: Option<String>,
    pub body: Option<String>,
    pub arrived_at: String,
    pub removed_at: Option<String>,
    pub read: bool,
}

impl Database {
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS notifications (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                win_id INTEGER UNIQUE,
                app_name TEXT NOT NULL,
                title TEXT,
                body TEXT,
                arrived_at DATETIME NOT NULL,
                removed_at DATETIME,
                read INTEGER DEFAULT 0
            );",
        )?;
        Ok(())
    }

    pub fn insert(&self, n: &Notification) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO notifications (win_id, app_name, title, body, arrived_at, removed_at, read)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![n.win_id, n.app_name, n.title, n.body, n.arrived_at, n.removed_at, n.read as i64],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn query_all(&self) -> Result<Vec<Notification>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, win_id, app_name, title, body, arrived_at, removed_at, read
             FROM notifications ORDER BY arrived_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Notification {
                id: row.get(0)?,
                win_id: row.get(1)?,
                app_name: row.get(2)?,
                title: row.get(3)?,
                body: row.get(4)?,
                arrived_at: row.get(5)?,
                removed_at: row.get(6)?,
                read: row.get::<_, i64>(7)? != 0,
            })
        })?;
        rows.collect()
    }

    pub fn mark_read(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE notifications SET read = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

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
    fn test_insert_ignore_duplicate_win_id() {
        let db = in_memory_db();
        let n = Notification {
            id: None,
            win_id: Some(4001),
            app_name: "App".to_string(),
            title: None,
            body: None,
            arrived_at: "2026-03-15T00:00:00".to_string(),
            removed_at: None,
            read: false,
        };
        db.insert(&n).unwrap();
        db.insert(&n).unwrap(); // duplicate, should be ignored
        let all = db.query_all().unwrap();
        assert_eq!(all.len(), 1);
    }
}
