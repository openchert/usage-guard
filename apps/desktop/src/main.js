import { app, BrowserWindow, ipcMain, Notification } from 'electron';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { evaluateAlerts } from '../../../core/alerts.js';
import { loadConfig, saveConfig } from '../../../core/config.js';
import { shouldNotifyNow } from '../../../core/notifications.js';
import { getOpenAISnapshot, getAnthropicSnapshot } from '../../../core/providers.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

function createWindow() {
  const win = new BrowserWindow({
    width: 980,
    height: 760,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false
    }
  });
  win.loadFile(path.join(__dirname, 'renderer.html'));
}

function maybeNotify(provider, alerts, cfg) {
  if (!shouldNotifyNow({ alerts, quietHours: cfg.quietHours })) return;
  const summary = alerts.map((a) => `${a.level}:${a.code}`).join(', ');
  new Notification({
    title: `UsageGuard • ${provider}`,
    body: summary || 'No alerts'
  }).show();
}

ipcMain.handle('evaluate-usage', async (_, payload) => {
  const cfg = loadConfig();
  const alerts = evaluateAlerts({
    ...payload,
    nearLimitRatio: cfg.nearLimitRatio,
    inactiveThresholdHours: cfg.inactiveThresholdHours
  });
  return { alerts, config: cfg };
});

ipcMain.handle('demo-snapshots', async () => {
  const cfg = loadConfig();
  const snapshots = [await getOpenAISnapshot(), await getAnthropicSnapshot()].map((s) => {
    const alerts = evaluateAlerts({
      ...s,
      nearLimitRatio: cfg.nearLimitRatio,
      inactiveThresholdHours: cfg.inactiveThresholdHours
    });
    maybeNotify(s.provider, alerts, cfg);
    return { ...s, alerts };
  });

  return { snapshots, config: cfg };
});

ipcMain.handle('get-config', async () => {
  return loadConfig();
});

ipcMain.handle('set-config', async (_, patch) => {
  const cfg = loadConfig();
  const next = {
    ...cfg,
    ...patch,
    quietHours: {
      ...cfg.quietHours,
      ...(patch?.quietHours || {})
    }
  };
  saveConfig(next);
  return next;
});

app.whenReady().then(() => {
  createWindow();
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit();
});
