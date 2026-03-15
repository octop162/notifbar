// Windows notification listener (UserNotificationListener)
// WinRT の UserNotificationListener API を使い、Windows の通知を取得するモジュール。
// 未パッケージアプリでは NotificationChanged イベントが使えないため、
// GetNotificationsAsync を定期ポーリングして差分検出する方式を採る。

#![allow(dead_code)]

use crate::db;
use std::collections::HashSet;
use std::sync::mpsc;
use windows::Foundation::{AsyncStatus, IAsyncOperation};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
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
    Added(Box<db::Notification>),
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
                let _ = tx.send(NotificationEvent::Added(Box::new(parsed)));
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

    let icon_bytes = fetch_icon_bytes(user_notif);

    Some(db::Notification {
        id: None,
        win_id: Some(win_id as i64),
        app_name,
        title,
        body,
        launch_url,
        icon_bytes,
        arrived_at,
        removed_at: None,
        read: false,
    })
}

/// AppInfo から GetLogo() でアプリアイコン画像のバイト列を取得する。
/// UWP アプリでは GetLogo() を使い、Win32 アプリでは AUMID 経由で Shell API にフォールバックする。
fn fetch_icon_bytes(user_notif: &UserNotification) -> Option<Vec<u8>> {
    use windows::Foundation::Size;

    let app_info = user_notif.AppInfo().ok()?;
    let display_info = app_info.DisplayInfo().ok()?;

    // 方法1: GetLogo() (UWP・パッケージアプリ向け)
    if let Ok(logo_ref) = display_info.GetLogo(Size {
        Width: 32.0,
        Height: 32.0,
    }) && let Some(bytes) = read_stream_ref(logo_ref.into())
    {
        return Some(bytes);
    }

    // 方法2: AUMID から Shell API でアイコン取得 (Win32 アプリ向け)
    let aumid = app_info.AppUserModelId().ok()?.to_string();
    if !aumid.is_empty() {
        return fetch_icon_from_shell(&aumid);
    }

    None
}

/// IRandomAccessStreamReference を開いてバイト列として読み取る。
fn read_stream_ref(logo_ref: IRandomAccessStreamReference) -> Option<Vec<u8>> {
    let stream = wait_for_async(&logo_ref.OpenReadAsync().ok()?).ok()?;
    let size = stream.Size().ok()? as u32;
    if size == 0 {
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    wait_for_async(&reader.LoadAsync(size).ok()?).ok()?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    Some(bytes)
}

/// AUMID から Shell の IShellItemImageFactory::GetImage でアイコンを取得し PNG バイト列で返す。
/// Win32 アプリ向けフォールバック。
fn fetch_icon_from_shell(aumid: &str) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF};

    let path = format!("shell:AppsFolder\\{aumid}");
    let path_wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let factory: IShellItemImageFactory =
            match SHCreateItemFromParsingName(windows::core::PCWSTR(path_wide.as_ptr()), None) {
                Ok(f) => f,
                Err(_) => return None,
            };

        let hbm = match factory.GetImage(SIZE { cx: 32, cy: 32 }, SIIGBF(0)) {
            Ok(h) => h,
            Err(_) => return None,
        };

        hbitmap_to_png(hbm, 32, 32)
    }
}

/// HBITMAP を RGBA ピクセル列に変換して PNG バイト列として返す。
fn hbitmap_to_png(
    hbm: windows::Win32::Graphics::Gdi::HBITMAP,
    width: i32,
    height: i32,
) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::*;

    let rgba = unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return None;
        }
        let prev = SelectObject(dc, HGDIOBJ(hbm.0));

        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = width;
        bmi.bmiHeader.biHeight = -height; // 負値でトップダウン
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = 0; // BI_RGB

        let mut bgra = vec![0u8; (width * height * 4) as usize];
        let lines = GetDIBits(
            dc,
            hbm,
            0,
            height as u32,
            Some(bgra.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        );

        SelectObject(dc, prev);
        let _ = DeleteDC(dc);
        let _ = DeleteObject(HGDIOBJ(hbm.0));

        if lines == 0 {
            return None;
        }

        // Windows は BGRA 形式なので RGBA に変換する
        for chunk in bgra.chunks_exact_mut(4) {
            chunk.swap(0, 2);
        }
        bgra
    };

    let img = image::RgbaImage::from_raw(width as u32, height as u32, rgba)?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .ok()?;
    Some(png)
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

/// ISO 8601 UTC文字列（"YYYY-MM-DDTHH:MM:SS"）をUnix秒に変換する。
fn parse_iso8601(s: &str) -> Option<i64> {
    if s.len() < 19 {
        return None;
    }
    let y: i64 = s[0..4].parse().ok()?;
    let m: i64 = s[5..7].parse().ok()?;
    let d: i64 = s[8..10].parse().ok()?;
    let h: i64 = s[11..13].parse().ok()?;
    let min: i64 = s[14..16].parse().ok()?;
    let sec: i64 = s[17..19].parse().ok()?;

    // Howard Hinnant のアルゴリズムで日付をUnix日数に変換する
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = (y - era * 400) as u32;
    let m_adj = if m > 2 { m as u32 - 3 } else { m as u32 + 9 };
    let doy = (153 * m_adj + 2) / 5 + d as u32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;

    Some(days * 86400 + h * 3600 + min * 60 + sec)
}

/// ISO 8601 UTC文字列をJST（UTC+9）のISO 8601文字列に変換する。
pub fn iso8601_utc_to_jst(s: &str) -> String {
    const JST_OFFSET: i64 = 9 * 3600;
    match parse_iso8601(s) {
        Some(unix_secs) => unix_secs_to_iso8601(unix_secs + JST_OFFSET),
        None => s.to_string(),
    }
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

    #[test]
    fn test_iso8601_utc_to_jst_basic() {
        // 2026-03-15T00:00:00 UTC → 2026-03-15T09:00:00 JST
        assert_eq!(
            iso8601_utc_to_jst("2026-03-15T00:00:00"),
            "2026-03-15T09:00:00"
        );
    }

    #[test]
    fn test_iso8601_utc_to_jst_date_rollover() {
        // 2026-03-15T23:00:00 UTC → 2026-03-16T08:00:00 JST
        assert_eq!(
            iso8601_utc_to_jst("2026-03-15T23:00:00"),
            "2026-03-16T08:00:00"
        );
    }

    #[test]
    fn test_iso8601_utc_to_jst_invalid() {
        // 不正な文字列はそのまま返す
        assert_eq!(iso8601_utc_to_jst("invalid"), "invalid");
    }
}
