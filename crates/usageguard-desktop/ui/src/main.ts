import { mount } from 'svelte';
import App from './App.svelte';
import './app.css';
import { invoke } from './tauri';

async function init() {
  try {
    const cfg = await invoke('get_config') as { light_mode: boolean };
    if (cfg.light_mode) {
      document.documentElement.classList.add('light-mode');
    }
  } catch {
    // Non-Tauri context or config unavailable — keep dark mode default.
  }

  mount(App, {
    target: document.getElementById('app')!,
  });
}

void init();
