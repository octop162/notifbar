// egui/eframe UI layer
// メインウィンドウのタイムライン表示を担うモジュール。

use crate::db::{Database, Notification};
use crate::notification::NotificationEvent;
use eframe::egui;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use tray_icon::menu::{MenuEvent, MenuId};
use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

/// バックグラウンドのトレイイベントスレッドからUIスレッドへ送るコマンド。
enum TrayCommand {
    /// ウィンドウを非表示にする（update()内で ViewportCommand を発行するため）
    Hide,
}

/// タイムラインUIアプリ。eframe::Appを実装し、通知履歴をスクロール可能なリストで表示する。
pub struct NotifBarApp {
    /// 表示中の通知リスト（新しい順）
    notifications: Vec<Notification>,
    /// バックグラウンドスレッドからの通知受信チャネル
    receiver: mpsc::Receiver<NotificationEvent>,
    /// DB接続（通知の永続化に使用）
    db: Database,
    /// トレイメニューの「表示/非表示」アイテムID
    show_hide_id: MenuId,
    /// トレイメニューの「終了」アイテムID
    exit_id: MenuId,
    /// ウィンドウの現在の表示状態（バックグラウンドスレッドと共有）
    window_visible: Arc<AtomicBool>,
    /// トレイイベントスレッドへのコマンド送信端
    tray_tx: mpsc::SyncSender<TrayCommand>,
    /// UIスレッド側のコマンド受信端
    tray_rx: mpsc::Receiver<TrayCommand>,
    /// トレイポーリングスレッドが起動済みかどうか
    tray_thread_started: bool,
}

impl NotifBarApp {
    /// 初期通知リスト・チャネルレシーバー・DB接続・トレイメニューIDを受け取り、アプリを生成する。
    pub fn new(
        initial: Vec<Notification>,
        receiver: mpsc::Receiver<NotificationEvent>,
        db: Database,
        show_hide_id: MenuId,
        exit_id: MenuId,
    ) -> Self {
        let (tray_tx, tray_rx) = mpsc::sync_channel(8);
        Self {
            notifications: initial,
            receiver,
            db,
            show_hide_id,
            exit_id,
            window_visible: Arc::new(AtomicBool::new(true)),
            tray_tx,
            tray_rx,
            tray_thread_started: false,
        }
    }

    /// バックグラウンドでトレイイベントをポーリングするスレッドを起動する。
    ///
    /// ウィンドウ表示中のトグル操作は `TrayCommand::Hide` を送り `ctx.request_repaint()` で
    /// UIスレッドに委ねる。ウィンドウ非表示中は eframe の `update()` が停止するため、
    /// 表示だけは Win32 の `ShowWindow` / `SetForegroundWindow` を直接呼び出す。
    fn spawn_tray_thread(
        ctx: egui::Context,
        tray_tx: mpsc::SyncSender<TrayCommand>,
        show_hide_id: MenuId,
        exit_id: MenuId,
        window_visible: Arc<AtomicBool>,
    ) {
        std::thread::spawn(move || {
            loop {
                while let Ok(event) = MenuEvent::receiver().try_recv() {
                    if event.id == show_hide_id {
                        handle_toggle(&window_visible, &tray_tx, &ctx);
                    } else if event.id == exit_id {
                        std::process::exit(0);
                    }
                }
                while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        handle_toggle(&window_visible, &tray_tx, &ctx);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
    }
}

/// 表示/非表示のトグル処理。
/// 表示中なら TrayCommand::Hide を UI スレッドに送る。
/// 非表示中なら Win32 で直接ウィンドウを表示する（update() が止まっているため）。
fn handle_toggle(
    window_visible: &Arc<AtomicBool>,
    tray_tx: &mpsc::SyncSender<TrayCommand>,
    ctx: &egui::Context,
) {
    if window_visible.load(Ordering::Relaxed) {
        tray_tx.send(TrayCommand::Hide).ok();
        ctx.request_repaint();
    } else {
        // update() が停止しているため ViewportCommand は使えない。Win32 で直接表示する。
        show_window_win32();
        window_visible.store(true, Ordering::Relaxed);
    }
}

/// Win32 API でウィンドウタイトルからHWNDを探し、直接表示・フォーカスする。
/// eframe の update() を介さないためウィンドウ非表示中でも即座に実行できる。
fn show_window_win32() {
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SW_SHOW, SetForegroundWindow, ShowWindow,
    };
    use windows::core::w;
    if let Ok(hwnd) = unsafe { FindWindowW(None, w!("notifbar")) } {
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

impl eframe::App for NotifBarApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ×ボタンによるクローズリクエストをキャンセルしてトレイに最小化する
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.window_visible.store(false, Ordering::Relaxed);
        }

        // 初回 update() 時にトレイポーリングスレッドを起動する
        if !self.tray_thread_started {
            Self::spawn_tray_thread(
                ctx.clone(),
                self.tray_tx.clone(),
                self.show_hide_id.clone(),
                self.exit_id.clone(),
                Arc::clone(&self.window_visible),
            );
            self.tray_thread_started = true;
        }

        // トレイスレッドからのコマンドを処理する
        while let Ok(cmd) = self.tray_rx.try_recv() {
            match cmd {
                TrayCommand::Hide => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                    self.window_visible.store(false, Ordering::Relaxed);
                }
            }
        }

        // チャネルから新着・削除イベントをすべて受け取って状態に反映する
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                NotificationEvent::Added(n) => {
                    // DB に書き込み
                    if let Err(e) = self.db.insert(&n) {
                        eprintln!("DB挿入エラー: {e}");
                    }
                    // win_id が重複する場合はスキップ
                    let already_exists = n
                        .win_id
                        .map(|id| self.notifications.iter().any(|e| e.win_id == Some(id)))
                        .unwrap_or(false);
                    if !already_exists {
                        self.notifications.insert(0, n);
                    }
                }
                NotificationEvent::Removed { win_id } => {
                    let now = crate::notification::now_iso8601();
                    // DB に反映
                    if let Err(e) = self.db.set_removed_at(win_id as i64, &now) {
                        eprintln!("DB更新エラー: {e}");
                    }
                    if let Some(pos) = self
                        .notifications
                        .iter()
                        .position(|n| n.win_id == Some(win_id as i64))
                    {
                        self.notifications[pos].removed_at = Some(now);
                    }
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            if self.notifications.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.weak("通知なし");
                });
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for n in &self.notifications {
                        render_notification_card(ui, n);
                        ui.add_space(4.0);
                    }
                });
            }
        });

        // バックグラウンドから通知が届いたとき即座に再描画するためポーリングする
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
}

/// 通知1件分のカードを描画する。launch_url がある場合はクリックでURLを開く。
fn render_notification_card(ui: &mut egui::Ui, n: &Notification) {
    let is_removed = n.removed_at.is_some();

    let bg_color = if is_removed {
        egui::Color32::from_gray(40)
    } else {
        egui::Color32::from_rgb(25, 38, 58)
    };

    let border_color = if is_removed {
        egui::Color32::from_gray(60)
    } else {
        egui::Color32::from_rgb(80, 140, 220)
    };

    egui::Frame::new()
        .inner_margin(egui::Margin::same(8))
        .corner_radius(egui::CornerRadius::same(4))
        .fill(bg_color)
        .stroke(egui::Stroke::new(1.0, border_color))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());

            // ヘッダ行: アプリ名 + 未読マーク + 到着時刻
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(130, 190, 255), &n.app_name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let time_str = n.arrived_at.get(11..19).unwrap_or(&n.arrived_at);
                    ui.weak(time_str);
                    if is_removed {
                        ui.weak("[削除済]");
                    }
                });
            });

            // タイトル
            if let Some(title) = &n.title {
                ui.label(egui::RichText::new(title).strong());
            }

            // 本文
            if let Some(body) = &n.body {
                ui.label(egui::RichText::new(body).color(egui::Color32::from_gray(200)));
            }
        });
}
