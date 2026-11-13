const state = {
  languages: [],
  mode: "fast",
  defaultSource: "auto",
  defaultTarget: "eng_Latn",
  theme: localStorage.getItem("fluent-theme") || "system",
};

const els = {
  sourceSearch: document.querySelector("#sourceSearch"),
  targetSearch: document.querySelector("#targetSearch"),
  sourceLanguage: document.querySelector("#sourceLanguage"),
  targetLanguage: document.querySelector("#targetLanguage"),
  sourceText: document.querySelector("#sourceText"),
  translationOutput: document.querySelector("#translationOutput"),
  translateButton: document.querySelector("#translateButton"),
  swapLanguages: document.querySelector("#swapLanguages"),
  clearText: document.querySelector("#clearText"),
  copyResult: document.querySelector("#copyResult"),
  charCount: document.querySelector("#charCount"),
  message: document.querySelector("#message"),
  detectedLanguage: document.querySelector("#detectedLanguage"),
  engineStatus: document.querySelector("#engineStatus"),
  deviceLabel: document.querySelector("#deviceLabel"),
  reloadStatus: document.querySelector("#reloadStatus"),
  themeToggle: document.querySelector("#themeToggle"),
  themeIcon: document.querySelector("#themeIcon"),
  accentColor: document.querySelector("#accentColor"),
  modeButtons: document.querySelectorAll("[data-mode]"),
};

function labelFor(language) {
  if (language.code === "auto") return "Auto - detect";
  return `${language.name} (${language.code})`;
}

function setMessage(text, isError = false) {
  els.message.textContent = text;
  els.message.classList.toggle("error", isError);
}

function applyTheme() {
  const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  const activeTheme = state.theme === "system" ? (prefersDark ? "dark" : "light") : state.theme;
  document.documentElement.dataset.theme = activeTheme;
  els.themeIcon.textContent = activeTheme === "dark" ? "☾" : "☼";
}

function cycleTheme() {
  state.theme = state.theme === "system" ? "dark" : state.theme === "dark" ? "light" : "system";
  localStorage.setItem("fluent-theme", state.theme);
  applyTheme();
}

function renderSelect(select, filter, includeAuto) {
  const previous = select.value;
  const term = filter.trim().toLowerCase();
  const options = state.languages
    .filter((language) => includeAuto || language.code !== "auto")
    .filter((language) => {
      const haystack = `${language.name} ${language.native} ${language.code}`.toLowerCase();
      return haystack.includes(term);
    });

  select.innerHTML = "";
  for (const language of options) {
    const option = document.createElement("option");
    option.value = language.code;
    option.textContent = labelFor(language);
    select.append(option);
  }

  if (options.some((language) => language.code === previous)) {
    select.value = previous;
  }
}

function languageName(code) {
  return state.languages.find((language) => language.code === code)?.name || code;
}

async function loadConfig() {
  const response = await fetch("/api/config");
  const config = await response.json();
  state.mode = config.default_mode || "fast";
  state.defaultSource = config.default_source_language || "auto";
  state.defaultTarget = config.default_target_language || "eng_Latn";
  els.modeButtons.forEach((button) => button.classList.toggle("active", button.dataset.mode === state.mode));
}

async function loadLanguages() {
  const response = await fetch("/api/languages");
  const data = await response.json();
  state.languages = data.languages;
  renderSelect(els.sourceLanguage, "", true);
  renderSelect(els.targetLanguage, "", false);
}

async function refreshStatus() {
  try {
    const response = await fetch("/api/status");
    const status = await response.json();
    els.engineStatus.innerHTML = `<span class="status-dot"></span><span>${status.device.toUpperCase()} ready</span>`;
    els.deviceLabel.textContent = status.cuda ? "GPU CUDA" : "CPU";
  } catch (error) {
    els.engineStatus.innerHTML = '<span class="status-dot"></span><span>Offline</span>';
    els.deviceLabel.textContent = "Unavailable";
  }
}

async function translate() {
  const text = els.sourceText.value.trim();
  if (!text) {
    setMessage("Enter some text before translating.", true);
    return;
  }

  els.translateButton.disabled = true;
  setMessage("Translation in progress. The first run may load the local model.");
  els.translationOutput.textContent = "";
  els.translationOutput.append(Object.assign(document.createElement("span"), { className: "placeholder", textContent: "Translation in progress..." }));

  try {
    const response = await fetch("/api/translate", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        text,
        source_language: els.sourceLanguage.value,
        target_language: els.targetLanguage.value,
        mode: state.mode,
      }),
    });
    const data = await response.json();
    if (!response.ok) throw new Error(data.error || "Translation failed.");
    els.translationOutput.textContent = data.translation;
    const source = data.detected_language ? languageName(data.detected_language.code) : languageName(data.source_language);
    els.detectedLanguage.textContent = `Source: ${source} → ${languageName(data.target_language)}`;
    setMessage(`Done with ${data.mode} mode on ${data.device.toUpperCase()}.`);
    refreshStatus();
  } catch (error) {
    els.translationOutput.innerHTML = '<span class="placeholder">No translation available.</span>';
    setMessage(error.message, true);
  } finally {
    els.translateButton.disabled = false;
  }
}

function swapLanguages() {
  if (els.sourceLanguage.value === "auto") {
    setMessage("Choose a specific source language before swapping.", true);
    return;
  }
  const oldSource = els.sourceLanguage.value;
  els.sourceLanguage.value = els.targetLanguage.value;
  els.targetLanguage.value = oldSource;
  const output = els.translationOutput.textContent.trim();
  if (output && !els.translationOutput.querySelector(".placeholder")) {
    els.sourceText.value = output;
    updateCount();
    els.translationOutput.innerHTML = '<span class="placeholder">The translation will appear here.</span>';
  }
}

function updateCount() {
  els.charCount.textContent = `${els.sourceText.value.length} / 5000`;
}

function bindEvents() {
  els.sourceSearch.addEventListener("input", () => renderSelect(els.sourceLanguage, els.sourceSearch.value, true));
  els.targetSearch.addEventListener("input", () => renderSelect(els.targetLanguage, els.targetSearch.value, false));
  els.sourceText.addEventListener("input", updateCount);
  els.translateButton.addEventListener("click", translate);
  els.swapLanguages.addEventListener("click", swapLanguages);
  els.clearText.addEventListener("click", () => {
    els.sourceText.value = "";
    els.translationOutput.innerHTML = '<span class="placeholder">The translation will appear here.</span>';
    els.detectedLanguage.textContent = "Automatic detection available";
    updateCount();
    setMessage("");
  });
  els.copyResult.addEventListener("click", async () => {
    const text = els.translationOutput.textContent.trim();
    if (!text || els.translationOutput.querySelector(".placeholder")) return;
    await navigator.clipboard.writeText(text);
    setMessage("Translation copied.");
  });
  els.reloadStatus.addEventListener("click", refreshStatus);
  els.themeToggle.addEventListener("click", cycleTheme);
  els.accentColor.addEventListener("input", (event) => {
    document.documentElement.style.setProperty("--accent", event.target.value);
  });
  els.modeButtons.forEach((button) => {
    button.addEventListener("click", () => {
      state.mode = button.dataset.mode;
      els.modeButtons.forEach((candidate) => candidate.classList.toggle("active", candidate === button));
    });
  });
}

async function boot() {
  applyTheme();
  bindEvents();
  await loadLanguages();
  await loadConfig();
  renderSelect(els.sourceLanguage, "", true);
  renderSelect(els.targetLanguage, "", false);
  els.sourceLanguage.value = state.defaultSource;
  els.targetLanguage.value = state.defaultTarget;
  updateCount();
  refreshStatus();
}

boot();
