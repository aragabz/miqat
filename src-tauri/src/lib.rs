use std::{
    fs,
    path::PathBuf,
    process::{Child, Command},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    thread,
    time::{Duration as StdDuration, Instant},
};

use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use salah::prelude::{Configuration, Coordinates, Madhab, Method, Prayer, PrayerTimes, Qiblah};
use serde::{Deserialize, Serialize};
use tauri::{
    image::Image,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, Rect, State, WindowEvent,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as _};
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_notification::NotificationExt as _;

const TRAY_ID: &str = "main";
const WINDOW: &str = "main";
const FALLBACK_SOUND: &str = "/System/Library/Sounds/Glass.aiff";
/// Bundled recording, relative to the app's resource directory.
const BUILTIN_ADHAN: &str = "resources/adhan.mp3";
/// The menu bar text turns red once the next prayer is this close.
const URGENT_SECONDS: i64 = 15 * 60;

/// The five prayers that get alerts, plus sunrise which is display-only.
const DISPLAYED: [(Prayer, &str, &str); 6] = [
    (Prayer::Fajr, "fajr", "Fajr"),
    (Prayer::Sunrise, "sunrise", "Sunrise"),
    (Prayer::Dhuhr, "dhuhr", "Dhuhr"),
    (Prayer::Asr, "asr", "Asr"),
    (Prayer::Maghrib, "maghrib", "Maghrib"),
    (Prayer::Isha, "isha", "Isha"),
];

// ---------------------------------------------------------------- settings

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default, rename_all = "camelCase")]
struct Settings {
    configured: bool,
    location_name: String,
    latitude: f64,
    longitude: f64,
    method: String,
    madhab: String,
    /// "countdown" | "time" | "icon"
    menubar_mode: String,
    hour24: bool,
    notifications: bool,
    adhan: bool,
    adhan_path: Option<String>,
    /// Keys of prayers that should stay silent.
    muted: Vec<String>,
    /// Minutes before each prayer to post a reminder; 0 disables it.
    reminder_minutes: u32,
    autostart: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            configured: false,
            location_name: String::new(),
            latitude: 0.0,
            longitude: 0.0,
            method: "MuslimWorldLeague".into(),
            madhab: "shafi".into(),
            menubar_mode: "countdown".into(),
            hour24: false,
            notifications: true,
            adhan: true,
            adhan_path: None,
            muted: Vec::new(),
            reminder_minutes: 0,
            autostart: false,
        }
    }
}

fn parse_method(name: &str) -> Method {
    match name {
        "Egyptian" => Method::Egyptian,
        "Karachi" => Method::Karachi,
        "UmmAlQura" => Method::UmmAlQura,
        "Dubai" => Method::Dubai,
        "MoonsightingCommittee" => Method::MoonsightingCommittee,
        "NorthAmerica" => Method::NorthAmerica,
        "Kuwait" => Method::Kuwait,
        "Qatar" => Method::Qatar,
        "Singapore" => Method::Singapore,
        "Tehran" => Method::Tehran,
        "Turkey" => Method::Turkey,
        _ => Method::MuslimWorldLeague,
    }
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("settings.json"))
}

fn load_settings(app: &AppHandle) -> Settings {
    settings_path(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn store_settings(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app).ok_or("no config directory")?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

// ------------------------------------------------------------------- state

struct AppState {
    settings: Mutex<Settings>,
    player: Mutex<Option<Child>>,
    /// The bundled adhan recording, when it could be located.
    builtin_adhan: Option<PathBuf>,
    /// Keeps the popover open while a native dialog has focus.
    pinned: AtomicBool,
    hidden_at: Mutex<Instant>,
}

// ------------------------------------------------------------- calculation

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PrayerRow {
    key: &'static str,
    label: &'static str,
    /// Unix milliseconds.
    time: i64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    settings: Settings,
    prayers: Vec<PrayerRow>,
    next: Option<PrayerRow>,
    /// Key of the prayer whose time we are currently in, if any.
    current: Option<&'static str>,
    /// Degrees clockwise from true north.
    qibla: f64,
}

fn day_times(settings: &Settings, date: NaiveDate) -> Vec<(usize, DateTime<Utc>)> {
    let madhab = if settings.madhab == "hanafi" {
        Madhab::Hanafi
    } else {
        Madhab::Shafi
    };
    let params = Configuration::with(parse_method(&settings.method), madhab);
    let coords = Coordinates::new(settings.latitude, settings.longitude);
    let times = PrayerTimes::new(date, coords, params);
    DISPLAYED
        .iter()
        .enumerate()
        .map(|(i, (prayer, _, _))| (i, times.time(*prayer)))
        .collect()
}

fn row(index: usize, time: DateTime<Utc>) -> PrayerRow {
    PrayerRow {
        key: DISPLAYED[index].1,
        label: DISPLAYED[index].2,
        time: time.timestamp_millis(),
    }
}

/// Prayer instants (sunrise excluded) for yesterday, today and tomorrow, in order.
fn nearby_prayers(settings: &Settings, today: NaiveDate) -> Vec<(usize, DateTime<Utc>)> {
    let mut all = Vec::new();
    for offset in -1..=1 {
        let date = today + Duration::days(offset);
        all.extend(day_times(settings, date).into_iter().filter(|(i, _)| *i != 1));
    }
    all.sort_by_key(|(_, t)| *t);
    all
}

fn snapshot(settings: &Settings, now: DateTime<Utc>) -> Snapshot {
    if !settings.configured {
        return Snapshot {
            settings: settings.clone(),
            prayers: Vec::new(),
            next: None,
            current: None,
            qibla: 0.0,
        };
    }
    let today = now.with_timezone(&Local).date_naive();
    let nearby = nearby_prayers(settings, today);
    let next = nearby.iter().find(|(_, t)| *t > now).map(|(i, t)| row(*i, *t));
    let current = nearby
        .iter()
        .rev()
        .find(|(_, t)| *t <= now)
        .map(|(i, _)| DISPLAYED[*i].1);
    Snapshot {
        settings: settings.clone(),
        prayers: day_times(settings, today)
            .into_iter()
            .map(|(i, t)| row(i, t))
            .collect(),
        next,
        current,
        qibla: Qiblah::new(Coordinates::new(settings.latitude, settings.longitude)).value(),
    }
}

fn format_clock(time: DateTime<Utc>, hour24: bool) -> String {
    let local = time.with_timezone(&Local);
    if hour24 {
        local.format("%H:%M").to_string()
    } else {
        local.format("%-I:%M %p").to_string()
    }
}

/// The menu bar text, and whether the next prayer is close enough to show it in red.
fn tray_title(settings: &Settings, now: DateTime<Utc>) -> (String, bool) {
    if !settings.configured {
        return ("Set location".into(), false);
    }
    if settings.menubar_mode == "icon" {
        return (String::new(), false);
    }
    let today = now.with_timezone(&Local).date_naive();
    let Some((index, time)) = nearby_prayers(settings, today).into_iter().find(|(_, t)| *t > now)
    else {
        return (String::new(), false);
    };
    let label = DISPLAYED[index].2;
    let seconds = (time - now).num_seconds();
    let urgent = seconds <= URGENT_SECONDS;
    if settings.menubar_mode == "time" {
        return (format!("{label} {}", format_clock(time, settings.hour24)), urgent);
    }
    // Round up so the title reads "1m" until the prayer actually starts.
    let minutes = (seconds + 59) / 60;
    let title = if minutes >= 60 {
        format!("{label} in {}h {:02}m", minutes / 60, minutes % 60)
    } else {
        format!("{label} in {minutes}m")
    };
    (title, urgent)
}

// ------------------------------------------------------------------ alerts

fn stop_player(state: &AppState) {
    if let Some(mut child) = state.player.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn play_adhan(state: &AppState, settings: &Settings) {
    stop_player(state);
    let path = settings
        .adhan_path
        .as_deref()
        .map(std::path::Path::new)
        .filter(|p| p.exists())
        .or(state.builtin_adhan.as_deref())
        .unwrap_or(std::path::Path::new(FALLBACK_SOUND));
    if let Ok(child) = Command::new("/usr/bin/afplay").arg(path).spawn() {
        *state.player.lock().unwrap() = Some(child);
    }
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// Fires alerts for every prayer whose time fell inside `(prev, now]`.
fn fire_alerts(app: &AppHandle, settings: &Settings, prev: DateTime<Utc>, now: DateTime<Utc>) {
    // After sleep or a clock change the window can be huge; never announce stale prayers.
    if now - prev > Duration::seconds(120) || now <= prev {
        return;
    }
    let state = app.state::<AppState>();
    let today = now.with_timezone(&Local).date_naive();
    let lead = Duration::minutes(settings.reminder_minutes as i64);
    for (index, time) in nearby_prayers(settings, today) {
        let (_, key, label) = DISPLAYED[index];
        if settings.muted.iter().any(|m| m == key) {
            continue;
        }
        if time > prev && time <= now {
            if settings.notifications {
                let place = if settings.location_name.is_empty() {
                    String::new()
                } else {
                    format!(" in {}", settings.location_name)
                };
                notify(app, &format!("{label}"), &format!("It is time for {label}{place}."));
            }
            if settings.adhan {
                play_adhan(&state, settings);
            }
            let _ = app.emit("adhan-started", key);
        }
        let reminder = time - lead;
        if settings.reminder_minutes > 0
            && settings.notifications
            && reminder > prev
            && reminder <= now
        {
            notify(
                app,
                &format!("{label} in {} minutes", settings.reminder_minutes),
                &format!("{label} is at {}.", format_clock(time, settings.hour24)),
            );
        }
    }
}

/// Recolours the status item's text. `set_title` always resets it to the default colour.
#[cfg(target_os = "macos")]
fn paint_title_red(tray: &tauri::tray::TrayIcon, title: String) {
    use objc2::{rc::Retained, runtime::AnyObject, MainThreadMarker};
    use objc2_app_kit::{NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName};
    use objc2_foundation::{NSAttributedString, NSDictionary, NSString};

    let _ = tray.with_inner_tray_icon(move |inner| {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(button) = inner.ns_status_item().and_then(|item| item.button(mtm)) else {
            return;
        };
        let font = button.font().unwrap_or_else(|| NSFont::menuBarFontOfSize(0.0));
        let color = NSColor::systemRedColor();
        let font: Retained<AnyObject> = Retained::into_super(Retained::into_super(font));
        let color: Retained<AnyObject> = Retained::into_super(Retained::into_super(color));
        unsafe {
            let attrs = NSDictionary::from_retained_objects(
                &[NSForegroundColorAttributeName, NSFontAttributeName],
                &[color, font],
            );
            let text = NSAttributedString::initWithString_attributes(
                mtm.alloc(),
                &NSString::from_str(&title),
                Some(&attrs),
            );
            button.setAttributedTitle(&text);
        }
    });
}

fn refresh_tray(app: &AppHandle, settings: &Settings, last: &mut (String, bool)) {
    let current = tray_title(settings, Utc::now());
    if current != *last {
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let (title, urgent) = &current;
            let _ = tray.set_title(if title.is_empty() { None } else { Some(title) });
            #[cfg(target_os = "macos")]
            if *urgent && !title.is_empty() {
                paint_title_red(&tray, title.clone());
            }
        }
        *last = current;
    }
}

fn start_ticker(app: AppHandle) {
    thread::spawn(move || {
        let mut prev = Utc::now();
        let mut last_title = (String::from("\u{0}"), false);
        let mut last_marker = String::new();
        loop {
            let now = Utc::now();
            let settings = app.state::<AppState>().settings.lock().unwrap().clone();
            refresh_tray(&app, &settings, &mut last_title);
            if settings.configured {
                fire_alerts(&app, &settings, prev, now);
                // Tell the popover to reload when the next prayer or the date changes.
                let snap = snapshot(&settings, now);
                let marker = format!(
                    "{:?}{}",
                    snap.next.as_ref().map(|n| n.time),
                    now.with_timezone(&Local).date_naive()
                );
                if marker != last_marker {
                    let _ = app.emit("schedule-changed", ());
                    last_marker = marker;
                }
            }
            prev = now;
            thread::sleep(StdDuration::from_secs(1));
        }
    });
}

// ----------------------------------------------------------------- popover

fn toggle_popover(app: &AppHandle, rect: Rect) {
    let Some(window) = app.get_webview_window(WINDOW) else {
        return;
    };
    let state = app.state::<AppState>();
    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
        return;
    }
    // Clicking the tray icon blurs the popover first, which already hid it;
    // without this the same click would immediately reopen it.
    if state.hidden_at.lock().unwrap().elapsed() < StdDuration::from_millis(250) {
        return;
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    let icon_pos = rect.position.to_physical::<f64>(scale);
    let icon_size = rect.size.to_physical::<f64>(scale);
    let width = window.outer_size().map(|s| s.width as f64).unwrap_or(340.0 * scale);
    let mut x = icon_pos.x + icon_size.width / 2.0 - width / 2.0;
    let y = icon_pos.y + icon_size.height + 6.0 * scale;
    if let Ok(Some(monitor)) = app.monitor_from_point(icon_pos.x, icon_pos.y) {
        let left = monitor.position().x as f64 + 8.0 * scale;
        let right = monitor.position().x as f64 + monitor.size().width as f64 - width - 8.0 * scale;
        x = x.min(right).max(left);
    }
    let _ = window.set_position(PhysicalPosition::new(x, y));
    let _ = window.show();
    let _ = window.set_focus();
    let _ = app.emit("popover-shown", ());
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_snapshot(state: State<AppState>) -> Snapshot {
    snapshot(&state.settings.lock().unwrap(), Utc::now())
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> Result<Snapshot, String> {
    if settings.configured
        && !(settings.latitude.abs() <= 90.0 && settings.longitude.abs() <= 180.0)
    {
        return Err("Latitude must be within ±90 and longitude within ±180.".into());
    }
    store_settings(&app, &settings)?;
    let launcher = app.autolaunch();
    if settings.autostart != launcher.is_enabled().unwrap_or(false) {
        let result = if settings.autostart {
            launcher.enable()
        } else {
            launcher.disable()
        };
        result.map_err(|e| format!("Could not change launch at login: {e}"))?;
    }
    *state.settings.lock().unwrap() = settings.clone();
    let _ = app.emit("schedule-changed", ());
    Ok(snapshot(&settings, Utc::now()))
}

#[tauri::command]
async fn pick_adhan(app: AppHandle) -> Option<String> {
    let state = app.state::<AppState>();
    state.pinned.store(true, Ordering::SeqCst);
    let picked = app
        .dialog()
        .file()
        .set_title("Choose an adhan recording")
        .add_filter("Audio", &["mp3", "m4a", "aac", "wav", "aiff", "aif", "caf"])
        .blocking_pick_file()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned());
    state.pinned.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.show();
        let _ = window.set_focus();
    }
    picked
}

#[tauri::command]
fn preview_adhan(state: State<AppState>) {
    let settings = state.settings.lock().unwrap().clone();
    play_adhan(&state, &settings);
}

#[tauri::command]
fn stop_adhan(state: State<AppState>) {
    stop_player(&state);
}

#[tauri::command]
fn quit_app(app: AppHandle, state: State<AppState>) {
    stop_player(&state);
    app.exit(0);
}

// --------------------------------------------------------------------- run

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            save_settings,
            pick_adhan,
            preview_adhan,
            stop_adhan,
            quit_app
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::Focused(false) = event {
                let state = window.state::<AppState>();
                if !state.pinned.load(Ordering::SeqCst) {
                    let _ = window.hide();
                    *state.hidden_at.lock().unwrap() = Instant::now();
                }
            }
        })
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();
            let settings = load_settings(&handle);
            let (title, _) = tray_title(&settings, Utc::now());
            let builtin_adhan = handle
                .path()
                .resolve(BUILTIN_ADHAN, tauri::path::BaseDirectory::Resource)
                .ok()
                .filter(|p| p.exists());
            app.manage(AppState {
                settings: Mutex::new(settings),
                player: Mutex::new(None),
                builtin_adhan,
                pinned: AtomicBool::new(false),
                hidden_at: Mutex::new(Instant::now()),
            });

            let icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
            TrayIconBuilder::with_id(TRAY_ID)
                .icon(icon)
                .icon_as_template(true)
                .title(title)
                .tooltip("Miqat")
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        rect,
                        ..
                    } = event
                    {
                        toggle_popover(tray.app_handle(), rect);
                    }
                })
                .build(app)?;

            start_ticker(handle);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Miqat");
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn vienna() -> Settings {
        Settings {
            configured: true,
            location_name: "Vienna".into(),
            latitude: 48.2082,
            longitude: 16.3738,
            ..Settings::default()
        }
    }

    #[test]
    fn day_is_ordered_and_qibla_points_south_east() {
        let now = Utc.with_ymd_and_hms(2026, 11, 13, 9, 0, 0).unwrap();
        let snap = snapshot(&vienna(), now);
        let times: Vec<i64> = snap.prayers.iter().map(|p| p.time).collect();
        assert_eq!(times.len(), 6);
        assert!(times.windows(2).all(|w| w[0] < w[1]), "{times:?}");
        // Solar noon in Vienna in mid-November is about 10:39 UTC.
        let dhuhr = Utc.timestamp_millis_opt(times[2]).unwrap();
        assert_eq!(dhuhr.format("%H").to_string(), "10", "{dhuhr}");
        assert_eq!(snap.next.as_ref().unwrap().key, "dhuhr");
        assert_eq!(snap.current, Some("fajr"));
        assert!((snap.qibla - 136.0).abs() < 3.0, "{}", snap.qibla);
    }

    #[test]
    fn after_isha_the_next_prayer_is_tomorrows_fajr() {
        let now = Utc.with_ymd_and_hms(2026, 11, 13, 21, 30, 0).unwrap();
        let snap = snapshot(&vienna(), now);
        let next = snap.next.unwrap();
        assert_eq!(next.key, "fajr");
        assert!(next.time > now.timestamp_millis());
        assert_eq!(snap.current, Some("isha"));
    }

    #[test]
    fn countdown_title_rounds_up() {
        let settings = vienna();
        let now = Utc.with_ymd_and_hms(2026, 11, 13, 9, 0, 0).unwrap();
        let (title, urgent) = tray_title(&settings, now);
        assert!(title.starts_with("Dhuhr in 1h "), "{title}");
        assert!(!urgent);
        assert_eq!(tray_title(&Settings::default(), now), ("Set location".into(), false));
    }

    #[test]
    fn title_turns_urgent_at_fifteen_minutes() {
        let settings = vienna();
        let start = Utc.with_ymd_and_hms(2026, 11, 13, 9, 0, 0).unwrap();
        let dhuhr = Utc.timestamp_millis_opt(snapshot(&settings, start).next.unwrap().time).unwrap();
        let at = |secs: i64| tray_title(&settings, dhuhr - Duration::seconds(secs));
        assert_eq!(at(15 * 60 + 1), ("Dhuhr in 16m".into(), false));
        assert_eq!(at(15 * 60), ("Dhuhr in 15m".into(), true));
        assert_eq!(at(30), ("Dhuhr in 1m".into(), true));
        let timed = Settings { menubar_mode: "time".into(), ..vienna() };
        assert!(tray_title(&timed, dhuhr - Duration::seconds(60)).1);
    }
}
