// Windows notification listener (UserNotificationListener)
// WinRT の UserNotificationListener API を使い、Windows の通知をリアルタイムに取得するモジュール。

#![allow(dead_code)]

use crate::db;
use tokio::sync::mpsc;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::Management::{
    UserNotificationListener, UserNotificationListenerAccessStatus,
};
use windows::UI::Notifications::{
    NotificationKinds, UserNotification, UserNotificationChangedEventArgs,
    UserNotificationChangedKind,
};

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
/// tokio タスクとしてバックグラウンドで実行する想定。
pub async fn start_listener(
    tx: mpsc::UnboundedSender<NotificationEvent>,
) -> windows::core::Result<()> {
    let listener = UserNotificationListener::Current()?;

    // ユーザーに通知アクセス許可をリクエスト
    let status = listener.RequestAccessAsync()?.await?;
    if status != UserNotificationListenerAccessStatus::Allowed {
        return Err(windows::core::Error::new(
            windows::core::HRESULT(-1),
            "通知アクセスが許可されていません。Windows設定 > 通知 > 通知アクセス を確認してください。",
        ));
    }

    // 既存の通知を一括取得して送信
    {
        let existing = listener
            .GetNotificationsAsync(NotificationKinds::Toast)?
            .await?;
        let size = existing.Size()?;
        for i in 0..size {
            let user_notif = existing.GetAt(i)?;
            let id = user_notif.Id()?;
            if let Some(parsed) = parse_user_notification(&user_notif, id) {
                let _ = tx.send(NotificationEvent::Added(parsed));
            }
        }
    }

    // NotificationChanged イベントをサブスクライブ
    let tx_event = tx.clone();
    let listener_for_event = listener.clone();
    let _token = listener.NotificationChanged(&TypedEventHandler::new(
        move |_sender: &Option<UserNotificationListener>,
              args: &Option<UserNotificationChangedEventArgs>| {
            if let Some(args) = args {
                let kind = args.ChangeKind()?;
                let id = args.UserNotificationId()?;

                match kind {
                    UserNotificationChangedKind::Added => {
                        if let Ok(user_notif) = listener_for_event.GetNotification(id)
                            && let Some(parsed) = parse_user_notification(&user_notif, id)
                        {
                            let _ = tx_event.send(NotificationEvent::Added(parsed));
                        }
                    }
                    UserNotificationChangedKind::Removed => {
                        let _ = tx_event.send(NotificationEvent::Removed { win_id: id });
                    }
                    _ => {}
                }
            }
            Ok(())
        },
    ))?;

    // リスナーを維持するために無限ループで待機
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
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

    Some(db::Notification {
        id: None,
        win_id: Some(win_id as i64),
        app_name,
        title,
        body,
        arrived_at,
        removed_at: None,
        read: false,
    })
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

/// WinRT の DateTime.UniversalTime (100ナノ秒単位、1601-01-01起点) を ISO 8601 文字列に変換する。
fn winrt_datetime_to_iso8601(universal_time: i64) -> String {
    // 1601-01-01 から 1970-01-01 までの100ナノ秒間隔数
    const EPOCH_DIFF: i64 = 116_444_736_000_000_000;
    let unix_secs = (universal_time - EPOCH_DIFF) / 10_000_000;

    // Howard Hinnant の civil_from_days アルゴリズム
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

#[cfg(test)]
mod tests {
    use super::*;

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
