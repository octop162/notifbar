# notifbar

Windowsの通知をリアルタイムで取得し、タイムライン上に表示・履歴をSQLiteに保存する常駐デスクトップアプリ。

## リポジトリ

https://github.com/octop162/notifbar

## 開発フロー

- タスク管理は **GitHub Issues** で行う
- 作業開始前に `gh issue list` で未着手のissueを確認する
- issueに対応するブランチを切って作業し、PRでマージする

## コーディング規約

- struct・フィールド・pub メソッドには日本語でコメントを付ける

## 技術スタック

- **Rust**
- **egui / eframe** - Immediate Mode GUI
- **windows-rs** (`windows` crate) - WinRT API呼び出し
- **rusqlite** (bundled) - SQLite保存
- **tray-icon** - システムトレイ常駐
- **tokio** - 非同期ランタイム（バックグラウンド通知監視）

## 通知取得方式

WinRT の `UserNotificationListener` APIを使用してリアルタイムに通知を取得する。

```
windows::UI::Notifications::Management::UserNotificationListener
```

### 主要API

| メソッド | 用途 |
|---------|------|
| `UserNotificationListener::Current()` | リスナーのシングルトン取得 |
| `RequestAccessAsync()` | ユーザーに通知アクセス許可をリクエスト |
| `NotificationChanged` イベント | 通知の追加・削除をリアルタイム検知 |
| `GetNotificationsAsync(NotificationKinds)` | 既存通知の一括取得 |
| `GetNotification(id)` | 個別通知の取得 |

### UserNotification から取得できる情報

- `AppInfo` - 通知元アプリの情報（DisplayInfo.DisplayName等）
- `Id` - Windows通知ID
- `Notification` - トースト通知の内容（XML Payload → タイトル・本文）
- `CreationTime` - 通知の到着時刻

### 権限（Notification access）

UserNotificationListenerを使うにはアプリに通知アクセス権限が必要。

- **開発時**: Windows設定 → システム → 通知 → 通知アクセス で手動許可
- **配布時**: MSIXパッケージで `userNotificationListener` capability を宣言

## アーキテクチャ

```
┌─────────────────────────┐
│  egui/eframe UI         │  メインスレッド
│  - タイムライン表示      │
│  - フィルタ・検索        │
│  - システムトレイ        │
└───────────┬─────────────┘
            │ mpsc::channel
┌───────────▼─────────────┐
│  Tokio バックグラウンド  │  別スレッド
│  - UserNotificationListener
│  - NotificationChanged監視
│  - DB書き込み
└───────────┬─────────────┘
            │
       ┌────▼────┐
       │ SQLite  │  notifications.db
       └─────────┘
```

### データフロー

1. バックグラウンドスレッドで `NotificationChanged` イベントを受信
2. 通知内容（タイトル、本文、アプリ名、到着時刻）をパース
3. rusqliteでアプリ独自SQLiteに保存
4. `mpsc::channel` でUIスレッドに送信
5. `ctx.request_repaint()` でegui再描画

## SQLiteスキーマ

```sql
CREATE TABLE notifications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    win_id INTEGER UNIQUE,        -- Windows通知ID
    app_name TEXT NOT NULL,
    title TEXT,
    body TEXT,
    arrived_at DATETIME NOT NULL,
    removed_at DATETIME,
    read INTEGER DEFAULT 0
);
```

## 主要依存crate

```toml
[dependencies]
windows = { version = "0.59", features = [
    "UI_Notifications_Management",
    "UI_Notifications",
    "Foundation",
] }
tokio = { version = "1", features = ["full"] }
eframe = "0.31"
rusqlite = { version = "0.31", features = ["bundled"] }
tray-icon = "0.19"
```

## 開発ツール

### フォーマッタ・リンター

- **rustfmt** - `cargo fmt` でコードフォーマット（デフォルト設定）
- **clippy** - `cargo clippy` で静的解析（デフォルト設定）

### テスト

- **cargo test** - 標準テストフレームワーク。`#[test]` アトリビュートでユニットテスト

### pre-commit フック (lefthook)

`lefthook.yml` で管理。コミット前に以下を順次実行:

1. `cargo fmt -- --check`
2. `cargo clippy -- -D warnings`
3. `cargo test`

hookのインストール: `lefthook install`

### コマンド一覧

| コマンド | 用途 |
|---------|------|
| `cargo run` | アプリ起動 |
| `cargo fmt` | コードフォーマット |
| `cargo clippy` | リンター実行 |
| `cargo test` | テスト実行 |
| `cargo fmt -- --check` | フォーマットチェック（修正なし） |
| `cargo clippy -- -D warnings` | 警告をエラーとして扱う |

## ビルド

Windows (x86_64-pc-windows-msvc) ターゲットでビルドする。

## 開発時のNotification access設定

1. `cargo run` でアプリを起動
2. Windows設定 → システム → 通知 を開く
3. 「通知アクセス」セクションでアプリを許可する
4. アプリを再起動

## 既知のハマりどころ

### eframe: ウィンドウ非表示中は update() が呼ばれない

`ViewportCommand::Visible(false)` でウィンドウを非表示にすると、eframe が `update()` の
呼び出しを停止する。`request_repaint_after()` も機能しなくなる。

**影響:** `update()` 内でトレイイベントをポーリングする実装だと、ウィンドウ非表示後に
トレイメニュー・クリックが完全に無視される。

**対策:** トレイイベントは専用バックグラウンドスレッドで処理し、イベント発生時に
`ctx.request_repaint()` でUIスレッドを起こす。`egui::Context` は `Clone + Send + Sync`
なのでスレッド間で安全に共有できる。詳細は Issue #16 を参照。
