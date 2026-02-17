type QueryValue = string | number | boolean | null | undefined;

export type QueryParams = Record<string, QueryValue>;

type BackendErrorKind = 'backend_unavailable' | 'api_error' | 'parse_error';

interface RequestJsonOptions {
  path: string;
  method?: 'GET' | 'POST';
  pathParams?: Record<string, QueryValue>;
  query?: QueryParams;
  body?: unknown;
  accessToken?: string;
  timeoutMs?: number;
}

interface SingleWsEventOptions {
  path: string;
  query?: QueryParams;
  timeoutMs?: number;
}

const DEFAULT_BASE_URL = 'http://127.0.0.1:8080';
const envBaseUrl = (import.meta as ImportMeta & { env?: Record<string, string | undefined> }).env?.VITE_API_BASE_URL;

const normalizeBaseUrl = (value: string) => value.trim().replace(/\/+$/, '');

export const API_BASE_URL = normalizeBaseUrl(envBaseUrl ?? DEFAULT_BASE_URL);

export class BackendError extends Error {
  readonly kind: BackendErrorKind;
  readonly status?: number;
  readonly payload?: unknown;

  constructor(kind: BackendErrorKind, message: string, status?: number, payload?: unknown) {
    super(message);
    this.name = 'BackendError';
    this.kind = kind;
    this.status = status;
    this.payload = payload;
  }
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null;

const buildQueryString = (query?: QueryParams) => {
  if (!query) return '';
  const params = new URLSearchParams();
  Object.entries(query).forEach(([key, value]) => {
    if (value === null || value === undefined) return;
    const normalized = String(value).trim();
    if (!normalized.length) return;
    params.set(key, normalized);
  });
  const serialized = params.toString();
  return serialized.length ? `?${serialized}` : '';
};

export const resolvePathTemplate = (
  path: string,
  pathParams?: Record<string, QueryValue>
) => {
  if (!pathParams) return path;
  return path.replace(/\{([^}]+)\}/g, (_match, key) => {
    const value = pathParams[key];
    if (value === undefined || value === null || String(value).trim().length === 0) {
      return `{${key}}`;
    }
    return encodeURIComponent(String(value));
  });
};

const toHttpUrl = (path: string, query?: QueryParams) => {
  const resolvedPath = path.startsWith('http://') || path.startsWith('https://')
    ? path
    : `${API_BASE_URL}${path.startsWith('/') ? path : `/${path}`}`;
  return `${resolvedPath}${buildQueryString(query)}`;
};

export const toWebSocketUrl = (path: string, query?: QueryParams) => {
  const base = new URL(API_BASE_URL);
  const wsProtocol = base.protocol === 'https:' ? 'wss:' : 'ws:';
  const resolvedPath = path.startsWith('/') ? path : `/${path}`;
  const url = `${wsProtocol}//${base.host}${resolvedPath}${buildQueryString(query)}`;
  return url;
};

const parseBody = async (response: Response): Promise<unknown> => {
  const contentType = response.headers.get('content-type') ?? '';
  if (contentType.includes('application/json')) {
    return response.json();
  }
  const raw = await response.text();
  if (!raw.length) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return raw;
  }
};

const extractMessage = (payload: unknown, fallback: string) => {
  if (typeof payload === 'string' && payload.trim().length > 0) return payload;
  if (!isRecord(payload)) return fallback;
  const message = payload.message;
  if (typeof message === 'string' && message.trim().length > 0) return message;
  const error = payload.error;
  if (typeof error === 'string' && error.trim().length > 0) return error;
  return fallback;
};

export const isBackendUnavailableError = (error: unknown) =>
  error instanceof BackendError && error.kind === 'backend_unavailable';

export const isBackendAuthMissingError = (error: unknown) =>
  error instanceof BackendError && error.kind === 'api_error' && (error.status === 404 || error.status === 501);

export const requestJson = async <T>({
  path,
  method = 'GET',
  pathParams,
  query,
  body,
  accessToken,
  timeoutMs = 8000
}: RequestJsonOptions): Promise<T> => {
  const resolvedPath = resolvePathTemplate(path, pathParams);
  const url = toHttpUrl(resolvedPath, query);

  const controller = new AbortController();
  const timeout = window.setTimeout(() => controller.abort(), timeoutMs);

  try {
    const headers = new Headers();
    if (body !== undefined) headers.set('Content-Type', 'application/json');
    if (accessToken) headers.set('Authorization', `Bearer ${accessToken}`);

    const response = await fetch(url, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: controller.signal
    });

    const payload = await parseBody(response);

    if (!response.ok) {
      throw new BackendError(
        'api_error',
        extractMessage(payload, `${response.status} ${response.statusText}`.trim()),
        response.status,
        payload
      );
    }

    return payload as T;
  } catch (error) {
    if (error instanceof BackendError) throw error;
    if (error instanceof DOMException && error.name === 'AbortError') {
      throw new BackendError('backend_unavailable', 'Request timeout');
    }
    throw new BackendError('backend_unavailable', 'Cannot connect to backend');
  } finally {
    window.clearTimeout(timeout);
  }
};

export const checkBackendHealth = async () => {
  try {
    await requestJson<{ status: string }>({ path: '/health', method: 'GET', timeoutMs: 2500 });
    return true;
  } catch {
    return false;
  }
};

export const readSingleWebSocketEvent = ({
  path,
  query,
  timeoutMs = 4500
}: SingleWsEventOptions): Promise<unknown> =>
  new Promise((resolve, reject) => {
    const socket = new WebSocket(toWebSocketUrl(path, query));
    let settled = false;

    const finish = (action: () => void) => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timeout);
      socket.onmessage = null;
      socket.onerror = null;
      socket.onclose = null;
      socket.close();
      action();
    };

    const timeout = window.setTimeout(() => {
      finish(() => reject(new BackendError('backend_unavailable', 'WebSocket timeout')));
    }, timeoutMs);

    socket.onmessage = (event) => {
      const raw = typeof event.data === 'string' ? event.data : String(event.data);
      try {
        const parsed = JSON.parse(raw) as unknown;
        finish(() => resolve(parsed));
      } catch {
        finish(() => reject(new BackendError('parse_error', 'Invalid WebSocket payload')));
      }
    };

    socket.onerror = () => {
      finish(() => reject(new BackendError('backend_unavailable', 'WebSocket connection failed')));
    };

    socket.onclose = () => {
      finish(() => reject(new BackendError('backend_unavailable', 'WebSocket closed before first event')));
    };
  });
