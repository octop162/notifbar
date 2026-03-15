mod db;
mod notification;
mod ui;

use eframe::egui;
use std::sync::mpsc;
use ui::NotifBarApp;

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
            Ok(Box::new(NotifBarApp::new(initial_notifications, rx, db)))
        }),
    )
    .expect("eframeアプリ起動失敗");
}
