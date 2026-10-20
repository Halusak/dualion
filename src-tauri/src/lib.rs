mod checksum;
mod commands;
#[cfg(debug_assertions)]
mod devtools;
mod error;
mod filter;
mod ftp;
mod icons;
mod jobs;
mod local;
mod location;
mod menu;
mod platform;
mod meta;
mod preview;
mod rename;
mod query;
mod search;
mod tags;
mod thumbs;
mod util;
mod vault;
mod viewer;
mod watcher;

pub(crate) use jobs::archive as archive_index;

use commands::AppState;
use std::collections::HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use tauri::http::{Request, Response};
use tauri::{Emitter, Manager, RunEvent, UriSchemeResponder, WindowEvent};

fn query_px(req: &Request<Vec<u8>>) -> u32 {
    req.uri()
        .query()
        .and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("s=")))
        .and_then(|v| v.parse().ok())
        .unwrap_or(32)
}

fn respond(responder: UriSchemeResponder, body: Option<(Vec<u8>, &'static str)>) {
    let resp = match body {
        Some((bytes, mime)) => Response::builder()
            .status(200)
            .header("Content-Type", mime)
            .header("Cache-Control", "max-age=604800")
            .header("Access-Control-Allow-Origin", "*")
            .body(bytes),
        None => Response::builder().status(404).header("Access-Control-Allow-Origin", "*").body(Vec::new()),
    };
    responder.respond(resp.expect("valid response"));
}

/// Applies the platform backdrop: vibrancy on macOS, Mica on Windows 11.
fn apply_effects(app: &tauri::App) -> &'static str {
    use tauri::window::{Effect, EffectState, EffectsBuilder};
    let Some(win) = app.get_webview_window("main") else { return "none" };
    if cfg!(target_os = "macos") {
        let fx = EffectsBuilder::new().effect(Effect::Sidebar).state(EffectState::FollowsWindowActiveState).build();
        if win.set_effects(fx).is_ok() {
            return "vibrancy";
        }
    } else if cfg!(windows) && platform::os_build() >= 22000 {
        let fx = EffectsBuilder::new().effect(Effect::Mica).build();
        if win.set_effects(fx).is_ok() {
            return "mica";
        }
    }
    "none"
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol("thumb", |_ctx, req, responder| {
            let path = percent_encoding::percent_decode_str(req.uri().path().trim_start_matches('/'))
                .decode_utf8_lossy()
                .into_owned();
            let px = query_px(&req);
            thumbs::spawn(move || {
                let t = thumbs::thumbnail(std::path::Path::new(&path), px);
                respond(responder, t.map(|t| (t.bytes.clone(), t.mime)));
            });
        })
        .register_asynchronous_uri_scheme_protocol("icon", |_ctx, req, responder| {
            let key = icons::IconKey::parse(req.uri().path());
            let px = query_px(&req);
            std::thread::spawn(move || {
                let png = key.and_then(|k| icons::icon_png(k, px));
                respond(responder, png.map(|p| (p.to_vec(), "image/png")));
            });
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_data_dir()?;
            if let Ok(cache) = app.path().app_cache_dir() {
                thumbs::set_cache_dir(cache.join("thumbs"));
            }
            let ftp = Arc::new(ftp::FtpRegistry::default());
            let emitter = handle.clone();
            let sink: jobs::EventSink = Arc::new(move |name, payload| {
                let _ = emitter.emit(name, payload);
            });
            let jobs = jobs::JobManager::new(sink, ftp.clone());
            let vault = Arc::new(vault::Vault::new(data_dir.join("sites.vault")));
            let watcher = watcher::DirWatcher::new(handle.clone());
            let effects = apply_effects(app);

            // The window starts hidden and the UI shows it once rendered; never
            // leave the user without a window if the frontend fails to start.
            let fallback = handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(4));
                if let Some(w) = fallback.get_webview_window("main") {
                    if !w.is_visible().unwrap_or(true) {
                        let _ = w.show();
                    }
                }
            });

            let keep = ftp.clone();
            std::thread::Builder::new()
                .name("ftp-keepalive".into())
                .spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_secs(30));
                    keep.keepalive();
                })?;

            #[cfg(debug_assertions)]
            devtools::start(handle.clone());

            app.manage(AppState {
                ftp,
                jobs,
                vault,
                watcher,
                ops: parking_lot::Mutex::new(HashMap::new()),
                next_op: AtomicU64::new(1),
                config_dir,
                effects,
            });
            Ok(())
        })
        .on_menu_event(|app, event| {
            let _ = app.emit("menu", event.id().0.as_str());
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if state.jobs.active_count() > 0 {
                    api.prevent_close();
                    let _ = window.emit("close-requested", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::set_menu,
            commands::store_load,
            commands::store_save,
            commands::watch_dirs,
            commands::cancel_op,
            commands::app_exit,
            commands::active_jobs,
            commands::open_path,
            commands::reveal_path,
            commands::open_url,
            commands::qr_svg,
            commands::open_terminal,
            commands::edit_file,
            commands::open_with,
            commands::quick_look,
            commands::show_properties,
            commands::list_dir,
            commands::volumes,
            commands::space,
            commands::dir_size,
            commands::resolve_path,
            commands::complete_path,
            commands::make_dir,
            commands::create_file,
            commands::rename_item,
            commands::batch_rename,
            commands::create_link,
            commands::write_text_file,
            commands::materialize,
            commands::job_enqueue,
            commands::jobs_list,
            commands::job_pause,
            commands::job_cancel,
            commands::job_answer,
            commands::job_start_now,
            commands::job_move,
            commands::jobs_clear_finished,
            commands::search_start,
            commands::viewer_probe,
            commands::viewer_read_text,
            commands::viewer_read_bytes,
            commands::preview_sheet,
            commands::preview_document,
            commands::preview_slides,
            commands::file_meta,
            commands::query_check,
            commands::query_filter,
            commands::checksum_start,
            commands::icon_png,
            commands::vault_status,
            commands::vault_unlock,
            commands::vault_lock,
            commands::vault_set_master,
            commands::sites_list,
            commands::site_save,
            commands::site_delete,
            commands::ftp_connect,
            commands::ftp_test,
            commands::ftp_disconnect,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Dualion");

    app.run(|handle, event| {
        if let RunEvent::ExitRequested { api, code, .. } = event {
            // Cmd+Q / last window closed while files are still being copied.
            if code.is_none() {
                if let Some(state) = handle.try_state::<AppState>() {
                    if state.jobs.active_count() > 0 {
                        api.prevent_exit();
                        let _ = handle.emit("close-requested", ());
                    }
                }
            }
        }
    });
}
