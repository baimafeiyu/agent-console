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

            // 鲸鱼娘宠物窗：贴屏幕右下（右 22px，底 120px，与 dsh web 宠物习惯一致）
            if let Some(pet) = app.get_webview_window("pet") {
                if let Ok(Some(mon)) = pet.current_monitor() {
                    if let Ok(size) = pet.outer_size() {
                        let x = mon.size().width as i32 - size.width as i32 - 22;
                        let y = mon.size().height as i32 - size.height as i32 - 120;
                        let _ = pet.set_position(tauri::PhysicalPosition::new(x, y));
                    }
                }
            }

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
