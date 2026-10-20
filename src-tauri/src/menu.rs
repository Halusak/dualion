//! Native application menu built from a spec sent by the frontend, so the
//! labels follow the selected UI language and check marks follow state.

use serde::Deserialize;
use tauri::menu::{
    AboutMetadata, CheckMenuItem, IsMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID,
    WINDOW_SUBMENU_ID,
};
use tauri::{AppHandle, Runtime};

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MenuNode {
    Item {
        id: String,
        label: String,
        accel: Option<String>,
        #[serde(default = "yes")]
        enabled: bool,
    },
    Check {
        id: String,
        label: String,
        accel: Option<String>,
        checked: bool,
        #[serde(default = "yes")]
        enabled: bool,
    },
    Separator,
    Submenu {
        label: String,
        items: Vec<MenuNode>,
        /// "window" / "help" make macOS treat the submenu specially.
        role: Option<String>,
    },
    Predefined {
        name: String,
        label: Option<String>,
    },
}

fn item<R: Runtime>(app: &AppHandle<R>, node: &MenuNode) -> tauri::Result<Option<Box<dyn IsMenuItem<R>>>> {
    Ok(Some(match node {
        MenuNode::Item { id, label, accel, enabled } => {
            match MenuItem::with_id(app, id.as_str(), label, *enabled, accel.as_deref()) {
                Ok(i) => Box::new(i),
                // An accelerator the platform does not understand must not break the menu.
                Err(_) => Box::new(MenuItem::with_id(app, id.as_str(), label, *enabled, None::<&str>)?),
            }
        }
        MenuNode::Check { id, label, accel, checked, enabled } => {
            match CheckMenuItem::with_id(app, id.as_str(), label, *enabled, *checked, accel.as_deref()) {
                Ok(i) => Box::new(i),
                Err(_) => Box::new(CheckMenuItem::with_id(app, id.as_str(), label, *enabled, *checked, None::<&str>)?),
            }
        }
        MenuNode::Separator => Box::new(PredefinedMenuItem::separator(app)?),
        MenuNode::Submenu { label, items, role } => {
            let id = match role.as_deref() {
                Some("window") => WINDOW_SUBMENU_ID.to_string(),
                Some("help") => HELP_SUBMENU_ID.to_string(),
                _ => format!("submenu:{label}"),
            };
            let sub = Submenu::with_id(app, id, label, true)?;
            for child in items {
                if let Some(c) = item(app, child)? {
                    sub.append(c.as_ref())?;
                }
            }
            Box::new(sub)
        }
        MenuNode::Predefined { name, label } => {
            let l = label.as_deref();
            let p = match name.as_str() {
                "about" => PredefinedMenuItem::about(
                    app,
                    l,
                    Some(AboutMetadata {
                        name: Some("Dualion".into()),
                        version: Some(app.package_info().version.to_string()),
                        authors: Some(vec!["Haluška".into()]),
                        comments: Some("Dual-pane file manager · Tauri · Rust · Svelte".into()),
                        copyright: Some("© 2026 Haluška".into()),
                        website: Some("https://discord.com/invite/DjnKfuYYgN".into()),
                        website_label: Some("Discord".into()),
                        ..Default::default()
                    }),
                )?,
                "hide" => PredefinedMenuItem::hide(app, l)?,
                "hideOthers" => PredefinedMenuItem::hide_others(app, l)?,
                "showAll" => PredefinedMenuItem::show_all(app, l)?,
                "services" => PredefinedMenuItem::services(app, l)?,
                "quit" => PredefinedMenuItem::quit(app, l)?,
                "undo" => PredefinedMenuItem::undo(app, l)?,
                "redo" => PredefinedMenuItem::redo(app, l)?,
                "cut" => PredefinedMenuItem::cut(app, l)?,
                "copy" => PredefinedMenuItem::copy(app, l)?,
                "paste" => PredefinedMenuItem::paste(app, l)?,
                "selectAll" => PredefinedMenuItem::select_all(app, l)?,
                "minimize" => PredefinedMenuItem::minimize(app, l)?,
                "maximize" => PredefinedMenuItem::maximize(app, l)?,
                "fullscreen" => PredefinedMenuItem::fullscreen(app, l)?,
                "closeWindow" => PredefinedMenuItem::close_window(app, l)?,
                "bringAllToFront" => PredefinedMenuItem::bring_all_to_front(app, l)?,
                _ => return Ok(None),
            };
            Box::new(p)
        }
    }))
}

pub fn build<R: Runtime>(app: &AppHandle<R>, nodes: &[MenuNode]) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;
    for n in nodes {
        if let Some(i) = item(app, n)? {
            menu.append(i.as_ref())?;
        }
    }
    Ok(menu)
}
