const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const ALERT_PRAYERS = [
  ["fajr", "Fajr"],
  ["dhuhr", "Dhuhr"],
  ["asr", "Asr"],
  ["maghrib", "Maghrib"],
  ["isha", "Isha"],
];

let snap = null;

// ------------------------------------------------------------ formatting

function clock(ms) {
  return new Date(ms).toLocaleTimeString([], {
    hour: "numeric",
    minute: "2-digit",
    hour12: !snap.settings.hour24,
  });
}

function countdown(ms) {
  const total = Math.max(0, Math.ceil((ms - Date.now()) / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n) => String(n).padStart(2, "0");
  return `${h}:${pad(m)}:${pad(s)}`;
}

function compassPoint(deg) {
  const points = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
  return points[Math.round(deg / 45) % 8];
}

function dateLine() {
  const now = new Date();
  const gregorian = now.toLocaleDateString([], { weekday: "long", day: "numeric", month: "long" });
  let hijri = "";
  try {
    hijri = new Intl.DateTimeFormat("en-u-ca-islamic-umalqura", {
      day: "numeric",
      month: "long",
      year: "numeric",
    }).format(now);
  } catch {
    // Calendar not available in this WebKit build; show the Gregorian date only.
  }
  return hijri ? `${gregorian} · ${hijri}` : gregorian;
}

// ----------------------------------------------------------------- today

function renderToday() {
  const s = snap.settings;
  $("setup").hidden = s.configured;
  $("today-body").hidden = !s.configured;
  $("place").textContent = s.configured ? s.locationName || "Custom location" : "Miqat";
  $("dates").textContent = dateLine();
  if (!s.configured) return;

  const list = $("prayers");
  list.replaceChildren();
  const now = Date.now();
  for (const p of snap.prayers) {
    const li = document.createElement("li");
    if (p.key === "sunrise") li.classList.add("minor");
    if (p.time <= now) li.classList.add("past");
    if (p.key === snap.current && p.time <= now) li.classList.add("current");
    if (snap.next && p.key === snap.next.key && p.time === snap.next.time) li.classList.add("next");

    const label = document.createElement("span");
    label.className = "label";
    label.textContent = p.label;
    if (s.muted.includes(p.key)) {
      const off = document.createElement("span");
      off.className = "bell-off";
      off.textContent = "muted";
      label.append(off);
    }
    const time = document.createElement("span");
    time.className = "time";
    time.textContent = clock(p.time);
    li.append(label, time);
    list.append(li);
  }

  $("needle").style.transform = `rotate(${snap.qibla}deg)`;
  $("qibla-deg").textContent = `${Math.round(snap.qibla)}° ${compassPoint(snap.qibla)}`;
  renderCountdown();
}

function renderCountdown() {
  if (!snap || !snap.next) return;
  if (snap.next.time <= Date.now()) {
    refresh();
    return;
  }
  $("next-name").textContent = snap.next.label;
  $("countdown").textContent = countdown(snap.next.time);
  $("next-at").textContent = `at ${clock(snap.next.time)}`;
}

async function refresh() {
  snap = await invoke("get_snapshot");
  renderToday();
}

// -------------------------------------------------------------- settings

function showView(name) {
  $("view-today").hidden = name !== "today";
  $("view-settings").hidden = name !== "settings";
  if (name === "settings") fillSettings();
}

function adhanLabel(path) {
  return path ? path.split("/").pop() : "Built-in adhan";
}

function fillSettings() {
  const s = snap.settings;
  $("loc-name").value = s.locationName;
  $("lat").value = s.configured ? s.latitude : "";
  $("lng").value = s.configured ? s.longitude : "";
  $("method").value = s.method;
  $("madhab").value = s.madhab;
  $("notifications").checked = s.notifications;
  $("adhan").checked = s.adhan;
  $("adhan-file").textContent = adhanLabel(s.adhanPath);
  $("adhan-file").title = s.adhanPath || "";
  $("reminder").value = String(s.reminderMinutes);
  $("menubar-mode").value = s.menubarMode;
  $("hour24").checked = s.hour24;
  $("autostart").checked = s.autostart;
  $("auto-location").checked = s.autoLocation;
  renderMutes();
}

function renderMutes() {
  const box = $("mutes");
  box.replaceChildren();
  for (const [key, label] of ALERT_PRAYERS) {
    const btn = document.createElement("button");
    const on = !snap.settings.muted.includes(key);
    btn.textContent = label;
    btn.className = on ? "on" : "";
    btn.title = on ? `Alerts on for ${label}` : `Alerts off for ${label}`;
    btn.setAttribute("aria-pressed", String(on));
    btn.addEventListener("click", () => {
      const muted = on
        ? [...snap.settings.muted, key]
        : snap.settings.muted.filter((k) => k !== key);
      save({ muted }).then(renderMutes);
    });
    box.append(btn);
  }
}

function setStatus(id, text, isError = false) {
  const el = $(id);
  el.textContent = text;
  el.classList.toggle("error", isError);
}

async function save(patch) {
  try {
    snap = await invoke("save_settings", { settings: { ...snap.settings, ...patch } });
    setStatus("save-status", "");
    renderToday();
    return true;
  } catch (err) {
    setStatus("save-status", String(err), true);
    fillSettings();
    return false;
  }
}

async function saveLocation(name, latitude, longitude) {
  if (!Number.isFinite(latitude) || !Number.isFinite(longitude)) {
    setStatus("loc-status", "Enter both latitude and longitude.", true);
    return;
  }
  if (Math.abs(latitude) > 90 || Math.abs(longitude) > 180) {
    setStatus("loc-status", "Latitude must be within ±90 and longitude within ±180.", true);
    return;
  }
  // A hand-picked place must not be replaced by the next background check.
  const ok = await save({ configured: true, locationName: name, latitude, longitude, autoLocation: false });
  if (ok) {
    fillSettings();
    setStatus("loc-status", usingLine(snap.settings));
  }
}

async function searchCity() {
  const query = $("city-query").value.trim();
  const list = $("city-results");
  list.replaceChildren();
  if (!query) return;
  setStatus("loc-status", "Searching…");
  try {
    const url = `https://geocoding-api.open-meteo.com/v1/search?count=6&language=en&name=${encodeURIComponent(query)}`;
    const res = await fetch(url);
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const places = (await res.json()).results || [];
    setStatus("loc-status", places.length ? "" : "No matching city found.");
    for (const p of places) {
      const li = document.createElement("li");
      const region = [p.admin1, p.country].filter(Boolean).join(", ");
      li.textContent = p.name;
      if (region) {
        const small = document.createElement("small");
        small.textContent = ` — ${region}`;
        li.append(small);
      }
      li.addEventListener("click", () => {
        list.replaceChildren();
        $("city-query").value = "";
        saveLocation(p.name, p.latitude, p.longitude);
      });
      list.append(li);
    }
  } catch (err) {
    setStatus("loc-status", `City search failed (${err.message}). Enter coordinates instead.`, true);
  }
}

function usingLine(s) {
  return `Using ${s.locationName || "custom location"} (${s.latitude.toFixed(4)}, ${s.longitude.toFixed(4)}).`;
}

async function detectLocation(statusId) {
  setStatus(statusId, "Detecting…");
  try {
    snap = await invoke("detect_location");
    renderToday();
    fillSettings();
    setStatus(statusId, statusId === "loc-status" ? usingLine(snap.settings) : "");
  } catch (err) {
    setStatus(statusId, `Could not detect your location (${err}). Search for your city instead.`, true);
  }
}

async function setAutoLocation(on) {
  if (!(await save({ autoLocation: on }))) return;
  setStatus(
    "loc-status",
    on ? "Checked every 30 minutes from your internet connection. A VPN will show the VPN's city." : "",
  );
}

function manualLocation() {
  saveLocation($("loc-name").value.trim(), parseFloat($("lat").value), parseFloat($("lng").value));
}

// ---------------------------------------------------------------- wiring

$("open-settings").addEventListener("click", () => showView("settings"));
$("setup-btn").addEventListener("click", () => showView("settings"));
$("close-settings").addEventListener("click", () => showView("today"));

$("city-search").addEventListener("click", searchCity);
$("city-query").addEventListener("keydown", (e) => {
  if (e.key === "Enter") searchCity();
});
$("detect").addEventListener("click", () => detectLocation("loc-status"));
$("setup-detect").addEventListener("click", () => detectLocation("setup-status"));
$("auto-location").addEventListener("change", (e) => setAutoLocation(e.target.checked));
$("lat").addEventListener("change", manualLocation);
$("lng").addEventListener("change", manualLocation);
$("loc-name").addEventListener("change", () => {
  if (snap.settings.configured) save({ locationName: $("loc-name").value.trim() });
});

$("method").addEventListener("change", (e) => save({ method: e.target.value }));
$("madhab").addEventListener("change", (e) => save({ madhab: e.target.value }));
$("notifications").addEventListener("change", (e) => save({ notifications: e.target.checked }));
$("adhan").addEventListener("change", (e) => save({ adhan: e.target.checked }));
$("reminder").addEventListener("change", (e) => save({ reminderMinutes: Number(e.target.value) }));
$("menubar-mode").addEventListener("change", (e) => save({ menubarMode: e.target.value }));
$("hour24").addEventListener("change", (e) => save({ hour24: e.target.checked }));
$("autostart").addEventListener("change", (e) => save({ autostart: e.target.checked }));

$("choose-adhan").addEventListener("click", async () => {
  const path = await invoke("pick_adhan");
  if (path && (await save({ adhanPath: path }))) fillSettings();
});
$("reset-adhan").addEventListener("click", async () => {
  if (await save({ adhanPath: null })) fillSettings();
});
function setPlaying(playing) {
  $("stop-adhan").hidden = !playing;
  $("preview-adhan").textContent = playing ? "Stop" : "Play";
}
$("preview-adhan").addEventListener("click", () => {
  const playing = $("preview-adhan").textContent === "Stop";
  invoke(playing ? "stop_adhan" : "preview_adhan");
  setPlaying(!playing);
});
$("stop-adhan").addEventListener("click", () => {
  invoke("stop_adhan");
  setPlaying(false);
});
$("quit").addEventListener("click", () => invoke("quit_app"));

document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && !$("view-settings").hidden) showView("today");
});
document.addEventListener("contextmenu", (e) => e.preventDefault());

listen("schedule-changed", refresh);
listen("popover-shown", refresh);
listen("adhan-started", () => {
  if (snap.settings.adhan) setPlaying(true);
});

setInterval(renderCountdown, 1000);
refresh();
