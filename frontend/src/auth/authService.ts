import { API_BASE_URL, BackendError, isBackendUnavailableError, requestJson } from '../lib/backend';
import type { AuthSession, AuthTokens, AuthUser, LoginInput, RegisterInput } from './types';

const SESSION_KEY = 'cyberlife.auth.session';
const ACCESS_TTL_MS = 1000 * 60 * 2;
const REFRESH_TTL_MS = 1000 * 60 * 60 * 24 * 7;

const AUTH_REGISTER_PATH = '/auth/register';
const AUTH_LOGIN_PATH = '/auth/login';
const AUTH_REFRESH_PATH = '/auth/refresh';

const now = () => Date.now();

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null;

const readString = (record: Record<string, unknown>, ...keys: string[]) => {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string' && value.trim().length > 0) return value;
  }
  return null;
};

const readNumber = (record: Record<string, unknown>, ...keys: string[]) => {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'number' && Number.isFinite(value)) return value;
  }
  return null;
};

const saveSession = (session: AuthSession) => {
  try {
    localStorage.setItem(SESSION_KEY, JSON.stringify(session));
  } catch {
    // Ignore storage write failures (private mode / quota / blocked storage).
  }
};

const clearSession = () => {
  try {
    localStorage.removeItem(SESSION_KEY);
  } catch {
    // Ignore storage delete failures.
  }
};

const randomId = (prefix: string) => `${prefix}_${Math.random().toString(36).slice(2)}_${Date.now().toString(36)}`;

const buildFallbackUser = (
  input: Pick<LoginInput, 'email'> & Partial<Pick<RegisterInput, 'name'>>,
  current?: AuthUser
): AuthUser => ({
  id: current?.id ?? randomId('user'),
  email: input.email.trim().toLowerCase(),
  name: input.name?.trim() || current?.name || 'Operator',
  createdAt: current?.createdAt ?? new Date().toISOString()
});

const normalizeTokens = (payload: unknown): AuthTokens | null => {
  if (!isRecord(payload)) return null;
  const source = isRecord(payload.tokens) ? payload.tokens : payload;

  const accessToken =
    readString(source, 'accessToken', 'access_token', 'access') ??
    readString(payload, 'accessToken', 'access_token', 'access');
  const refreshToken =
    readString(source, 'refreshToken', 'refresh_token', 'refresh') ??
    readString(payload, 'refreshToken', 'refresh_token', 'refresh');

  if (!accessToken || !refreshToken) return null;

  const created = now();
  const accessExpiresAt =
    readString(source, 'accessExpiresAt', 'access_expires_at') ??
    readString(payload, 'accessExpiresAt', 'access_expires_at');
  const refreshExpiresAt =
    readString(source, 'refreshExpiresAt', 'refresh_expires_at') ??
    readString(payload, 'refreshExpiresAt', 'refresh_expires_at');

  const accessTtlSec = readNumber(source, 'access_expires_in', 'accessExpiresIn', 'expires_in');
  const refreshTtlSec = readNumber(source, 'refresh_expires_in', 'refreshExpiresIn');

  const normalizeExpiry = (raw: string | null, fallbackMs: number) => {
    if (raw) {
      const parsed = new Date(raw);
      if (Number.isFinite(parsed.getTime())) return parsed.toISOString();
    }
    return new Date(created + fallbackMs).toISOString();
  };

  return {
    accessToken,
    refreshToken,
    accessExpiresAt: normalizeExpiry(accessExpiresAt, (accessTtlSec ?? ACCESS_TTL_MS / 1000) * 1000),
    refreshExpiresAt: normalizeExpiry(refreshExpiresAt, (refreshTtlSec ?? REFRESH_TTL_MS / 1000) * 1000)
  };
};

const normalizeUser = (payload: unknown, fallback: AuthUser): AuthUser => {
  if (!isRecord(payload)) return fallback;
  const source = isRecord(payload.user) ? payload.user : payload;

  const email = readString(source, 'email') ?? fallback.email;
  const name = readString(source, 'name', 'display_name', 'displayName') ?? fallback.name;
  const id = readString(source, 'id', 'user_id', 'userId') ?? fallback.id;
  const createdAt = readString(source, 'createdAt', 'created_at') ?? fallback.createdAt;

  return { id, email, name, createdAt };
};

const normalizeSession = (payload: unknown, fallbackUser: AuthUser): AuthSession | null => {
  const tokens = normalizeTokens(payload);
  if (!tokens) return null;
  return {
    user: normalizeUser(payload, fallbackUser),
    tokens
  };
};

const toAuthError = (error: unknown, fallbackMessage = 'Authentication failed.'): Error => {
  if (isBackendUnavailableError(error)) {
    return new Error('Internal error. Authentication service is unavailable. Reconnecting...');
  }
  if (error instanceof BackendError && error.kind === 'api_error') {
    return new Error(error.message || fallbackMessage);
  }
  if (error instanceof Error) return error;
  return new Error(fallbackMessage);
};

export const authService = {
  getSession(): AuthSession | null {
    try {
      const raw = localStorage.getItem(SESSION_KEY);
      if (!raw) return null;
      const parsed = JSON.parse(raw) as AuthSession;
      if (!parsed?.user || !parsed?.tokens) {
        clearSession();
        return null;
      }
      const accessExpiresAt = new Date(parsed.tokens.accessExpiresAt).getTime();
      const refreshExpiresAt = new Date(parsed.tokens.refreshExpiresAt).getTime();
      if (!Number.isFinite(accessExpiresAt) || !Number.isFinite(refreshExpiresAt)) {
        clearSession();
        return null;
      }
      return parsed;
    } catch {
      clearSession();
      return null;
    }
  },

  async register(input: RegisterInput): Promise<AuthSession> {
    const fallbackUser = buildFallbackUser(input);

    try {
      const payload = await requestJson<unknown>({
        path: AUTH_REGISTER_PATH,
        method: 'POST',
        body: {
          name: input.name,
          email: input.email,
          password: input.password
        }
      });
      const session = normalizeSession(payload, fallbackUser);
      if (!session) throw new Error('Invalid auth response payload.');
      saveSession(session);
      return session;
    } catch (error) {
      throw toAuthError(error);
    }
  },

  async login(input: LoginInput): Promise<AuthSession> {
    const current = this.getSession();
    const fallbackUser = buildFallbackUser(input, current?.user);

    try {
      const payload = await requestJson<unknown>({
        path: AUTH_LOGIN_PATH,
        method: 'POST',
        body: {
          email: input.email,
          password: input.password
        }
      });
      const session = normalizeSession(payload, fallbackUser);
      if (!session) throw new Error('Invalid auth response payload.');
      saveSession(session);
      return session;
    } catch (error) {
      throw toAuthError(error);
    }
  },

  async refresh(refreshToken: string): Promise<AuthSession> {
    const current = this.getSession();
    if (!current) throw new Error('No active session.');

    try {
      const payload = await requestJson<unknown>({
        path: AUTH_REFRESH_PATH,
        method: 'POST',
        body: {
          refresh_token: refreshToken
        },
        accessToken: current.tokens.accessToken
      });
      const session = normalizeSession(payload, current.user);
      if (!session) throw new Error('Invalid refresh response payload.');
      saveSession(session);
      return session;
    } catch (error) {
      throw toAuthError(error, 'Session refresh failed.');
    }
  },

  logout() {
    clearSession();
  },

  authModeHint() {
    return `Backend auth is expected at ${API_BASE_URL}${AUTH_LOGIN_PATH}, ${API_BASE_URL}${AUTH_REGISTER_PATH}, ${API_BASE_URL}${AUTH_REFRESH_PATH}`;
  }
};
