#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod logging;
mod model;
mod sensor;
mod signature;
mod species;
mod store;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, State, WebviewWindow};

use model::{Agg, Eco, Genome};
use sensor::{Event, Sample, Tracker};
use signature::Signature;
use store::Store;

const WIN_W: f64 = 224.0;
const WIN_H: f64 = 300.0;
/// 收进小圆点时的窗口尺寸。
const DOCK_SIZE: f64 = 40.0;
/// 图鉴面板打开时临时放大的窗口尺寸(关闭恢复)。
const CODEX_W: f64 = 340.0;
const CODEX_H: f64 = 460.0;

/// 培养皿的几何中心与半径（逻辑像素，未缩放），用于点击穿透的命中测试。
const DISH_CX: f64 = 112.0;
const DISH_CY: f64 = 104.0;
const DISH_R: f64 = 88.0;

const ERROR_ALREADY_EXISTS: u32 = 183;

/// Shell 是否收下了托盘图标（这台机器上被安全软件拒绝，前端要如实显示）。
static TRAY_OK: AtomicBool = AtomicBool::new(false);

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(windows)]
fn local_date_key() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t = unsafe { std::mem::zeroed::<windows_sys::Win32::Foundation::SYSTEMTIME>() };
    unsafe { GetLocalTime(&mut t) };
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}

#[cfg(not(windows))]
fn local_date_key() -> String {
    String::from("1970-01-01")
}

#[cfg(windows)]
fn local_hour() -> u8 {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut t = unsafe { std::mem::zeroed::<windows_sys::Win32::Foundation::SYSTEMTIME>() };
    unsafe { GetLocalTime(&mut t) };
    (t.wHour & 0xFF) as u8
}

#[cfg(not(windows))]
fn local_hour() -> u8 {
    0
}

/// 单实例：第二个进程直接退出，避免两个缸抢同一个 WebView2 目录。
fn already_running() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let name: Vec<u16> = "Local\\BehaviorEcoJar.SingleInstance\0".encode_utf16().collect();
        let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        if handle.is_null() {
            logging::write("single-instance mutex creation failed");
            return false;
        }
        windows_sys::Win32::Foundation::GetLastError() == ERROR_ALREADY_EXISTS
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[derive(Serialize, Clone)]
struct TraitsView {
    vitality: f32,
    glow: f32,
    hue: f32,
    turbidity: f32,
    motility: f32,
    rhythm: f32,
    spots: f32,
}

#[derive(Serialize, Clone)]
struct SpeciesView {
    id: String,
    name: String,
    archetype: String,
}

#[derive(Serialize, Clone)]
struct Snapshot {
    stage: u8,
    stage_name: String,
    growth: f32,
    traits: TraitsView,
    sig: Signature,
    caption: String,
    active_minutes: f64,
    focus_minutes: f64,
    paused: bool,
    demo: bool,
    last_break_ms: u64,
    data_dir: String,
    /// 光标是否落在培养皿内（由悬停线程写入，供前端轮询）。
    hover: bool,
    /// 最近一次采样是否处于活跃状态，前端据此调整帧率。
    active: bool,
    idle_sec: u64,
    fg_app: Option<String>,
    uptime_sec: u64,
    diag_open: bool,
    buckets60: Vec<u8>,
    day_active_min: f64,
    day_breaks: u32,
    autostart_state: u8,
    tray_ok: bool,
    log_path: String,
    ui_scale: f32,
    opacity: f32,
    collapsed: bool,
    /// 当前代数与个体基因(M0 起随快照下发,前端 M1 使用)。
    generation: u32,
    archetype: String,
    genome: Genome,
    creature_name: String,
    /// 显形后锁定的物种;None = 未定型(按通用浮伞形态渲染)。
    species: Option<SpeciesView>,
    /// 是否已可收集(成体起)。
    can_collect: bool,
}

struct Inner {
    tracker: Tracker,
    eco: Eco,
    sig: Signature,
    paused: bool,
    demo: bool,
    demo_base: u64,
    demo_elapsed: u64,
    last_save: u64,
    data_dir: String,
    last_active: bool,
    last_idle_ms: u64,
    last_fg: Option<String>,
    started_ms: u64,
    last_wall: u64,
    day_key: String,
    day_active_sec: u64,
    day_breaks: u32,
    autostart: bool,
    /// 0=关闭 1=已登记 2=被安全软件拦截
    autostart_state: u8,
    /// 截图/演示用：临时把外观钉在某个阶段，不改真实进度。
    preview_stage: Option<u8>,
    ui_scale: f32,
    opacity: f32,
    collapsed: bool,
    /// 世代制(M0):基因、终身聚合、代数。
    genome: Genome,
    agg: Agg,
    generation: u32,
    creature_name: Option<String>,
    /// 世代制(M1):显形物种、落缸日期、累计休息、演示自动收集计时。
    species: Option<&'static species::SpeciesDef>,
    started_on: String,
    gen_breaks: u32,
    demo_bloom_sim: u64,
}

struct TrayHandles {
    pause: MenuItem<tauri::Wry>,
    autostart: CheckMenuItem<tauri::Wry>,
    show: MenuItem<tauri::Wry>,
}

struct AppState {
    inner: Mutex<Inner>,
    menu_open: AtomicBool,
    hover_now: AtomicBool,
    diag_open: AtomicBool,
    codex_open: AtomicBool,
    ui_scale_bits: AtomicU32,
    collapsed: AtomicBool,
    store: Store,
    tray: TrayHandles,
}

impl AppState {
    fn snapshot_at(&self, inner: &Inner, now: u64) -> Snapshot {
        let diag = self.diag_open.load(Ordering::Relaxed);
        let (stage, growth, stage_name) = match inner.preview_stage {
            Some(s) => {
                let s = s.min(3);
                (s, 0.5, model::STAGES[s as usize].1.to_string())
            }
            None => (
                inner.eco.stage,
                inner.eco.growth(),
                inner.eco.stage_name().to_string(),
            ),
        };
        Snapshot {
            stage,
            stage_name,
            growth,
            traits: TraitsView {
                vitality: inner.eco.vitality,
                glow: inner.eco.glow,
                hue: inner.eco.hue,
                turbidity: inner.eco.turbidity,
                motility: inner.eco.motility,
                rhythm: inner.eco.rhythm_t,
                spots: inner.eco.spots,
            },
            sig: inner.sig,
            caption: inner.eco.caption(&inner.sig).to_string(),
            active_minutes: inner.eco.active_minutes,
            focus_minutes: inner.eco.focus_minutes,
            paused: inner.paused,
            demo: inner.demo,
            last_break_ms: inner.eco.last_break_ms,
            data_dir: inner.data_dir.clone(),
            hover: self.hover_now.load(Ordering::Relaxed),
            active: inner.last_active,
            idle_sec: inner.last_idle_ms / 1000,
            fg_app: inner.last_fg.clone(),
            uptime_sec: now.saturating_sub(inner.started_ms) / 1000,
            diag_open: diag,
            buckets60: if diag {
                inner.tracker.recent_active(60, now)
            } else {
                Vec::new()
            },
            day_active_min: inner.day_active_sec as f64 / 60.0,
            day_breaks: inner.day_breaks,
            autostart_state: inner.autostart_state,
            tray_ok: TRAY_OK.load(Ordering::Relaxed),
            log_path: logging::path_string(),
            ui_scale: inner.ui_scale,
            opacity: inner.opacity,
            collapsed: inner.collapsed,
            generation: inner.generation,
            archetype: inner
                .species
                .map(|d| d.archetype)
                .unwrap_or("medusa")
                .to_string(),
            genome: inner.genome.clone(),
            creature_name: inner
                .creature_name
                .clone()
                .unwrap_or_else(|| format!("第 {} 代", inner.generation)),
            species: inner.species.map(|d| SpeciesView {
                id: d.id.to_string(),
                name: d.name.to_string(),
                archetype: d.archetype.to_string(),
            }),
            can_collect: inner.eco.stage >= 2,
        }
    }
}

#[tauri::command]
fn get_snapshot(state: State<AppState>) -> Snapshot {
    let inner = state.inner.lock().unwrap();
    state.snapshot_at(&inner, now_ms())
}

#[tauri::command]
fn set_paused(app: AppHandle, paused: bool, state: State<AppState>) {
    {
        let mut inner = state.inner.lock().unwrap();
        inner.paused = paused;
        inner.eco.last_tick_ms = now_ms();
    }
    let _ = state
        .tray
        .pause
        .set_text(if paused { "继续观察" } else { "暂停观察" });
    logging::write(if paused { "paused by user" } else { "resumed by user" });
    let _ = app.emit("paused", paused);
}

#[tauri::command]
fn set_menu_open(open: bool, state: State<AppState>) {
    state.menu_open.store(open, Ordering::Relaxed);
}

#[tauri::command]
fn set_diag(open: bool, state: State<AppState>) {
    state.diag_open.store(open, Ordering::Relaxed);
}

/// 找到本进程里 tray-icon 创建的消息窗口。
#[cfg(windows)]
unsafe fn own_tray_window() -> windows_sys::Win32::Foundation::HWND {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowThreadProcessId,
    };

    unsafe extern "system" fn collect(hwnd: HWND, lparam: isize) -> i32 {
        let out = &mut *(lparam as *mut Vec<isize>);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == std::process::id() {
            let mut buf = [0u16; 64];
            let n = GetClassNameW(hwnd, buf.as_mut_ptr(), 64);
            if n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == "tray_icon_app" {
                out.push(hwnd as isize);
            }
        }
        1
    }

    let mut windows: Vec<isize> = Vec::new();
    EnumWindows(Some(collect), &mut windows as *mut _ as isize);
    windows
        .first()
        .map(|h| *h as HWND)
        .unwrap_or(std::ptr::null_mut())
}

/// 直接用 Shell_NotifyIcon 装一个测试图标，绕开 Tauri 的封装。
#[tauri::command]
fn tray_self_test() -> String {
    self_test().1
}

#[cfg(windows)]
fn self_test() -> (bool, String) {
    unsafe {
        use windows_sys::Win32::UI::Shell::{
            Shell_NotifyIconGetRect, Shell_NotifyIconW, NOTIFYICONDATAW, NOTIFYICONIDENTIFIER,
            NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{LoadIconW, IDI_APPLICATION};

        let hwnd = own_tray_window();
        if hwnd.is_null() {
            return (false, "no tray window".into());
        }
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 4242;
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = 0x0400 + 1;
        nid.hIcon = LoadIconW(std::ptr::null_mut(), IDI_APPLICATION);
        for (i, ch) in "EcoJar self test".encode_utf16().enumerate() {
            nid.szTip[i] = ch;
        }
        let ok = Shell_NotifyIconW(NIM_ADD, &nid);
        let err = windows_sys::Win32::Foundation::GetLastError();

        let mut id: NOTIFYICONIDENTIFIER = std::mem::zeroed();
        id.cbSize = std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32;
        id.hWnd = hwnd;
        id.uID = 4242;
        let mut rect: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
        let hr = Shell_NotifyIconGetRect(&id, &mut rect);
        // 别在能用的机器上留下第二个图标。
        Shell_NotifyIconW(NIM_DELETE, &nid);
        (
            ok != 0,
            format!(
                "NIM_ADD ok={ok} err={err} rect_hr=0x{hr:X} rect=({},{},{},{})",
                rect.left, rect.top, rect.right, rect.bottom
            ),
        )
    }
}

#[cfg(not(windows))]
fn self_test() -> (bool, String) {
    (false, "windows only".into())
}

/// 诊断：读回托盘图标在屏幕上的真实矩形。装不上或藏起来时会拿不到 S_OK。
#[tauri::command]
fn tray_probe() -> String {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{HWND, LPARAM};
        use windows_sys::Win32::UI::Shell::{Shell_NotifyIconGetRect, NOTIFYICONIDENTIFIER};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumWindows, GetClassNameW, GetWindowLongPtrW, GetWindowThreadProcessId, GWLP_USERDATA,
        };

        unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> i32 {
            let out = &mut *(lparam as *mut Vec<isize>);
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == std::process::id() {
                let mut buf = [0u16; 64];
                let n = GetClassNameW(hwnd, buf.as_mut_ptr(), 64);
                if n > 0 && String::from_utf16_lossy(&buf[..n as usize]) == "tray_icon_app" {
                    out.push(hwnd as isize);
                }
            }
            1
        }

        let mut windows: Vec<isize> = Vec::new();        EnumWindows(Some(collect), &mut windows as *mut _ as LPARAM);
        if windows.is_empty() {
            return "no tray_icon_app window in this process".into();
        }
        let mut out = String::new();
        for h in &windows {
            let hwnd = *h as HWND;
            let user = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
            out.push_str(&format!("hwnd={hwnd:?} userdata=0x{user:X} "));
            for uid in 1..=8u32 {
                let mut id: NOTIFYICONIDENTIFIER = std::mem::zeroed();
                id.cbSize = std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32;
                id.hWnd = hwnd;
                id.uID = uid;
                let mut rect: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
                let hr = Shell_NotifyIconGetRect(&id, &mut rect);
                if hr == 0 {
                    out.push_str(&format!(
                        "uid={uid} rect=({},{},{},{}) ",
                        rect.left, rect.top, rect.right, rect.bottom
                    ));
                } else {
                    out.push_str(&format!("uid={uid} hr=0x{hr:X} "));
                }
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        "windows only".into()
    }
}

#[tauri::command]
fn reset_jar(app: AppHandle, state: State<AppState>) {
    {
        let mut inner = state.inner.lock().unwrap();
        state.store.archive_state();
        inner.tracker = Tracker::default();
        inner.eco = Eco::default();
        inner.eco.last_tick_ms = now_ms();
        inner.eco.last_break_ms = now_ms();
        inner.sig = Signature::default();
        // 重置 = 重新开始这一代:换一只新生物,代数不前进(不计入图鉴)。
        inner.genome = Genome::random();
        inner.agg = Agg::neutral();
        inner.species = None;
        inner.started_on = local_date_key();
        inner.gen_breaks = 0;
        inner.day_active_sec = 0;
        inner.day_breaks = 0;
        let snap = state.snapshot_at(&inner, now_ms());
        let _ = app.emit("state", snap);
    }
    logging::write("jar reset by user");
    let _ = app.emit("reset", ());
}

#[tauri::command]
fn set_autostart(enabled: bool, app: AppHandle, state: State<AppState>) -> u8 {
    apply_autostart(&app, &state, enabled)
}

/// 只改外观预览，不动真实成长进度。
#[tauri::command]
fn set_preview_stage(stage: Option<u8>, state: State<AppState>) {
    let mut inner = state.inner.lock().unwrap();
    inner.preview_stage = stage.map(|s| s.min(3));
}

/// 收集核心:把当前代归档成 GenerationRecord,并把生态重置为下一代孢子。
/// 上游需保证 stage >= 2(可收集)。
fn perform_collect(inner: &mut Inner, now: u64, today: &str) -> store::GenerationRecord {
    let def = inner
        .species
        .unwrap_or(&species::SPECIES[species::best(&inner.agg)]);
    let full_bloom = inner.eco.stage >= 3;
    let mut marks = Vec::new();
    if inner.generation == 1 {
        marks.push("初代".to_string());
    }
    if !full_bloom {
        marks.push("提前收集".to_string());
    }
    let rec = store::GenerationRecord {
        gen: inner.generation,
        name: inner
            .creature_name
            .clone()
            .unwrap_or_else(|| format!("第 {} 代", inner.generation)),
        species_id: def.id.to_string(),
        species_name: def.name.to_string(),
        archetype: def.archetype.to_string(),
        full_bloom,
        from: inner.started_on.clone(),
        to: today.to_string(),
        active_min: inner.eco.active_minutes,
        focus_min: inner.eco.focus_minutes,
        breaks: inner.gen_breaks,
        days: inner.agg.days,
        marks,
        genome: inner.genome.clone(),
        agg: inner.agg.clone(),
        visual: model::EcoVisual::from(&inner.eco),
    };

    let scale = inner.eco.stage_scale;
    inner.tracker = Tracker::default();
    inner.eco = Eco::default();
    inner.eco.stage_scale = scale;
    inner.eco.last_tick_ms = now;
    inner.eco.last_break_ms = now;
    inner.sig = Signature::default();
    inner.agg = Agg::neutral();
    inner.genome = Genome::random();
    inner.species = None;
    inner.generation += 1;
    inner.creature_name = None;
    inner.started_on = today.to_string();
    inner.gen_breaks = 0;
    rec
}

/// 收集当前这一代:归档 → 下一代孢子落缸。
#[tauri::command]
fn collect_creature(
    app: AppHandle,
    state: State<AppState>,
) -> Result<serde_json::Value, String> {
    let now = now_ms();
    let today = local_date_key();
    let rec;
    {
        let mut inner = state.inner.lock().unwrap();
        if inner.eco.stage < 2 {
            return Err("它还没有定形,再陪它一阵".into());
        }
        rec = perform_collect(&mut inner, now, &today);
        state.store.save_generation(&rec);
    }
    logging::write(&format!(
        "collected gen={} species={} full_bloom={}",
        rec.gen, rec.species_id, rec.full_bloom
    ));
    let _ = app.emit("collected", rec.species_name.clone());
    save_now(&app, &state);
    Ok(serde_json::json!({
        "gen": rec.gen,
        "name": rec.name,
        "species": rec.species_name,
        "full_bloom": rec.full_bloom,
    }))
}

/// 图鉴:物种槽位(含暗示) + 历代档案。
#[tauri::command]
fn get_codex(state: State<AppState>) -> serde_json::Value {
    let gens = state.store.load_generations();
    let slots: Vec<serde_json::Value> = species::SPECIES
        .iter()
        .map(|d| {
            let count = gens.iter().filter(|g| g.species_id == d.id).count() as u32;
            serde_json::json!({
                "id": d.id, "name": d.name, "archetype": d.archetype,
                "hint": d.hint, "collected": count > 0, "count": count,
            })
        })
        .collect();
    let briefs: Vec<serde_json::Value> = gens
        .iter()
        .map(|g| serde_json::to_value(g).unwrap_or_default())
        .collect();
    serde_json::json!({ "slots": slots, "generations": briefs })
}

/// 图鉴面板开合:临时放大/恢复窗口。点击穿透由前端另调 set_menu_open。
#[tauri::command]
fn set_codex(open: bool, app: AppHandle, state: State<AppState>) {
    state.codex_open.store(open, Ordering::Relaxed);
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let (w, h) = if open {
        (CODEX_W, CODEX_H)
    } else {
        let s = f32::from_bits(state.ui_scale_bits.load(Ordering::Relaxed));
        window_size(s, state.collapsed.load(Ordering::Relaxed))
    };
    let _ = win.set_size(tauri::LogicalSize::new(w, h));
    if open {
        resize_and_clamp(&app);
    }
}

#[tauri::command]
fn set_ui_scale(scale: f32, app: AppHandle, state: State<AppState>) {
    let s = if scale < 0.9 {
        0.8
    } else if scale < 1.12 {
        1.0
    } else {
        1.25
    };
    {
        let mut inner = state.inner.lock().unwrap();
        inner.ui_scale = s;
    }
    state.ui_scale_bits.store(s.to_bits(), Ordering::Relaxed);
    let collapsed = state.collapsed.load(Ordering::Relaxed);
    if let Some(win) = app.get_webview_window("main") {
        let (w, h) = window_size(s, collapsed);
        let _ = win.set_size(tauri::LogicalSize::new(w, h));
    }
    resize_and_clamp(&app);
    save_now(&app, &state);
    logging::write(&format!("ui scale set to {s}"));
}

#[tauri::command]
fn set_opacity(value: f32, app: AppHandle, state: State<AppState>) {
    let v = value.clamp(0.5, 1.0);
    {
        let mut inner = state.inner.lock().unwrap();
        inner.opacity = v;
    }
    save_now(&app, &state);
    logging::write(&format!("opacity set to {v:.2}"));
}

#[tauri::command]
fn set_collapsed(value: bool, app: AppHandle, state: State<AppState>) {
    {
        let mut inner = state.inner.lock().unwrap();
        inner.collapsed = value;
    }
    state.collapsed.store(value, Ordering::Relaxed);
    let s = f32::from_bits(state.ui_scale_bits.load(Ordering::Relaxed));
    if let Some(win) = app.get_webview_window("main") {
        let (w, h) = window_size(s, value);
        let _ = win.set_size(tauri::LogicalSize::new(w, h));
    }
    resize_and_clamp(&app);
    save_now(&app, &state);
    logging::write(if value {
        "collapsed to dock"
    } else {
        "expanded from dock"
    });
}

#[tauri::command]
fn quit_app(app: AppHandle, state: State<AppState>) {
    save_now(&app, &state);
    logging::write("quit by user");
    app.exit(0);
}

#[tauri::command]
fn begin_drag(window: WebviewWindow) -> Result<(), String> {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SendMessageW, HTCAPTION, WM_NCLBUTTONDOWN,
        };
        let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
        ReleaseCapture();
        SendMessageW(hwnd as _, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
    }
    Ok(())
}

/// 切换点击穿透。
///
/// 不用 Tauri 的 `set_ignore_cursor_events`：tao 在清除该标志时会重建整个
/// 扩展样式，把 `WS_EX_LAYERED` 一起去掉，而透明 WebView2 窗口一旦失去
/// 分层样式就不再显示内容（窗口在、画面没了）。这里只改
/// `WS_EX_TRANSPARENT`，并保证 `WS_EX_LAYERED` 始终保留。
#[cfg(windows)]
fn set_click_through(win: &tauri::WebviewWindow, through: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_TRANSPARENT,
    };
    let Ok(hwnd) = win.hwnd() else {
        return;
    };
    let h = hwnd.0 as isize;
    unsafe {
        let ex = GetWindowLongPtrW(h as _, GWL_EXSTYLE) as u32;
        let mut next = ex | WS_EX_LAYERED;
        if through {
            next |= WS_EX_TRANSPARENT;
        } else {
            next &= !WS_EX_TRANSPARENT;
        }
        if next != ex {
            SetWindowLongPtrW(h as _, GWL_EXSTYLE, next as isize);
        }
    }
}

#[cfg(not(windows))]
fn set_click_through(_win: &tauri::WebviewWindow, _through: bool) {}

fn inside_dish(x: f64, y: f64, scale: f64) -> bool {
    let cx = DISH_CX * scale;
    let cy = DISH_CY * scale;
    let r = DISH_R * scale;
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= r * r
}

fn window_size(scale: f32, collapsed: bool) -> (f64, f64) {
    if collapsed {
        (DOCK_SIZE, DOCK_SIZE)
    } else {
        (WIN_W * scale as f64, WIN_H * scale as f64)
    }
}

fn resize_and_clamp(app: &AppHandle) {
    let handle = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(90));
        if let Some(win) = handle.get_webview_window("main") {
            clamp_window(&win);
        }
    });
}

/// 把窗口夹回某个显示器的可见范围内。
fn clamp_window(win: &WebviewWindow) {
    let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) else {
        return;
    };
    let monitors = win.available_monitors().unwrap_or_default();
    if monitors.is_empty() {
        return;
    }

    let area = |m: &tauri::Monitor| {
        let wa = m.work_area();
        let mp = wa.position;
        let ms = wa.size;
        let w = ((pos.x + size.width as i32).min(mp.x + ms.width as i32) - pos.x.max(mp.x)).max(0);
        let h = ((pos.y + size.height as i32).min(mp.y + ms.height as i32) - pos.y.max(mp.y)).max(0);
        w as i64 * h as i64
    };
    let best = monitors.iter().max_by_key(|m| area(m));
    let Some(mon) = best else { return };

    let wa = mon.work_area();
    let mp = wa.position;
    let ms = wa.size;
    let max_x = mp.x + ms.width as i32 - size.width as i32;
    let max_y = mp.y + ms.height as i32 - size.height as i32;
    let nx = pos.x.clamp(mp.x, max_x.max(mp.x));
    let ny = pos.y.clamp(mp.y, max_y.max(mp.y));
    if nx != pos.x || ny != pos.y {
        let _ = win.set_position(tauri::PhysicalPosition::new(nx, ny));
        logging::write(&format!(
            "window clamped from {},{} to {},{}",
            pos.x, pos.y, nx, ny
        ));
    }
}

/// 演示模式：把 1 秒压成 1 分钟，脚本化走过“专注—休息—多线—过劳”。
fn demo_sample(sim_ms: u64) -> Sample {
    let m = (sim_ms / 60_000) % 60;
    let s = (sim_ms / 1000) % 60;
    let (idle, app) = match m {
        0..=21 => (0, Some("editor")),
        22..=25 => (240_000, None),
        26..=43 => {
            let app = if (m % 18) < 9 { "editor" } else { "browser" };
            (0, Some(app))
        }
        44..=53 => {
            let apps = ["chat", "mail", "browser", "editor"];
            (0, Some(apps[((s / 9) % 4) as usize]))
        }
        54..=57 => (240_000, None),
        _ => (0, Some("meeting")),
    };
    Sample {
        idle_ms: idle,
        app: app.map(|str| str.to_string()),
        input: idle < 1500,
    }
}

fn main() {
    let demo = std::env::args().any(|a| a == "--demo");
    let data_root = store::data_root();
    logging::init(&data_root);
    logging::write(&format!(
        "start version={} demo={} exe={}",
        env!("CARGO_PKG_VERSION"),
        demo,
        std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    ));

    if already_running() {
        logging::write("another instance is running; exit");
        return;
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            set_paused,
            set_menu_open,
            set_diag,
            set_autostart,
            set_preview_stage,
            set_ui_scale,
            set_opacity,
            set_collapsed,
            set_codex,
            get_codex,
            collect_creature,
            tray_probe,
            tray_self_test,
            reset_jar,
            quit_app,
            begin_drag
        ])
        .setup(move |app| {
            let store = if demo {
                Store::at(data_root.join("demo"))
            } else {
                Store::new()
            };
            let store_root = store.dir().to_path_buf();
            let data_dir = store_root.to_string_lossy().to_string();
            let persisted = store.load();
            store.prune(14, now_ms());

            let autostart_on = persisted.autostart.unwrap_or(true);
            let mut autostart_state: u8 = 0;
            if !demo {
                if autostart_on {
                    if autostart::is_enabled() {
                        autostart_state = 1;
                    } else {
                        match autostart::set_enabled(true) {
                            Ok(()) => {
                                autostart_state = 1;
                                logging::write("autostart enabled");
                            }
                            Err(e) => {
                                autostart_state = 2;
                                logging::write(&format!("autostart blocked: {e}"));
                            }
                        }
                    }
                } else if autostart::is_enabled() {
                    let _ = autostart::set_enabled(false);
                    logging::write("autostart disabled");
                }
                if let Some(cmd) = autostart::registered_command() {
                    logging::write(&format!("autostart command: {cmd}"));
                }
            }

            let mut eco = persisted.eco.clone();
            eco.apply_offline(now_ms());
            eco.stage_scale = if demo { 0.1 } else { 1.0 };

            let ui_scale = persisted.ui_scale.unwrap_or(1.0).clamp(0.8, 1.25);
            let opacity = persisted.opacity.unwrap_or(1.0).clamp(0.5, 1.0);
            let collapsed = persisted.collapsed.unwrap_or(false);

            let today = local_date_key();
            let (day_active_sec, day_breaks) =
                if persisted.day_key.as_deref() == Some(today.as_str()) {
                    (
                        persisted.day_active_sec.unwrap_or(0),
                        persisted.day_breaks.unwrap_or(0),
                    )
                } else {
                    (0, 0)
                };
            logging::write(&format!(
                "day restore: persisted={:?} today={} active_sec={} breaks={}",
                persisted.day_key, today, day_active_sec, day_breaks
            ));

            let genome = persisted.genome.clone().unwrap_or_else(Genome::random);
            let agg = persisted.agg.clone().unwrap_or_else(|| Agg::estimated(&eco));
            let generation = persisted.generation.max(1);
            let creature_name = persisted.creature_name.clone();
            let species_id = persisted.species.as_deref().and_then(species::by_id);
            let started_on = persisted
                .started_on
                .clone()
                .unwrap_or_else(|| today.clone());
            let gen_breaks = persisted.gen_breaks.unwrap_or(0);

            let tray = build_tray(app, persisted.paused, autostart_state == 1)?;

            let state = AppState {
                inner: Mutex::new(Inner {
                    tracker: Tracker::default(),
                    sig: Signature::default(),
                    eco,
                    paused: persisted.paused,
                    demo,
                    demo_base: now_ms(),
                    demo_elapsed: 0,
                    last_save: now_ms(),
                    data_dir,
                    last_active: true,
                    last_idle_ms: 0,
                    last_fg: None,
                    started_ms: now_ms(),
                    last_wall: now_ms(),
                    day_key: today,
                    day_active_sec,
                    day_breaks,
                    autostart: autostart_on,
                    autostart_state,
                    preview_stage: None,
                    ui_scale,
                    opacity,
                    collapsed,
                    genome,
                    agg,
                    generation,
                    creature_name,
                    species: species_id,
                    started_on,
                    gen_breaks,
                    demo_bloom_sim: 0,
                }),
                menu_open: AtomicBool::new(false),
                hover_now: AtomicBool::new(false),
                diag_open: AtomicBool::new(false),
                codex_open: AtomicBool::new(false),
                ui_scale_bits: AtomicU32::new(ui_scale.to_bits()),
                collapsed: AtomicBool::new(collapsed),
                store,
                tray,
            };
            let _ = state
                .tray
                .pause
                .set_text(if persisted.paused { "继续观察" } else { "暂停观察" });
            app.manage(state);

            // 窗口在这里创建而不是写进配置，只为一件事：
            // 把 WebView2 的用户数据目录放到可写的位置。
            let win = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title("行为生态缸")
            .inner_size(WIN_W, WIN_H)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .shadow(false)
            .focused(false)
            .data_directory(store_root.join("webview2"))
            .build()?;

            set_click_through(&win, true);
            if collapsed || (ui_scale - 1.0).abs() > 0.01 {
                let (w, h) = window_size(ui_scale, collapsed);
                let _ = win.set_size(tauri::LogicalSize::new(w, h));
            }
            match (persisted.x, persisted.y) {
                (Some(x), Some(y)) => {
                    let _ = win.set_position(LogicalPosition::new(x as f64, y as f64));
                }
                _ => {
                    if let Ok(Some(mon)) = win.primary_monitor() {
                        let scale = mon.scale_factor();
                        let wa = mon.work_area();
                        let pos = wa.position;
                        let size = wa.size;
                        let x = pos.x as f64 / scale + size.width as f64 / scale - WIN_W - 24.0;
                        let y = pos.y as f64 / scale + size.height as f64 / scale - WIN_H - 8.0;
                        let _ = win.set_position(LogicalPosition::new(x.max(0.0), y.max(0.0)));
                    }
                }
            }
            clamp_window(&win);
            let at = win
                .outer_position()
                .map(|p| format!("{},{}", p.x, p.y))
                .unwrap_or_else(|_| "?".into());
            logging::write(&format!(
                "window ready at {at} data={} autostart={}",
                store_root.display(),
                autostart_on
            ));

            let handle = app.handle().clone();
            thread::spawn(move || hover_loop(handle));

            let handle = app.handle().clone();
            spawn_hotkey_thread(handle);

            let handle = app.handle().clone();
            thread::spawn(move || tick_loop(handle));

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn build_tray(
    app: &tauri::App,
    paused: bool,
    autostart_on: bool,
) -> tauri::Result<TrayHandles> {
    let show = MenuItem::with_id(app, "show", "隐藏窗口", true, None::<&str>)?;
    let pause = MenuItem::with_id(
        app,
        "pause",
        if paused { "继续观察" } else { "暂停观察" },
        true,
        None::<&str>,
    )?;
    let diag = MenuItem::with_id(app, "diag", "自检面板", true, None::<&str>)?;
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "开机自启",
        true,
        autostart_on,
        None::<&str>,
    )?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[&show, &pause, &diag, &sep1, &autostart, &sep2, &quit],
    )?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("行为生态缸")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu(app, event.id.as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_window(tray.app_handle());
            }
        });
    // 托盘图标必须显式给一张图：default_window_icon 在这种无边框窗口上不保证存在。
    match tauri::image::Image::from_bytes(include_bytes!("../icons/tray-32.png")) {
        Ok(icon) => builder = builder.icon(icon),
        Err(e) => logging::write(&format!("tray icon load failed: {e}")),
    }
    builder.build(app)?;

    // tray-icon 会静默吞掉 NIM_ADD 失败，这里直接用 Shell 探一次并记下来。
    let (tray_ok, msg) = self_test();
    TRAY_OK.store(tray_ok, Ordering::Relaxed);
    logging::write(&format!("tray self-test: {msg}"));

    Ok(TrayHandles {
        pause,
        autostart,
        show,
    })
}

fn toggle_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let visible = win.is_visible().unwrap_or(true);
    if visible {
        let _ = win.hide();
    } else {
        let _ = win.show();
        let _ = win.set_focus();
    }
    if let Some(state) = app.try_state::<AppState>() {
        let _ = state
            .tray
            .show
            .set_text(if visible { "显示窗口" } else { "隐藏窗口" });
    }
    logging::write(if visible { "window hidden" } else { "window shown" });
}

fn handle_menu(app: &AppHandle, id: &str) {
    match id {
        "show" => toggle_window(app),
        "pause" => {
            let Some(state) = app.try_state::<AppState>() else {
                return;
            };
            let paused = {
                let mut inner = state.inner.lock().unwrap();
                inner.paused = !inner.paused;
                if !inner.paused {
                    inner.eco.last_tick_ms = now_ms();
                }
                inner.paused
            };
            let _ = state
                .tray
                .pause
                .set_text(if paused { "继续观察" } else { "暂停观察" });
            logging::write(if paused {
                "paused from tray"
            } else {
                "resumed from tray"
            });
        }
        "diag" => {
            if let Some(state) = app.try_state::<AppState>() {
                let open = !state.diag_open.load(Ordering::Relaxed);
                state.diag_open.store(open, Ordering::Relaxed);
                logging::write(if open {
                    "diag opened"
                } else {
                    "diag closed"
                });
            }
        }
        "autostart" => toggle_autostart(app),
        "quit" => {
            if let Some(state) = app.try_state::<AppState>() {
                save_now(app, &state);
                logging::write("quit from tray");
                app.exit(0);
            }
        }
        _ => {}
    }
}

fn toggle_autostart(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let effective = {
        let inner = state.inner.lock().unwrap();
        inner.autostart_state == 1
    };
    apply_autostart(app, &state, !effective);
}

/// 开关自启，返回 0=关闭 1=已开启 2=被拦截。
fn apply_autostart(app: &AppHandle, state: &AppState, enabled: bool) -> u8 {
    let new_state = match autostart::set_enabled(enabled) {
        Ok(()) => {
            logging::write(if enabled {
                "autostart enabled by user"
            } else {
                "autostart disabled by user"
            });
            if enabled {
                1
            } else {
                0
            }
        }
        Err(e) => {
            logging::write(&format!("autostart toggle failed: {e}"));
            if enabled {
                2
            } else {
                0
            }
        }
    };
    {
        let mut inner = state.inner.lock().unwrap();
        inner.autostart = enabled;
        inner.autostart_state = new_state;
    }
    let _ = state.tray.autostart.set_checked(new_state == 1);
    save_now(app, state);
    new_state
}

#[cfg(windows)]
fn hover_loop(handle: AppHandle) {
    let mut last_inside = false;
    loop {
        thread::sleep(Duration::from_millis(40));
        let Some(win) = handle.get_webview_window("main") else {
            continue;
        };
        if !win.is_visible().unwrap_or(true) {
            continue;
        }
        let (menu, codex, scale, collapsed) = handle
            .try_state::<AppState>()
            .map(|s| {
                (
                    s.menu_open.load(Ordering::Relaxed),
                    s.codex_open.load(Ordering::Relaxed),
                    f32::from_bits(s.ui_scale_bits.load(Ordering::Relaxed)) as f64,
                    s.collapsed.load(Ordering::Relaxed),
                )
            })
            .unwrap_or((false, false, 1.0, false));

        let inside = if menu || codex || collapsed {
            true
        } else {
            let (Ok(pos), Ok(win_scale)) = (win.outer_position(), win.scale_factor()) else {
                continue;
            };
            let mut pt = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
            if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt) } == 0 {
                continue;
            }
            let x = (pt.x - pos.x) as f64 / win_scale;
            let y = (pt.y - pos.y) as f64 / win_scale;
            inside_dish(x, y, scale)
        };

        if inside != last_inside {
            last_inside = inside;
            set_click_through(&win, !inside);
            if let Some(state) = handle.try_state::<AppState>() {
                state.hover_now.store(inside, Ordering::Relaxed);
            }
            let _ = handle.emit("hover", inside);
        }
    }
}

#[cfg(not(windows))]
fn hover_loop(_handle: AppHandle) {}

fn tick_loop(handle: AppHandle) {
    loop {
        thread::sleep(Duration::from_millis(1000));
        tick_once(&handle);
    }
}

/// 全局快捷键 Ctrl+Alt+J：显示/隐藏培养皿。
///
/// 这台机器的安全软件挡住了 Shell_NotifyIcon（NIM_ADD 返回拒绝访问），
/// 托盘图标装不上，所以给一个不依赖托盘的入口。
#[cfg(windows)]
fn spawn_hotkey_thread(handle: AppHandle) {
    thread::spawn(move || unsafe {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            RegisterHotKey, MOD_ALT, MOD_CONTROL, VK_J,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

        let ok = RegisterHotKey(
            std::ptr::null_mut(),
            1,
            MOD_CONTROL | MOD_ALT,
            VK_J as u32,
        );
        logging::write(if ok != 0 {
            "hotkey registered: Ctrl+Alt+J"
        } else {
            "hotkey registration failed"
        });
        if ok == 0 {
            return;
        }
        loop {
            let mut msg: MSG = std::mem::zeroed();
            let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if r <= 0 {
                break;
            }
            if msg.message == WM_HOTKEY {
                toggle_window(&handle);
            }
        }
    });
}

#[cfg(not(windows))]
fn spawn_hotkey_thread(_handle: AppHandle) {}

fn tick_once(handle: &AppHandle) {
    let Some(state) = handle.try_state::<AppState>() else {
        return;
    };
    let state: &AppState = &state;

    let now = now_ms();
    let snap;
    let should_save;
    {
        let mut inner = state.inner.lock().unwrap();

        let today = local_date_key();
        if inner.day_key != today {
            inner.day_key = today.clone();
            inner.day_active_sec = 0;
            inner.day_breaks = 0;
            inner.agg.roll_day();
            logging::write("day rollover");
        }

        if !inner.paused {
            if inner.last_wall > 0 && now.saturating_sub(inner.last_wall) > 120_000 {
                let gap = now.saturating_sub(inner.last_wall) / 1000;
                logging::write(&format!("resume after {gap}s gap"));
                inner.eco.apply_offline(now);
                inner.tracker.reset_after_gap(now);
            }

            if inner.demo {
                let start = inner.demo_base;
                for _ in 0..60 {
                    inner.demo_elapsed += 1000;
                    let sim = start + inner.demo_elapsed;
                    let sample = demo_sample(sim);
                    let active = sample.idle_ms < 60_000;
                    advance(&mut inner, &state.store, sim, &sample, active);
                }
            } else {
                let sample = platform::sample();
                let active = sample.idle_ms < 60_000;
                advance(&mut inner, &state.store, now, &sample, active);
            }
            inner.last_wall = now;
        }

        // 显形:成体起尝试锁定物种;到繁盛仍不定型则取最高分兜底。
        if inner.species.is_none() && inner.eco.stage >= 2 {
            let idx = if inner.eco.stage >= 3 {
                Some(species::best(&inner.agg))
            } else {
                species::determine(&inner.agg)
            };
            if let Some(i) = idx {
                let def = &species::SPECIES[i];
                inner.species = Some(def);
                logging::write(&format!(
                    "species revealed: {} (gen {})",
                    def.id, inner.generation
                ));
            }
        }

        // 演示模式:繁盛稳定展示约 25 秒后自动收集,循环走完整个世代。
        if inner.demo && inner.eco.stage >= 3 && inner.species.is_some() {
            inner.demo_bloom_sim += 1000;
            if inner.demo_bloom_sim > 25_000 {
                let rec = perform_collect(&mut inner, now, &today);
                state.store.save_generation(&rec);
                logging::write(&format!("demo auto-collected gen {}", rec.gen));
            }
        } else {
            inner.demo_bloom_sim = 0;
        }

        snap = state.snapshot_at(&inner, now);
        should_save = now.saturating_sub(inner.last_save) > 30_000;
    }

    let _ = handle.emit("state", snap);
    if should_save {
        save_now(handle, state);
    }
}

fn advance(inner: &mut Inner, store: &Store, now: u64, sample: &Sample, active: bool) {
    for ev in inner.tracker.step(now, sample) {
        match ev {
            Event::MinuteRolled => {
                if !inner.demo {
                    if let Some(b) = inner.tracker.buckets.last() {
                        store.append_bucket(b);
                    }
                }
            }
            Event::Break => {
                inner.eco.last_break_ms = now;
                inner.day_breaks += 1;
                inner.gen_breaks += 1;
            }
        }
    }

    inner.last_idle_ms = sample.idle_ms;
    inner.last_fg = sample.app.clone();
    inner.sig = inner.tracker.signature(now);
    inner.last_active = active;
    if active {
        inner.day_active_sec += 1;
        inner.agg.tick(&inner.sig, local_hour());
    }
    inner
        .eco
        .tick(&inner.sig, 1.0, if active { 1.0 } else { 0.0 }, now);
}

fn save_now(handle: &AppHandle, state: &AppState) {
    let mut inner = state.inner.lock().unwrap();
    let win = handle.get_webview_window("main");
    let scale = win
        .as_ref()
        .and_then(|w| w.scale_factor().ok())
        .unwrap_or(1.0);
    let (x, y) = win
        .as_ref()
        .and_then(|w| w.outer_position().ok())
        .map(|p| {
            (
                (p.x as f64 / scale).round() as i32,
                (p.y as f64 / scale).round() as i32,
            )
        })
        .unwrap_or((0, 0));
    state.store.save(&store::Persisted {
        eco: inner.eco.clone(),
        paused: inner.paused,
        x: Some(x),
        y: Some(y),
        autostart: Some(inner.autostart),
        day_key: Some(inner.day_key.clone()),
        day_active_sec: Some(inner.day_active_sec),
        day_breaks: Some(inner.day_breaks),
        ui_scale: Some(inner.ui_scale),
        opacity: Some(inner.opacity),
        collapsed: Some(inner.collapsed),
        schema: 2,
        generation: inner.generation,
        genome: Some(inner.genome.clone()),
        agg: Some(inner.agg.clone()),
        creature_name: inner.creature_name.clone(),
        species: inner.species.map(|d| d.id.to_string()),
        started_on: Some(inner.started_on.clone()),
        gen_breaks: Some(inner.gen_breaks),
    });
    inner.last_save = now_ms();
}

#[cfg(windows)]
mod platform {
    use super::Sample;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };

    pub fn sample() -> Sample {
        let idle = idle_ms();
        Sample {
            idle_ms: idle,
            app: foreground_app(),
            input: idle < 1500,
        }
    }

    fn idle_ms() -> u64 {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            dwTime: 0,
        };
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return 0;
        }
        let tick = unsafe { GetTickCount64() };
        tick.saturating_sub(info.dwTime as u64)
    }

    fn foreground_app() -> Option<String> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == 0 {
            return None;
        }
        // 自己获得焦点不算一次“工作切换”。
        if pid == std::process::id() {
            return None;
        }
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len) };
        unsafe { CloseHandle(handle) };
        if ok == 0 || len == 0 {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::Sample;
    pub fn sample() -> Sample {
        Sample {
            idle_ms: 0,
            app: None,
            input: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inner_at_bloom() -> Inner {
        let mut inner = Inner {
            tracker: Tracker::default(),
            eco: Eco::default(),
            sig: Signature::default(),
            paused: false,
            demo: false,
            demo_base: 0,
            demo_elapsed: 0,
            last_save: 0,
            data_dir: String::new(),
            last_active: true,
            last_idle_ms: 0,
            last_fg: None,
            started_ms: 0,
            last_wall: 0,
            day_key: "2026-10-25".into(),
            day_active_sec: 0,
            day_breaks: 0,
            autostart: false,
            autostart_state: 0,
            preview_stage: None,
            ui_scale: 1.0,
            opacity: 1.0,
            collapsed: false,
            genome: Genome::from_seed(42),
            agg: Agg::neutral(),
            generation: 1,
            creature_name: None,
            species: Some(&species::SPECIES[0]),
            started_on: "2026-10-20".into(),
            gen_breaks: 5,
            demo_bloom_sim: 0,
        };
        inner.eco.active_minutes = 950.0;
        inner.eco.stage = 3;
        inner
    }

    #[test]
    fn collect_archives_and_renews() {
        let mut inner = inner_at_bloom();
        let before_seed = inner.genome.seed;
        let rec = perform_collect(&mut inner, 1_000_000, "2026-10-25");
        assert_eq!(rec.gen, 1);
        assert_eq!(rec.species_id, "deep_glow");
        assert!(rec.full_bloom);
        assert!(rec.marks.iter().any(|m| m == "初代"));
        assert_eq!(rec.from, "2026-10-20");
        assert_eq!(rec.to, "2026-10-25");
        assert_eq!(rec.breaks, 5);
        assert!((rec.active_min - 950.0).abs() < 0.001);

        // 下一代是新孢子
        assert_eq!(inner.generation, 2);
        assert!(inner.species.is_none());
        assert_eq!(inner.eco.active_minutes, 0.0);
        assert_eq!(inner.eco.stage, 0);
        assert_eq!(inner.started_on, "2026-10-25");
        assert_eq!(inner.gen_breaks, 0);
        assert_ne!(inner.genome.seed, before_seed);
    }

    #[test]
    fn early_collect_marks_and_keeps_species() {
        let mut inner = inner_at_bloom();
        inner.eco.active_minutes = 320.0;
        inner.eco.stage = 2;
        inner.generation = 4;
        let rec = perform_collect(&mut inner, 1_000_000, "2026-10-25");
        assert!(!rec.full_bloom);
        assert!(rec.marks.iter().any(|m| m == "提前收集"));
        assert!(rec.marks.iter().all(|m| m != "初代"));
        assert_eq!(rec.species_id, "deep_glow");
        assert_eq!(inner.generation, 5);
    }
}
