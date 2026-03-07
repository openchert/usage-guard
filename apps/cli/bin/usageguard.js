#!/usr/bin/env node
import { Command } from 'commander';
import { evaluateAlerts } from '../../../core/alerts.js';
import { loadConfig, saveConfig } from '../../../core/config.js';
import { getOpenAISnapshot, getAnthropicSnapshot } from '../../../core/providers.js';

const program = new Command();
program.name('usageguard').description('Local-first AI usage monitor CLI');

function printSnapshot(snapshot, cfg) {
  console.log(`Provider: ${snapshot.provider} (${snapshot.accountLabel})`);
  console.log(`Spend: $${snapshot.spent.toFixed(2)} / $${snapshot.limit.toFixed(2)}`);
  console.log(`Tokens: in=${snapshot.tokensIn.toLocaleString()} out=${snapshot.tokensOut.toLocaleString()}`);
  console.log(`Inactive: ${snapshot.inactiveHours}h`);
  console.log(`Source: ${snapshot.source || 'unknown'}`);
  const alerts = evaluateAlerts({
    ...snapshot,
    nearLimitRatio: cfg.nearLimitRatio,
    inactiveThresholdHours: cfg.inactiveThresholdHours
  });
  if (alerts.length === 0) {
    console.log('Alerts: none');
  } else {
    for (const a of alerts) console.log(`- [${a.level}] ${a.message}`);
  }
  console.log('');
}

program.command('demo').description('Show provider snapshots (real adapters if configured)').action(async () => {
  const cfg = loadConfig();
  printSnapshot(await getOpenAISnapshot(), cfg);
  printSnapshot(await getAnthropicSnapshot(), cfg);
});

program.command('check')
  .description('Check usage against limits and inactivity thresholds')
  .requiredOption('--spent <number>', 'Spent amount in USD', Number)
  .requiredOption('--limit <number>', 'Budget limit in USD', Number)
  .option('--inactive-hours <number>', 'Hours without activity', Number, 0)
  .option('--inactive-threshold-hours <number>', 'Trigger inactivity alert at N hours', Number)
  .option('--near-limit-ratio <number>', 'Near-limit threshold ratio', Number)
  .action((opts) => {
    const cfg = loadConfig();
    const snapshot = {
      provider: 'custom',
      accountLabel: 'local',
      spent: opts.spent,
      limit: opts.limit,
      tokensIn: 0,
      tokensOut: 0,
      inactiveHours: opts.inactiveHours,
      source: 'cli'
    };
    printSnapshot(snapshot, {
      ...cfg,
      inactiveThresholdHours: opts.inactiveThresholdHours ?? cfg.inactiveThresholdHours,
      nearLimitRatio: opts.nearLimitRatio ?? cfg.nearLimitRatio
    });

    const alerts = evaluateAlerts({
      ...snapshot,
      inactiveThresholdHours: opts.inactiveThresholdHours ?? cfg.inactiveThresholdHours,
      nearLimitRatio: opts.nearLimitRatio ?? cfg.nearLimitRatio
    });

    if (!alerts.length) process.exit(0);
    const hasCritical = alerts.some((a) => a.level === 'critical');
    process.exit(hasCritical ? 2 : 1);
  });

program.command('config')
  .description('Set local config values')
  .option('--near-limit-ratio <number>', 'Set near-limit ratio', Number)
  .option('--inactive-threshold-hours <number>', 'Set inactivity threshold', Number)
  .option('--quiet-start <number>', 'Quiet hours start (0-23)', Number)
  .option('--quiet-end <number>', 'Quiet hours end (0-23)', Number)
  .action((opts) => {
    const cfg = loadConfig();
    if (opts.nearLimitRatio != null) cfg.nearLimitRatio = opts.nearLimitRatio;
    if (opts.inactiveThresholdHours != null) cfg.inactiveThresholdHours = opts.inactiveThresholdHours;
    if (opts.quietStart != null) cfg.quietHours.startHour = opts.quietStart;
    if (opts.quietEnd != null) cfg.quietHours.endHour = opts.quietEnd;
    saveConfig(cfg);
    console.log('Config saved');
    console.log(JSON.stringify(cfg, null, 2));
  });

program.parse();
