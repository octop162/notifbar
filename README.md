# notifbar

Windowsの通知をリアルタイムで取得し、タイムライン上に表示・履歴をSQLiteに保存する常駐デスクトップアプリ。

## 概要

- WinRT の `UserNotificationListener` API でシステム通知をリアルタイム監視
- egui/eframe による GUI でタイムライン表示
- 通知履歴を SQLite に永続化
- システムトレイ常駐

## 必要環境

- Windows 10 / 11
- Rust (stable)
- Visual Studio Build Tools (MSVC)

## ローカルビルド・起動

```bash
# 開発用ビルド & 起動
cargo run

# リリースビルド
cargo build --release --target x86_64-pc-windows-msvc
```

ビルドした実行ファイルは `target/x86_64-pc-windows-msvc/release/notifbar.exe` に生成されます。

### 通知アクセス権限の設定

初回起動時は通知アクセスを手動で許可する必要があります。

1. `cargo run` でアプリを起動
2. Windows設定 → システム → 通知 を開く
3. 「通知アクセス」セクションで **notifbar** を許可する
4. アプリを再起動

## コード品質

### フォーマット

```bash
# フォーマット適用
cargo fmt

# チェックのみ（修正なし）
cargo fmt -- --check
```

### リンター (Clippy)

```bash
# 通常実行
cargo clippy

# 警告をエラーとして扱う（CI相当）
cargo clippy -- -D warnings
```

### テスト

```bash
cargo test
```

### pre-commit フック

[lefthook](https://github.com/evilmartians/lefthook) で管理しています。コミット前に `fmt --check` → `clippy` → `test` を自動実行します。

```bash
# フックのインストール
lefthook install
```

## リリース

`v*` タグを push すると GitHub Actions が自動でビルドし、GitHub Release に `notifbar.exe` をアップロードします。

```bash
git tag v0.1.0
git push origin v0.1.0
```
