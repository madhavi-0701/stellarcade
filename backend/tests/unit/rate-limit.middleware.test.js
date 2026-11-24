const { policies } = require('../../src/middleware/rate-limit.middleware');

describe('Rate limit environment threshold validation', () => {
  const originalEnv = process.env;

  beforeEach(() => {
    process.env = { ...originalEnv };
  });

  afterAll(() => {
    process.env = originalEnv;
  });

  it('uses defaults when environment variables are missing', () => {
    delete process.env.RATE_LIMIT_WALLET_MAX;
    delete process.env.RATE_LIMIT_WALLET_WINDOW;
    delete process.env.RATE_LIMIT_AUTH_MAX;
    delete process.env.RATE_LIMIT_AUTH_WINDOW;
    delete process.env.RATE_LIMIT_GAMES_MAX;
    delete process.env.RATE_LIMIT_GAMES_WINDOW;

    expect(policies.wallet.max()).toBe(10);
    expect(policies.wallet.window()).toBe(60);
    expect(policies.auth.max()).toBe(20);
    expect(policies.auth.window()).toBe(60);
    expect(policies.games.max()).toBe(60);
    expect(policies.games.window()).toBe(60);
  });

  it('accepts valid positive integer thresholds', () => {
    process.env.RATE_LIMIT_WALLET_MAX = '50';
    process.env.RATE_LIMIT_WALLET_WINDOW = '120';

    expect(policies.wallet.max()).toBe(50);
    expect(policies.wallet.window()).toBe(120);
  });

  it('rejects negative, zero, fractional, and prefix-invalid values', () => {
    process.env.RATE_LIMIT_WALLET_MAX = '-5';
    process.env.RATE_LIMIT_WALLET_WINDOW = '0';
    process.env.RATE_LIMIT_AUTH_MAX = '10junk';
    process.env.RATE_LIMIT_AUTH_WINDOW = '12.5';
    process.env.RATE_LIMIT_GAMES_MAX = 'Infinity';
    process.env.RATE_LIMIT_GAMES_WINDOW = 'NaN';

    expect(policies.wallet.max()).toBe(10);
    expect(policies.wallet.window()).toBe(60);
    expect(policies.auth.max()).toBe(20);
    expect(policies.auth.window()).toBe(60);
    expect(policies.games.max()).toBe(60);
    expect(policies.games.window()).toBe(60);
  });
});
