// Adapter layer (local-first). API key/OAuth provider integrations can be plugged in here.

export async function getOpenAIMockSnapshot() {
  return {
    provider: 'openai',
    accountLabel: 'OpenAI',
    spent: 12.4,
    limit: 30,
    tokensIn: 184000,
    tokensOut: 12300,
    inactiveHours: 2
  };
}

export async function getAnthropicMockSnapshot() {
  return {
    provider: 'anthropic',
    accountLabel: 'Anthropic',
    spent: 6.7,
    limit: 20,
    tokensIn: 92000,
    tokensOut: 8400,
    inactiveHours: 11
  };
}
