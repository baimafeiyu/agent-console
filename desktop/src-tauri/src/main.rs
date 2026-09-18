#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod server;

use tauri::Manager;

fn main() {
    // 先起 HTTP 服务（前端 webview 要访问 http://127.0.0.1:8766）
    if let Err(e) = server::start() {
        eprintln!("HTTP 服务启动失败: {}", e);
        // 继续启动，webview 会显示连接失败，但托盘/窗口仍可用
    }

    tauri::Builder::default()
        .setup(|app| {
            server::set_app_handle(app.handle().clone());
            use tauri::menu::{Menu, MenuItem};
            use tauri::tray::TrayIconBuilder;

            let show = MenuItem::with_id(app, "show", "显示指挥台", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            let mut tray_builder = TrayIconBuilder::with_id("main-tray")
                .tooltip("智能体指挥台")
                .menu(&menu)
                .show_menu_on_left_click(false);
            if let Some(icon) = app.default_window_icon() {
                tray_builder = tray_builder.icon(icon.clone());
            }
            tray_builder.build(app)?;

            // 鲸鱼娘宠物窗 + 右侧网速气泡：整组贴屏幕右下（右 22px，底 120px）
            // 单独左移 166px 给气泡让位，否则气泡会被推出屏幕右侧。计算放在 server 里
            // 以复用 NET_GAP_LP，避免两处硬编码间隙。
            server::place_pet_home();

            // 气泡跟随线程：宠物拖到哪，气泡跟到哪（越界时自动翻到左侧）
            server::start_net_follow();

            Ok(())
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_window_event(|window, event| {
            // 关窗 = 隐藏到托盘，不退出
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("tauri 启动失败");
}
