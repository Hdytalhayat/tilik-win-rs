//! Windows platform integration: single-instance lock, system tray,
//! global hotkey, and autostart registration.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use eframe::egui;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIconBuilder};
use winapi::shared::winerror::ERROR_ALREADY_EXISTS;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::handleapi::CloseHandle;
use winapi::um::synchapi::CreateMutexW;
use winapi::um::winuser::{
    DispatchMessageW, FindWindowW, MSG, PM_REMOVE, PeekMessageW, SW_SHOW, SetForegroundWindow,
    ShowWindow, TranslateMessage,
};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
use winreg::RegKey;

const MUTEX_NAME: &str = "Local\\TilikWinRs";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const AUTOSTART_VALUE: &str = "Tilik";

/// Set by `App::update` so the dispatcher thread can show/hide the window
/// even while it is hidden (egui::Context is Send+Sync).
static UI_CTX: Mutex<Option<egui::Context>> = Mutex::new(None);
static VISIBLE: AtomicBool = AtomicBool::new(true);
static NEED_FOCUS: AtomicBool = AtomicBool::new(false);

/// Holds the named mutex alive; the tray/hotkey live on their own thread.
pub struct Platform {
    _instance_lock: *mut winapi::ctypes::c_void,
}

/// Fails with `already running` when a second instance launches.
pub fn init(
    hotkey_spec: &str,
    reindex_tx: mpsc::Sender<crate::Cmd>,
) -> anyhow::Result<Platform> {
    // 1. Single-instance lock.
    let wide: Vec<u16> = format!("{MUTEX_NAME}\0").encode_utf16().collect();
    let lock = unsafe { CreateMutexW(std::ptr::null_mut(), 0, wide.as_ptr()) };
    if lock.is_null() {
        return Err(anyhow::anyhow!("failed to create instance mutex"));
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe { CloseHandle(lock) };
        return Err(anyhow::anyhow!("already running"));
    }

    // 2-4. Menu items are Rc-based (!Send), so they and the dispatcher
    //      live together on one dedicated platform thread; only the
    //      Send `TrayIcon` handle crosses back to the main thread.
    let (tray_tx, tray_rx) = mpsc::channel::<anyhow::Result<()>>();
    let spec = hotkey_spec.to_string();
    thread::spawn(move || {
        let open_item = MenuItem::new("Open Search", true, None);
        let reindex_item = MenuItem::new("Re-index Now", true, None);
        let autostart_item =
            CheckMenuItem::new("Start with Windows", true, is_autostart_enabled(), None);
        let quit_item = MenuItem::new("Quit", true, None);
        let open_id = open_item.id();
        let reindex_id = reindex_item.id();
        let autostart_id = autostart_item.id();
        let quit_id = quit_item.id();

        let menu = Menu::new();
        if menu.append(&open_item).is_err()
            || menu.append(&reindex_item).is_err()
            || menu.append(&autostart_item).is_err()
            || menu.append(&quit_item).is_err()
        {
            let _ = tray_tx.send(Err(anyhow::anyhow!("menu build failed")));
            return;
        }

        let tray = match TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(make_icon())
            .with_tooltip("Tilik")
            .build()
        {
            Ok(t) => t,
            Err(e) => {
                let _ = tray_tx.send(Err(anyhow::anyhow!("tray: {e}")));
                return;
            }
        };

        // Hotkey is optional: a conflict (e.g. Alt+Space owned by another
        // app/Windows) must not kill the tray or autostart functionality.
        let hotkey_ok = {
            let mut ok = false;
            if let Ok(manager) = GlobalHotKeyManager::new() {
                if let Ok(hotkey) = parse_hotkey(&spec) {
                    if manager.register(hotkey).is_ok() {
                        // Leak the manager into a Box so registration stays alive.
                        Box::leak(Box::new(manager));
                        ok = true;
                    } else {
                        eprintln!("tilik: hotkey '{spec}' already in use; global hotkey disabled");
                    }
                } else {
                    eprintln!("tilik: invalid hotkey '{spec}'");
                }
            } else {
                eprintln!("tilik: global hotkey manager unavailable");
            }
            ok
        };

        // Keep the tray alive for the lifetime of this thread (TrayIcon is !Send).
        let _tray = tray;
        let _ = tray_tx.send(Ok(()));

        // Dispatcher: polls hotkey + tray events, and pumps Win32 messages.
        loop {
            if hotkey_ok {
                if let Ok(ev) = GlobalHotKeyEvent::receiver().try_recv() {
                    if ev.state() == HotKeyState::Pressed {
                        toggle_visibility();
                    }
                }
            }
            if let Ok(ev) = MenuEvent::receiver().try_recv() {
                match ev.id() {
                    id if id == open_id => show_window(),
                    id if id == reindex_id => {
                        let _ = reindex_tx.send(crate::Cmd::Reindex);
                    }
                    id if id == autostart_id => toggle_autostart(&autostart_item),
                    id if id == quit_id => std::process::exit(0),
                    _ => {}
                }
            }
            unsafe {
                let mut msg: MSG = std::mem::zeroed();
                while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            thread::sleep(Duration::from_millis(16));
        }
    });

    tray_rx.recv().unwrap()?;
 
    Ok(Platform {
        _instance_lock: lock,
    })
}

fn show_window() {
    set_visible(true);
}

fn toggle_visibility() {
    set_visible(!VISIBLE.load(Ordering::SeqCst));
}

fn set_visible(visible: bool) {
    VISIBLE.store(visible, Ordering::SeqCst);
    if visible {
        NEED_FOCUS.store(true, Ordering::SeqCst);
        unsafe {
            let title: Vec<u16> = "Tilik\0".encode_utf16().collect();
            let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
            if !hwnd.is_null() {
                ShowWindow(hwnd, SW_SHOW);
                SetForegroundWindow(hwnd);
            }
        }
    }
    let Some(ctx) = UI_CTX.lock().unwrap().clone() else {
        return;
    };
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(visible));
    if visible {
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
            egui::UserAttentionType::Critical,
        ));
    }
    ctx.request_repaint();
}

fn toggle_autostart(item: &CheckMenuItem) {
    let next = !is_autostart_enabled();
    match set_autostart(next) {
        Ok(()) => item.set_checked(next),
        Err(e) => eprintln!("autostart toggle failed: {e}"),
    }
}

/// `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` registration.
pub fn set_autostart(enabled: bool) -> anyhow::Result<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_SET_VALUE)?;
    if enabled {
        let exe = std::env::current_exe()?;
        key.set_value(AUTOSTART_VALUE, &format!("\"{}\"", exe.to_string_lossy()))?;
    } else {
        let _ = key.delete_value(AUTOSTART_VALUE); // absent value is not an error
    }
    Ok(())
}

pub fn is_autostart_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .ok()
        .and_then(|k| k.get_value::<String, _>(AUTOSTART_VALUE).ok())
        .is_some()
}

fn make_icon() -> Icon {
    const SIZE: u32 = 32;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for px in rgba.chunks_exact_mut(4) {
        px.copy_from_slice(&[74, 144, 217, 255]); // solid Windows blue
    }
    Icon::from_rgba(rgba, SIZE, SIZE).expect("valid icon bytes")
}

/// Parses "Ctrl+Alt+F"-style specs (Ctrl/Alt/Shift/CmdOrCtrl + letter,
/// F1-F24, or Space).
fn parse_hotkey(spec: &str) -> anyhow::Result<HotKey> {
    let mut mods = Modifiers::empty();
    let mut key: Option<Code> = None;
    for part in spec.split('+') {
        let p = part.trim().to_ascii_lowercase();
        match p.as_str() {
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "alt" => mods |= Modifiers::ALT,
            "shift" => mods |= Modifiers::SHIFT,
            // shortcut: CmdOrCtrl/Super maps to Ctrl on Windows
            "cmdorctrl" | "cmd" | "super" => mods |= Modifiers::CONTROL,
            "space" => key = Some(Code::Space),
            s if s.len() == 1 && s.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) => {
                key = Some(match s.chars().next().unwrap().to_ascii_uppercase() {
                    'A' => Code::KeyA,
                    'B' => Code::KeyB,
                    'C' => Code::KeyC,
                    'D' => Code::KeyD,
                    'E' => Code::KeyE,
                    'F' => Code::KeyF,
                    'G' => Code::KeyG,
                    'H' => Code::KeyH,
                    'I' => Code::KeyI,
                    'J' => Code::KeyJ,
                    'K' => Code::KeyK,
                    'L' => Code::KeyL,
                    'M' => Code::KeyM,
                    'N' => Code::KeyN,
                    'O' => Code::KeyO,
                    'P' => Code::KeyP,
                    'Q' => Code::KeyQ,
                    'R' => Code::KeyR,
                    'S' => Code::KeyS,
                    'T' => Code::KeyT,
                    'U' => Code::KeyU,
                    'V' => Code::KeyV,
                    'W' => Code::KeyW,
                    'X' => Code::KeyX,
                    'Y' => Code::KeyY,
                    'Z' => Code::KeyZ,
                    _ => unreachable!(),
                });
            }
            s if s.starts_with('f') && s.len() <= 3 => {
                let n: u8 = s[1..]
                    .parse()
                    .map_err(|_| anyhow::anyhow!("bad function key '{p}'"))?;
                key = Some(match n {
                    1 => Code::F1,
                    2 => Code::F2,
                    3 => Code::F3,
                    4 => Code::F4,
                    5 => Code::F5,
                    6 => Code::F6,
                    7 => Code::F7,
                    8 => Code::F8,
                    9 => Code::F9,
                    10 => Code::F10,
                    11 => Code::F11,
                    12 => Code::F12,
                    13 => Code::F13,
                    14 => Code::F14,
                    15 => Code::F15,
                    16 => Code::F16,
                    17 => Code::F17,
                    18 => Code::F18,
                    19 => Code::F19,
                    20 => Code::F20,
                    21 => Code::F21,
                    22 => Code::F22,
                    23 => Code::F23,
                    24 => Code::F24,
                    _ => return Err(anyhow::anyhow!("unsupported function key '{p}'")),
                });
            }
            _ => return Err(anyhow::anyhow!("unsupported hotkey part '{p}'")),
        }
    }
    Ok(HotKey::new(
        if mods.is_empty() { None } else { Some(mods) },
        key.ok_or_else(|| anyhow::anyhow!("hotkey '{spec}' has no key"))?,
    ))
}

/// Called from `App::update` so the dispatcher can reach the UI context.
pub fn publish_context(ctx: &egui::Context) {
    *UI_CTX.lock().unwrap() = Some(ctx.clone());
}

/// Consumes a pending focus request created by showing the window.
pub fn take_focus_request() -> bool {
    NEED_FOCUS.swap(false, Ordering::SeqCst)
}
