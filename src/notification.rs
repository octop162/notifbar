// Windows notification listener (UserNotificationListener)
// WinRT の UserNotificationListener API を使い、Windows の通知を取得するモジュール。
// 未パッケージアプリでは NotificationChanged イベントが使えないため、
// GetNotificationsAsync を定期ポーリングして差分検出する方式を採る。

#![allow(dead_code)]

use crate::db;
use std::collections::HashSet;
use std::sync::mpsc;
use windows::Foundation::{AsyncStatus, IAsyncOperation};
use windows::UI::Notifications::Management::{
    UserNotificationListener, UserNotificationListenerAccessStatus,
};
use windows::UI::Notifications::{NotificationKinds, UserNotification};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

/// ポーリング間隔（秒）
const POLL_INTERVAL_SECS: u64 = 2;

/// 通知イベントの種別。チャネル経由でUIスレッドに送信される。
#[derive(Debug)]
pub enum NotificationEvent {
    /// 新しい通知が追加された
    Added(db::Notification),
    /// 通知が削除された（win_id で識別）
    Removed {
        /// Windows通知ID
        win_id: u32,
    },
}

/// UserNotificationListener を起動し、通知イベントをチャネルに送信する。
/// STA スレッド上で呼び出す想定。ポーリングループに入り戻らない。
pub fn start_listener(tx: mpsc::Sender<NotificationEvent>) -> windows::core::Result<()> {
    let listener = UserNotificationListener::Current()?;

    // ユーザーに通知アクセス許可をリクエスト
    let status: UserNotificationListenerAccessStatus =
        wait_for_async(&listener.RequestAccessAsync()?)?;
    eprintln!("[notifbar] 通知アクセスステータス: {status:?}");
    if status != UserNotificationListenerAccessStatus::Allowed {
        return Err(windows::core::Error::new(
            windows::core::HRESULT(-1),
            "通知アクセスが許可されていません。Windows設定 > 通知 > 通知アクセス を確認してください。",
        ));
    }

    // 既知の通知IDセット（差分検出用）
    let mut known_ids: HashSet<u32> = HashSet::new();

    eprintln!("[notifbar] ポーリング開始（{POLL_INTERVAL_SECS}秒間隔）");

    loop {
        let notifications =
            wait_for_async(&listener.GetNotificationsAsync(NotificationKinds::Toast)?)?;
        let size = notifications.Size()?;

        let mut current_ids: HashSet<u32> = HashSet::with_capacity(size as usize);

        for i in 0..size {
            let user_notif = notifications.GetAt(i)?;
            let id = user_notif.Id()?;
            current_ids.insert(id);

            // 新規通知を検出
            if !known_ids.contains(&id)
                && let Some(parsed) = parse_user_notification(&user_notif, id)
            {
                let _ = tx.send(NotificationEvent::Added(parsed));
            }
        }

        // 削除された通知を検出
        for &id in known_ids.difference(&current_ids) {
            let _ = tx.send(NotificationEvent::Removed { win_id: id });
        }

        known_ids = current_ids;

        std::thread::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS));
    }
}

/// WinRT の IAsyncOperation を Win32 メッセージポンプで待機して結果を取得する。
fn wait_for_async<T: windows::core::RuntimeType>(
    op: &IAsyncOperation<T>,
) -> windows::core::Result<T> {
    while op.Status()? == AsyncStatus::Started {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).into() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    op.GetResults()
}

/// UserNotification からアプリ名・タイトル・本文・到着時刻をパースする。
/// パースに失敗した場合は None を返す。
fn parse_user_notification(user_notif: &UserNotification, win_id: u32) -> Option<db::Notification> {
    // アプリ名の取得
    let app_name = user_notif
        .AppInfo()
        .ok()
        .and_then(|info| info.DisplayInfo().ok())
        .and_then(|display| display.DisplayName().ok())
        .map(|name| name.to_string())
        .unwrap_or_else(|| "Unknown".to_string());

    // 到着時刻の取得
    let creation_time = user_notif.CreationTime().ok()?;
    let arrived_at = winrt_datetime_to_iso8601(creation_time.UniversalTime);

    // タイトル・本文の取得
    let (title, body) = extract_text(user_notif);

    // launch URL の取得
    let launch_url = extract_launch_url(user_notif);

    Some(db::Notification {
        id: None,
        win_id: Some(win_id as i64),
        app_name,
        title,
        body,
        launch_url,
        arrived_at,
        removed_at: None,
        read: false,
    })
}

/// トースト通知の Content XML から launch 属性値を取得する。
/// `UserNotification::Notification()` が返す型は `Notification` だが、
/// 実体は `ToastNotification` なので cast して `Content()` を呼ぶ。
fn extract_launch_url(user_notif: &UserNotification) -> Option<String> {
    use windows::UI::Notifications::ToastNotification;
    use windows::core::Interface;
    let notification = user_notif.Notification().ok()?;
    let toast = notification.cast::<ToastNotification>().ok()?;
    let xml = toast.Content().ok()?.GetXml().ok()?;
    extract_launch_url_from_xml(&xml.to_string())
}

/// XML文字列の <toast launch="..."> 属性値を解析して返す。
/// 属性が存在しない・空の場合は None を返す。
fn extract_launch_url_from_xml(xml: &str) -> Option<String> {
    let toast_start = xml.find("<toast")?;
    let tag_end = xml[toast_start..].find('>')?;
    let toast_tag = &xml[toast_start..toast_start + tag_end];

    let launch_pos = toast_tag.find("launch=\"")?;
    let after_launch = &toast_tag[launch_pos + 8..];
    let end = after_launch.find('"')?;
    let url = &after_launch[..end];

    if url.is_empty() {
        None
    } else {
        Some(url.to_string())
    }
}

/// 通知の Visual > Binding > TextElements からタイトルと本文を取得する。
fn extract_text(user_notif: &UserNotification) -> (Option<String>, Option<String>) {
    let texts = (|| -> windows::core::Result<_> {
        let visual = user_notif.Notification()?.Visual()?;
        let bindings = visual.Bindings()?;
        let binding = bindings.GetAt(0)?;
        binding.GetTextElements()
    })();

    let Ok(texts) = texts else {
        return (None, None);
    };

    let title = texts
        .GetAt(0)
        .ok()
        .and_then(|t| t.Text().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());

    let body = texts
        .GetAt(1)
        .ok()
        .and_then(|t| t.Text().ok())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());

    (title, body)
}

/// 現在時刻を ISO 8601 文字列（UTC）で返す。
pub fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    unix_secs_to_iso8601(secs)
}

/// Unix秒（UTC）を ISO 8601 文字列に変換する。
fn unix_secs_to_iso8601(unix_secs: i64) -> String {
    let z = unix_secs.div_euclid(86400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    let rem = unix_secs.rem_euclid(86400);
    let h = rem / 3600;
    let min = (rem % 3600) / 60;
    let s = rem % 60;

    format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{s:02}")
}

/// WinRT の DateTime.UniversalTime (100ナノ秒単位、1601-01-01起点) を ISO 8601 文字列に変換する。
fn winrt_datetime_to_iso8601(universal_time: i64) -> String {
    // 1601-01-01 から 1970-01-01 までの100ナノ秒間隔数
    const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
    let unix_secs = (universal_time - EPOCH_DIFF) / 10_000_000;
    unix_secs_to_iso8601(unix_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_launch_url_from_xml_with_url() {
        let xml = r#"<toast launch="https://example.com/path?q=1"><visual></visual></toast>"#;
        assert_eq!(
            extract_launch_url_from_xml(xml),
            Some("https://example.com/path?q=1".to_string())
        );
    }

    #[test]
    fn test_extract_launch_url_from_xml_no_launch() {
        let xml = r#"<toast><visual></visual></toast>"#;
        assert_eq!(extract_launch_url_from_xml(xml), None);
    }

    #[test]
    fn test_extract_launch_url_from_xml_empty_launch() {
        let xml = r#"<toast launch=""><visual></visual></toast>"#;
        assert_eq!(extract_launch_url_from_xml(xml), None);
    }

    #[test]
    fn test_winrt_datetime_to_iso8601() {
        // 2026-03-15T00:00:00 UTC
        let unix_ts: i64 = 1_773_532_800;
        let universal_time = unix_ts * 10_000_000 + 116_444_736_000_000_000;
        assert_eq!(
            winrt_datetime_to_iso8601(universal_time),
            "2026-03-15T00:00:00"
        );
    }

    #[test]
    fn test_winrt_datetime_epoch() {
        // Unix epoch: 1970-01-01T00:00:00
        let universal_time: i64 = 116_444_736_000_000_000;
        assert_eq!(
            winrt_datetime_to_iso8601(universal_time),
            "1970-01-01T00:00:00"
        );
    }

    #[test]
    fn test_winrt_datetime_with_time() {
        // 2026-03-15T14:30:45 UTC
        let unix_ts: i64 = 1_773_532_800 + 14 * 3600 + 30 * 60 + 45;
        let universal_time = unix_ts * 10_000_000 + 116_444_736_000_000_000;
        assert_eq!(
            winrt_datetime_to_iso8601(universal_time),
            "2026-03-15T14:30:45"
        );
    }
}
