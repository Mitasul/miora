(() => {
  const storageKey = 'miora.theme';
  const modes = ['system', 'light', 'dark'];
  const labels = {
    system: '外观：跟随系统',
    light: '外观：浅色',
    dark: '外观：深色',
  };
  const icons = {
    system: '<svg class="icon" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"><circle cx="12" cy="12" r="9"/><path d="M12 3v18a9 9 0 0 0 0-18Z" fill="currentColor"/></svg>',
    light: '<svg class="icon" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"><circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.93 4.93l1.41 1.41m11.32 11.32 1.41 1.41M2 12h2m16 0h2M4.93 19.07l1.41-1.41m11.32-11.32 1.41-1.41"/></svg>',
    dark: '<svg class="icon" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79Z"/></svg>',
  };
  const system = window.matchMedia('(prefers-color-scheme: dark)');
  const root = document.documentElement;
  let mode = 'system';
  let buttons = [];

  function syncButton(button) {
    button.value = mode;
    button.dataset.themeMode = mode;
    button.innerHTML = icons[mode];
    button.setAttribute('aria-label', labels[mode]);
    button.setAttribute('title', labels[mode]);
  }

  function apply(value) {
    mode = modes.includes(value) ? value : 'system';
    root.dataset.themeMode = mode;
    root.dataset.theme = mode === 'system' ? (system.matches ? 'dark' : 'light') : mode;
    for (const button of buttons) syncButton(button);
  }

  function nextMode() {
    return modes[(modes.indexOf(mode) + 1) % modes.length];
  }

  try { mode = window.localStorage.getItem(storageKey); } catch {}
  apply(mode);

  document.addEventListener('DOMContentLoaded', () => {
    buttons = document.querySelectorAll('[data-theme-toggle]');
    for (const button of buttons) syncButton(button);
  }, { once: true });

  document.addEventListener('click', event => {
    const button = event.target.closest?.('[data-theme-toggle]');
    if (!button) return;
    apply(nextMode());
    try {
      if (mode === 'system') window.localStorage.removeItem(storageKey);
      else window.localStorage.setItem(storageKey, mode);
    } catch {}
  });

  system.addEventListener('change', () => {
    if (mode === 'system') apply(mode);
  });

  window.addEventListener('storage', event => {
    if (event.key === storageKey || event.key === null) apply(event.newValue);
  });
})();
