// Masque la console sous Windows en version release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use companion_core::{discover, report, verify, Package, Report};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

#[derive(Default)]
struct AppState {
    /// Dossier (ou fichier) déposé et MHL trouvés
    source: Mutex<Option<PathBuf>>,
    packages: Mutex<Vec<Package>>,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    last: Mutex<Option<Report>>,
}

#[derive(Serialize, Clone)]
struct Fatal {
    message: String,
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    app.dialog().file().blocking_pick_folder().map(|p| p.to_string())
}

#[tauri::command]
async fn pick_mhl(app: AppHandle) -> Option<String> {
    app.dialog().file().add_filter("MHL", &["mhl"]).blocking_pick_file().map(|p| p.to_string())
}

/// Cherche les MHL sous le chemin donné et les garde pour le contrôle.
#[tauri::command]
async fn scan(state: State<'_, AppState>, path: String) -> Result<Vec<Package>, String> {
    if state.cancel.lock().unwrap().is_some() {
        return Err("busy".into());
    }
    let p = PathBuf::from(&path);
    let p2 = p.clone();
    let pkgs = tauri::async_runtime::spawn_blocking(move || discover(&p2)).await.map_err(|e| e.to_string())??;
    *state.source.lock().unwrap() = Some(p);
    *state.packages.lock().unwrap() = pkgs.clone();
    *state.last.lock().unwrap() = None;
    Ok(pkgs)
}

/// Lance le contrôle des paquets choisis. Les événements partent sous le nom « check ».
#[tauri::command]
fn start_check(app: AppHandle, state: State<AppState>, ids: Vec<usize>) -> Result<(), String> {
    let mut guard = state.cancel.lock().unwrap();
    if guard.is_some() {
        return Err("busy".into());
    }
    let source = state.source.lock().unwrap().clone().ok_or("no source")?;
    let pkgs: Vec<Package> = state.packages.lock().unwrap().iter().filter(|p| ids.contains(&p.id)).cloned().collect();
    if pkgs.is_empty() {
        return Err("nothing to check".into());
    }
    let flag = Arc::new(AtomicBool::new(false));
    *guard = Some(flag.clone());
    drop(guard);

    std::thread::spawn(move || {
        let app2 = app.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            verify(&source, &pkgs, &flag, &mut |e| {
                let _ = app2.emit("check", &e);
            })
        }));
        let st = app.state::<AppState>();
        match result {
            Ok(r) => *st.last.lock().unwrap() = Some(r),
            Err(_) => {
                let _ = app.emit("check-fatal", Fatal { message: "internal error".into() });
            }
        }
        *st.cancel.lock().unwrap() = None;
    });
    Ok(())
}

#[tauri::command]
fn cancel_check(state: State<AppState>) {
    if let Some(f) = state.cancel.lock().unwrap().as_ref() {
        f.store(true, Ordering::Relaxed);
    }
}

/// Enregistre le rapport texte là où l'utilisateur le choisit.
#[tauri::command]
async fn export_report(app: AppHandle, state: State<'_, AppState>, lang: String) -> Result<Option<String>, String> {
    let Some(r) = state.last.lock().unwrap().clone() else { return Err("no report".into()) };
    let base = r
        .source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "rushes".into());
    let prefix = match lang.as_str() {
        "en" => "Integrity_check",
        "es" => "Control_integridad",
        _ => "Controle_integrite",
    };
    let name = format!("{prefix}_{base}_{}.txt", r.finished.format("%Y-%m-%d_%H%M"));
    let Some(dest) = app.dialog().file().set_file_name(&name).add_filter("Text", &["txt"]).blocking_save_file() else {
        return Ok(None);
    };
    let dest = dest.into_path().map_err(|e| e.to_string())?;
    let mut txt = report::text(&r, &lang);
    if cfg!(windows) {
        txt = txt.replace('\n', "\r\n");
    }
    std::fs::write(&dest, txt).map_err(|e| e.to_string())?;
    Ok(Some(dest.display().to_string()))
}

#[tauri::command]
fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if p.exists() {
        return app.opener().reveal_item_in_dir(p).map_err(|e| e.to_string());
    }
    // fichier manquant : on ouvre le dossier qui aurait dû le contenir
    match p.ancestors().skip(1).find(|a| a.is_dir()) {
        Some(d) => app.opener().open_path(d.display().to_string(), None::<&str>).map_err(|e| e.to_string()),
        None => Err("not found".into()),
    }
}

#[tauri::command]
fn open_path(app: AppHandle, path: String) -> Result<(), String> {
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

/// Chemin passé au lancement (dossier glissé sur l'icône de l'app, « Ouvrir avec »).
#[tauri::command]
fn initial_path() -> Option<String> {
    std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .find(|a| !a.starts_with('-') && std::path::Path::new(a).exists())
}

#[tauri::command]
fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("invalid url".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            pick_folder,
            pick_mhl,
            scan,
            start_check,
            cancel_check,
            export_report,
            reveal_path,
            open_path,
            open_url,
            initial_path
        ])
        .run(tauri::generate_context!())
        .expect("Bandua MHL Companion failed to start");
}
