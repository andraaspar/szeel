//! The szeel editor: a Tauri shell around the Svelte UI in `web/src`.
//!
//! The UI does all editing; this side parses and renders scenes with the
//! `szeel` crate, reads and writes files, and watches the open file for
//! changes made by other programs.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analyze;

use std::fs;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::ipc::Response;
use tauri::{AppHandle, Emitter, State};

/// Parses a scene and outlines its paths.
#[tauri::command]
async fn analyze(src: String) -> analyze::Analysis {
    analyze::analyze(&src)
}

/// Renders a scene at `scale` times its size. Returns straight RGBA bytes,
/// preceded by the width and height as little-endian `u32`s and the render
/// time in milliseconds as a little-endian `f32`; or nothing if the scene
/// has errors.
#[tauri::command]
async fn render(src: String, scale: f64) -> Result<Response, String> {
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let t0 = Instant::now();
        let Some(scene) = analyze::scene(&src, scale) else {
            return Vec::new();
        };
        let canvas = szeel::render::render(&scene);
        let ms = t0.elapsed().as_secs_f32() * 1000.0;
        let (w, h) = (canvas.width, canvas.height);
        let mut out = Vec::with_capacity(12 + w * h * 4);
        out.extend_from_slice(&(w as u32).to_le_bytes());
        out.extend_from_slice(&(h as u32).to_le_bytes());
        out.extend_from_slice(&ms.to_le_bytes());
        for y in 0..h {
            for x in 0..w {
                out.extend_from_slice(&canvas.pixel(x, y));
            }
        }
        out
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(Response::new(bytes))
}

/// Renders a scene at `scale` times its size and writes it to `path` as PNG.
#[tauri::command]
async fn export_png(src: String, scale: f64, path: PathBuf) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let scene = analyze::scene(&src, scale).ok_or("The scene has errors")?;
        let canvas = szeel::render::render(&scene);
        let file = fs::File::create(&path).map_err(|e| e.to_string())?;
        canvas.write_png(BufWriter::new(file)).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Reformats a scene's source.
#[tauri::command]
fn format(src: String) -> Result<String, String> {
    szeel::format::format(&src).map_err(|e| e.to_string())
}

#[tauri::command]
fn read_text(path: PathBuf) -> Result<String, String> {
    fs::read_to_string(&path).map_err(|e| e.to_string())
}

/// Writes `text` to `path` through a temporary file in the same directory,
/// so a crash mid-write never leaves a truncated scene behind.
#[tauri::command]
fn write_text(path: PathBuf, text: String) -> Result<(), String> {
    let name = path.file_name().ok_or("Not a file path")?.to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.szeel-tmp"));
    let result = fs::write(&tmp, text).and_then(|()| {
        if let Ok(meta) = fs::metadata(&path) {
            fs::set_permissions(&tmp, meta.permissions())?;
        }
        fs::rename(&tmp, &path)
    });
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(|e| e.to_string())
}

/// The scene file named on the command line, if any.
#[tauri::command]
fn initial_file() -> Option<PathBuf> {
    let arg = std::env::args_os().skip(1).find(|a| !a.to_string_lossy().starts_with('-'))?;
    std::path::absolute(arg).ok()
}

/// The watcher for the open file, if there is one.
#[derive(Default)]
struct Watch(Mutex<Option<RecommendedWatcher>>);

/// Watches `path`, replacing any earlier watch, and emits `file-changed`
/// when it changes. `None` stops watching.
///
/// Watches the directory rather than the file, since many programs save by
/// writing a new file and renaming it over the old one.
#[tauri::command]
fn watch(path: Option<PathBuf>, app: AppHandle, state: State<Watch>) -> Result<(), String> {
    let mut slot = state.0.lock().unwrap();
    *slot = None;
    let Some(path) = path else { return Ok(()) };
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let target = path.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        if matches!(ev.kind, EventKind::Access(_)) {
            return;
        }
        if ev.paths.iter().any(|p| *p == target) {
            let _ = app.emit("file-changed", &target);
        }
    })
    .map_err(|e| e.to_string())?;
    watcher.watch(dir, RecursiveMode::NonRecursive).map_err(|e| e.to_string())?;
    *slot = Some(watcher);
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Watch::default())
        .invoke_handler(tauri::generate_handler![
            analyze,
            render,
            export_png,
            format,
            read_text,
            write_text,
            initial_file,
            watch,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the szeel editor");
}
