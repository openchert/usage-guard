import { contextBridge, ipcRenderer } from 'electron';

contextBridge.exposeInMainWorld('usageGuard', {
  evaluate: (payload) => ipcRenderer.invoke('evaluate-usage', payload),
  demo: () => ipcRenderer.invoke('demo-snapshots'),
  getConfig: () => ipcRenderer.invoke('get-config'),
  setConfig: (patch) => ipcRenderer.invoke('set-config', patch)
});
