import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface Settings {
  show_session: boolean;
  show_weekly: boolean;
  show_fable: boolean;
  refresh_interval_secs: number;
}

interface MeterDto {
  utilization: number;
  pct_label: string;
  reset_label: string | null;
  resets_at_epoch_secs: number | null;
}

interface UsageDto {
  session: MeterDto | null;
  weekly: MeterDto | null;
  weekly_fable: MeterDto | null;
  fetched_at_epoch_secs: number | null;
  status: string;
  status_message: string;
}

let showSessionEl: HTMLInputElement;
let showWeeklyEl: HTMLInputElement;
let showFableEl: HTMLInputElement;
let fableHintEl: HTMLElement;
let refreshIntervalEl: HTMLSelectElement;
let statusMessageEl: HTMLElement;
let lastUpdatedEl: HTMLElement;
let meterListEl: HTMLElement;
let refreshNowBtn: HTMLButtonElement;

let savingFromUi = false;

function renderUsage(dto: UsageDto) {
  statusMessageEl.textContent = dto.status_message;
  statusMessageEl.classList.toggle("error", dto.status !== "ok");

  lastUpdatedEl.textContent = dto.fetched_at_epoch_secs
    ? `Last updated: ${new Date(dto.fetched_at_epoch_secs * 1000).toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
      })}`
    : "";

  meterListEl.innerHTML = "";
  const rows: [string, MeterDto | null][] = [
    ["Session", dto.session],
    ["Weekly", dto.weekly],
    ["Weekly (Fable)", dto.weekly_fable],
  ];
  for (const [label, meter] of rows) {
    if (!meter) continue;
    const li = document.createElement("li");
    li.textContent = meter.reset_label
      ? `${label}: ${meter.pct_label} — resets in ${meter.reset_label}`
      : `${label}: ${meter.pct_label}`;
    meterListEl.appendChild(li);
  }

  // Fable meter isn't available on every plan — disable (but don't
  // silently hide) the checkbox when we've never seen data for it.
  const fableAvailable = dto.weekly_fable !== null || dto.status === "no_data_yet";
  showFableEl.disabled = !fableAvailable;
  fableHintEl.textContent = fableAvailable ? "" : "(not available on your plan)";
}

async function loadSettings() {
  const settings = await invoke<Settings>("get_settings");
  savingFromUi = true;
  showSessionEl.checked = settings.show_session;
  showWeeklyEl.checked = settings.show_weekly;
  showFableEl.checked = settings.show_fable;
  refreshIntervalEl.value = String(settings.refresh_interval_secs);
  savingFromUi = false;
}

async function saveSettings() {
  if (savingFromUi) return;
  const settings: Settings = {
    show_session: showSessionEl.checked,
    show_weekly: showWeeklyEl.checked,
    show_fable: showFableEl.checked,
    refresh_interval_secs: Number(refreshIntervalEl.value),
  };
  await invoke<Settings>("save_settings", { settings });
}

window.addEventListener("DOMContentLoaded", async () => {
  showSessionEl = document.querySelector("#show-session")!;
  showWeeklyEl = document.querySelector("#show-weekly")!;
  showFableEl = document.querySelector("#show-fable")!;
  fableHintEl = document.querySelector("#fable-hint")!;
  refreshIntervalEl = document.querySelector("#refresh-interval")!;
  statusMessageEl = document.querySelector("#status-message")!;
  lastUpdatedEl = document.querySelector("#last-updated")!;
  meterListEl = document.querySelector("#meter-list")!;
  refreshNowBtn = document.querySelector("#refresh-now")!;

  for (const el of [showSessionEl, showWeeklyEl, showFableEl, refreshIntervalEl]) {
    el.addEventListener("change", saveSettings);
  }

  refreshNowBtn.addEventListener("click", () => {
    invoke("refresh_now");
  });

  await loadSettings();

  const initial = await invoke<UsageDto>("get_latest_usage");
  renderUsage(initial);

  await listen<UsageDto>("usage-updated", (event) => {
    renderUsage(event.payload);
  });
});
