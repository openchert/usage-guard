#!/usr/bin/env node
import { Command } from 'commander';
import { evaluateAlerts } from '../../../core/alerts.js';
import { getOpenAIMockSnapshot, getAnthropicMockSnapshot } from '../../../core/providers.js';

const program = new Command();
program.name('usageguard').description('Local-first AI usage monitor CLI');

function printSnapshot(snapshot) {
  console.log(`Provider: ${snapshot.provider} (${snapshot.accountLabel})`);
  console.log(`Spend: $${snapshot.spent.toFixed(2)} / $${snapshot.limit.toFixed(2)}`);
  console.log(`Tokens: in=${snapshot.tokensIn.toLocaleString()} out=${snapshot.tokensOut.toLocaleString()}`);
  console.log(`Inactive: ${snapshot.inactiveHours}h`);
  const alerts = evaluateAlerts(snapshot);
  if (alerts.length === 0) {
    console.log('Alerts: none');
  } else {
    for (const a of alerts) console.log(`- [${a.level}] ${a.message}`);
  }
  console.log('');
}

program.command('demo').description('Show sample provider snapshots').action(async () => {
  printSnapshot(await getOpenAIMockSnapshot());
  printSnapshot(await getAnthropicMockSnapshot());
});

program.command('check')
  .description('Check usage against limits and inactivity thresholds')
  .requiredOption('--spent <number>', 'Spent amount in USD', Number)
  .requiredOption('--limit <number>', 'Budget limit in USD', Number)
  .option('--inactive-hours <number>', 'Hours without activity', Number, 0)
  .option('--inactive-threshold-hours <number>', 'Trigger inactivity alert at N hours', Number, 8)
  .action((opts) => {
    const snapshot = {
      provider: 'custom',
      accountLabel: 'local',
      spent: opts.spent,
      limit: opts.limit,
      tokensIn: 0,
      tokensOut: 0,
      inactiveHours: opts.inactiveHours
    };
    printSnapshot(snapshot);
    const alerts = evaluateAlerts({ ...snapshot, inactiveThresholdHours: opts.inactiveThresholdHours });
    if (!alerts.length) process.exit(0);
    const hasCritical = alerts.some(a => a.level === 'critical');
    process.exit(hasCritical ? 2 : 1);
  });

program.parse();
