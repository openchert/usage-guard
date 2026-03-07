import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';

export const DEFAULT_CONFIG = {
  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC',
  nearLimitRatio: 0.85,
  inactiveThresholdHours: 8,
  quietHours: {
    enabled: true,
    startHour: 23,
    endHour: 8
  },
  profiles: []
};

export function getDefaultConfigPath() {
  return path.join(os.homedir(), '.usage-guard', 'config.json');
}

export function loadConfig(configPath = process.env.USAGEGUARD_CONFIG || getDefaultConfigPath()) {
  try {
    if (!fs.existsSync(configPath)) return DEFAULT_CONFIG;
    const raw = fs.readFileSync(configPath, 'utf8');
    const parsed = JSON.parse(raw);
    return {
      ...DEFAULT_CONFIG,
      ...parsed,
      quietHours: { ...DEFAULT_CONFIG.quietHours, ...(parsed.quietHours || {}) }
    };
  } catch {
    return DEFAULT_CONFIG;
  }
}

export function saveConfig(config, configPath = process.env.USAGEGUARD_CONFIG || getDefaultConfigPath()) {
  const dir = path.dirname(configPath);
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(configPath, JSON.stringify(config, null, 2), 'utf8');
}
