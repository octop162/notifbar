#![windows_subsystem = "windows"]

mod db;
mod notification;
mod ui;

use eframe::egui;
use std::sync::mpsc;
use tray_icon::TrayIconBuilder;
use tray_icon::menu::{Menu, MenuItem};
use ui::NotifBarApp;

/// トレイアイコン用の16x16 RGBAピクセルデータを生成する。
/// 境界線を明るい青、内側を濃紺にしたシンプルなアイコン。
fn create_tray_icon() -> tray_icon::Icon {
    let size = 16u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let is_border = x == 0 || x == size - 1 || y == 0 || y == size - 1;
            if is_border {
                rgba.extend_from_slice(&[0x50, 0x8C, 0xDC, 0xFF]);
            } else {
                rgba.extend_from_slice(&[0x19, 0x26, 0x3A, 0xFF]);
            }
        }
    }
    tray_icon::Icon::from_rgba(rgba, size, size).expect("トレイアイコン作成失敗")
}

/// Yu Gothic（Windowsシステムフォント）を egui に登録して日本語を表示できるようにする。
fn setup_japanese_font(ctx: &egui::Context) {
    let font_path = "C:\\Windows\\Fonts\\YuGothR.ttc";
    let Ok(font_data) = std::fs::read(font_path) else {
        eprintln!("フォント読み込み失敗: {font_path}");
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "yugothic".to_owned(),
        egui::FontData::from_owned(font_data).into(),
    );
    // 全ファミリーの先頭に挿入することで日本語グリフが優先される
    for family in fonts.families.values_mut() {
        family.insert(0, "yugothic".to_owned());
    }
    ctx.set_fonts(fonts);
}

fn main() {
    // DBを開いて起動時の履歴を読み込む
    let db_path = "notifications.db";
    let db = db::Database::open(db_path).expect("DBオープン失敗");
    let initial_notifications = db.query_all().unwrap_or_default();

    // リスナースレッド → UI スレッド 通信チャネル
    let (tx, rx) = mpsc::channel::<notification::NotificationEvent>();

    // バックグラウンドスレッドで通知のポーリングを開始する
    std::thread::spawn(move || {
        if let Err(e) = notification::start_listener(tx) {
            eprintln!("通知リスナーエラー: {e}");
        }
    });

    // トレイ右クリックメニューを作成する
    let tray_menu = Menu::new();
    let show_hide_item = MenuItem::new("表示/非表示", true, None);
    let exit_item = MenuItem::new("終了", true, None);
    tray_menu
        .append(&show_hide_item)
        .expect("メニュー項目追加失敗");
    tray_menu.append(&exit_item).expect("メニュー項目追加失敗");

    let show_hide_id = show_hide_item.id().clone();
    let exit_id = exit_item.id().clone();

    // トレイアイコンを作成してアプリ終了まで保持する
    let _tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_icon(create_tray_icon())
        .with_tooltip("notifbar")
        .build()
        .expect("トレイアイコン起動失敗");

    // eframe アプリをメインスレッドで起動する
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("notifbar")
            .with_inner_size([420.0, 650.0]),
        ..Default::default()
    };

    eframe::run_native(
        "notifbar",
        options,
        Box::new(|cc| {
            setup_japanese_font(&cc.egui_ctx);
            Ok(Box::new(NotifBarApp::new(
                initial_notifications,
                rx,
                db,
                show_hide_id,
                exit_id,
            )))
        }),
    )
    .expect("eframeアプリ起動失敗");
}
