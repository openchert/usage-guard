import { app, BrowserWindow, ipcMain } from 'electron';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { evaluateAlerts } from '../../../core/alerts.js';
import { getOpenAIMockSnapshot, getAnthropicMockSnapshot } from '../../../core/providers.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

function createWindow() {
  const win = new BrowserWindow({
    width: 920,
    height: 700,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
      nodeIntegration: false
    }
  });
  win.loadFile(path.join(__dirname, 'renderer.html'));
}

ipcMain.handle('evaluate-usage', async (_, payload) => {
  const alerts = evaluateAlerts(payload);
  return { alerts };
});

ipcMain.handle('demo-snapshots', async () => {
  const openai = await getOpenAIMockSnapshot();
  const anthropic = await getAnthropicMockSnapshot();
  return {
    snapshots: [openai, anthropic].map((s) => ({ ...s, alerts: evaluateAlerts(s) }))
  };
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
