/**
 * Redis-backed rate limiter middleware.
 *
 * Uses a sliding window counter pattern stored in Redis.
 * Thresholds are configurable via environment variables:
 *   RATE_LIMIT_WALLET_MAX    — max requests for wallet routes (default: 10)
 *   RATE_LIMIT_WALLET_WINDOW — window in seconds for wallet routes (default: 60)
 *   RATE_LIMIT_AUTH_MAX      — max requests for auth-sensitive user routes (default: 20)
 *   RATE_LIMIT_AUTH_WINDOW   — window in seconds for auth routes (default: 60)
 *   RATE_LIMIT_GAMES_MAX     — max requests for game routes (default: 60)
 *   RATE_LIMIT_GAMES_WINDOW  — window in seconds for game routes (default: 60)
 *
 * Failure modes are a deliberate per-policy choice (`onStoreError`):
 *   auth   -> deny  (fail closed, 503; brute-force targets)
 *   wallet -> allow (fail open, availability over enforcement)
 *   games  -> allow (fail open, read-only routes)
 */
const { client } = require('../config/redis');
const logger = require('../utils/logger');

const parsePositiveInt = (val, defaultVal, min = 1, max = 1_000_000) => {
  if (val === undefined || val === null || val === '') return defaultVal;
  if (typeof val === 'string' && !/^\d+$/.test(val.trim())) return defaultVal;
  const num = Number(val);
  if (!Number.isSafeInteger(num) || num < min || num > max) return defaultVal;
  return num;
};

const policies = {
  wallet: {
    max: () => parsePositiveInt(process.env.RATE_LIMIT_WALLET_MAX, 10),
    window: () => parsePositiveInt(process.env.RATE_LIMIT_WALLET_WINDOW, 60),
    onStoreError: 'allow',
  },
  auth: {
    max: () => parsePositiveInt(process.env.RATE_LIMIT_AUTH_MAX, 20),
    window: () => parsePositiveInt(process.env.RATE_LIMIT_AUTH_WINDOW, 60),
    onStoreError: 'deny',
  },
  games: {
    max: () => parsePositiveInt(process.env.RATE_LIMIT_GAMES_MAX, 60),
    window: () => parsePositiveInt(process.env.RATE_LIMIT_GAMES_WINDOW, 60),
    onStoreError: 'allow',
  },
};

/**
 * Creates a rate limiter middleware for the given policy name.
 * @param {'wallet'|'auth'|'games'} policyName
 * @returns {Function} Express middleware
 */
const rateLimit = (policyName) => {
  return async (req, res, next) => {
    const policy = policies[policyName];
    if (!policy) {
      logger.error(`[RateLimit] Unknown policy: ${policyName}`);
      return next();
    }

    const max = policy.max();
    const windowSecs = policy.window();
    const identifier = req.user?.id || req.ip;
    const key = `ratelimit:${policyName}:${identifier}`;

    try {
      const current = await client.incr(key);

      if (current === 1) {
        await client.expire(key, windowSecs);
      }

      const ttl = await client.ttl(key);
      const remaining = Math.max(0, max - current);

      res.set('X-RateLimit-Limit', String(max));
      res.set('X-RateLimit-Remaining', String(remaining));
      res.set('X-RateLimit-Reset', String(ttl));

      if (current > max) {
        res.set('Retry-After', String(ttl));
        return res.status(429).json({
          error: {
            message: 'Too many requests, please try again later.',
            code: 'RATE_LIMIT_EXCEEDED',
            retryAfter: ttl,
          },
        });
      }

      next();
    } catch (error) {
      logger.error(`[RateLimit] Redis error for policy ${policyName}:`, error);
      if (policy.onStoreError === 'deny') {
        return res.status(503).json({
          error: {
            message: 'Rate limiting is temporarily unavailable. Please try again later.',
            code: 'RATE_LIMIT_UNAVAILABLE',
          },
        });
      }
      next();
    }
  };
};

module.exports = { rateLimit, policies };
