import fs from 'node:fs';

function parseNumber(v, fallback = 0) {
  const n = Number(v);
  return Number.isFinite(n) ? n : fallback;
}

function snapshotBase(partial) {
  return {
    provider: partial.provider,
    accountLabel: partial.accountLabel || partial.provider,
    spent: parseNumber(partial.spent, 0),
    limit: parseNumber(partial.limit, 0),
    tokensIn: parseNumber(partial.tokensIn, 0),
    tokensOut: parseNumber(partial.tokensOut, 0),
    inactiveHours: parseNumber(partial.inactiveHours, 0),
    source: partial.source || 'unknown'
  };
}

function computeFromNdjson(logPath) {
  if (!logPath || !fs.existsSync(logPath)) return null;
  const lines = fs.readFileSync(logPath, 'utf8').split(/\r?\n/).filter(Boolean);
  let tokensIn = 0;
  let tokensOut = 0;
  let lastTs = 0;
  for (const line of lines) {
    try {
      const row = JSON.parse(line);
      tokensIn += parseNumber(row.tokensIn || row.prompt_tokens || row.input_tokens, 0);
      tokensOut += parseNumber(row.tokensOut || row.completion_tokens || row.output_tokens, 0);
      const ts = Date.parse(row.timestamp || row.ts || row.createdAt || '') || 0;
      if (ts > lastTs) lastTs = ts;
    } catch {
      // ignore bad rows
    }
  }
  const inactiveHours = lastTs ? Math.floor((Date.now() - lastTs) / 3_600_000) : 999;
  return { tokensIn, tokensOut, inactiveHours };
}

async function tryFetchOpenAICosts() {
  const apiKey = process.env.OPENAI_API_KEY;
  if (!apiKey) return null;

  const endpoint = process.env.OPENAI_COSTS_ENDPOINT || 'https://api.openai.com/v1/organization/costs';
  const since = process.env.OPENAI_SINCE || new Date(Date.now() - 30 * 24 * 3600_000).toISOString().slice(0, 10);
  const url = new URL(endpoint);
  if (!url.searchParams.has('start_time')) url.searchParams.set('start_time', since);

  try {
    const res = await fetch(url, {
      headers: {
        Authorization: `Bearer ${apiKey}`,
        'Content-Type': 'application/json'
      }
    });
    if (!res.ok) return null;
    const json = await res.json();
    const spent = parseNumber(json?.total || json?.data?.[0]?.total || json?.amount?.value, NaN);
    if (!Number.isFinite(spent)) return null;
    return { spent, source: 'openai-api' };
  } catch {
    return null;
  }
}

async function tryFetchAnthropicCosts() {
  // Anthropic billing endpoints are account-dependent and not consistently public.
  // Support explicit endpoint override for local/proxy usage.
  const apiKey = process.env.ANTHROPIC_API_KEY;
  const endpoint = process.env.ANTHROPIC_COSTS_ENDPOINT;
  if (!apiKey || !endpoint) return null;
  try {
    const res = await fetch(endpoint, {
      headers: {
        'x-api-key': apiKey,
        'anthropic-version': '2023-06-01'
      }
    });
    if (!res.ok) return null;
    const json = await res.json();
    const spent = parseNumber(json?.total || json?.spent, NaN);
    if (!Number.isFinite(spent)) return null;
    return { spent, source: 'anthropic-api' };
  } catch {
    return null;
  }
}

export async function getOpenAISnapshot() {
  const limit = parseNumber(process.env.OPENAI_LIMIT_USD, 30);
  const fromApi = await tryFetchOpenAICosts();
  const fromLog = computeFromNdjson(process.env.OPENAI_USAGE_LOG);

  if (fromApi || fromLog) {
    return snapshotBase({
      provider: 'openai',
      accountLabel: process.env.OPENAI_ACCOUNT_LABEL || 'OpenAI',
      spent: fromApi?.spent ?? parseNumber(process.env.OPENAI_SPENT_USD, 0),
      limit,
      tokensIn: fromLog?.tokensIn ?? parseNumber(process.env.OPENAI_TOKENS_IN, 0),
      tokensOut: fromLog?.tokensOut ?? parseNumber(process.env.OPENAI_TOKENS_OUT, 0),
      inactiveHours: fromLog?.inactiveHours ?? parseNumber(process.env.OPENAI_INACTIVE_HOURS, 0),
      source: fromApi?.source || (fromLog ? 'openai-log' : 'openai-env')
    });
  }

  return snapshotBase({
    provider: 'openai',
    accountLabel: 'OpenAI',
    spent: 12.4,
    limit,
    tokensIn: 184000,
    tokensOut: 12300,
    inactiveHours: 2,
    source: 'mock'
  });
}

export async function getAnthropicSnapshot() {
  const limit = parseNumber(process.env.ANTHROPIC_LIMIT_USD, 20);
  const fromApi = await tryFetchAnthropicCosts();
  const fromLog = computeFromNdjson(process.env.ANTHROPIC_USAGE_LOG);

  if (fromApi || fromLog) {
    return snapshotBase({
      provider: 'anthropic',
      accountLabel: process.env.ANTHROPIC_ACCOUNT_LABEL || 'Anthropic',
      spent: fromApi?.spent ?? parseNumber(process.env.ANTHROPIC_SPENT_USD, 0),
      limit,
      tokensIn: fromLog?.tokensIn ?? parseNumber(process.env.ANTHROPIC_TOKENS_IN, 0),
      tokensOut: fromLog?.tokensOut ?? parseNumber(process.env.ANTHROPIC_TOKENS_OUT, 0),
      inactiveHours: fromLog?.inactiveHours ?? parseNumber(process.env.ANTHROPIC_INACTIVE_HOURS, 0),
      source: fromApi?.source || (fromLog ? 'anthropic-log' : 'anthropic-env')
    });
  }

  return snapshotBase({
    provider: 'anthropic',
    accountLabel: 'Anthropic',
    spent: 6.7,
    limit,
    tokensIn: 92000,
    tokensOut: 8400,
    inactiveHours: 11,
    source: 'mock'
  });
}
