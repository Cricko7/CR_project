import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { authService } from './authService';
import type { AuthSession, LoginInput, RegisterInput } from './types';

interface AuthContextValue {
  session: AuthSession | null;
  loading: boolean;
  login: (input: LoginInput) => Promise<void>;
  register: (input: RegisterInput) => Promise<void>;
  logout: () => void;
  refreshNow: () => Promise<void>;
}

const AuthContext = createContext<AuthContextValue | null>(null);

const willExpireSoon = (isoTime: string, withinMs = 1000 * 20) =>
  new Date(isoTime).getTime() - Date.now() <= withinMs;

export const AuthProvider = ({ children }: { children: ReactNode }) => {
  const [session, setSession] = useState<AuthSession | null>(null);
  const [loading, setLoading] = useState(true);
  const refreshInFlightRef = useRef<Promise<AuthSession> | null>(null);

  const logout = useCallback(() => {
    authService.logout();
    setSession(null);
  }, []);

  const runRefresh = useCallback(
    async (refreshToken: string) => {
      if (refreshInFlightRef.current) return refreshInFlightRef.current;

      const request = authService
        .refresh(refreshToken)
        .then((refreshed) => {
          setSession(refreshed);
          return refreshed;
        })
        .catch((error) => {
          logout();
          throw error;
        })
        .finally(() => {
          if (refreshInFlightRef.current === request) {
            refreshInFlightRef.current = null;
          }
        });

      refreshInFlightRef.current = request;
      return request;
    },
    [logout]
  );

  const refreshNow = useCallback(async () => {
    const refreshToken = session?.tokens.refreshToken;
    if (!refreshToken) return;
    try {
      await runRefresh(refreshToken);
    } catch {
      // Logout is handled in runRefresh.
    }
  }, [runRefresh, session]);

  useEffect(() => {
    const restored = authService.getSession();
    if (!restored) {
      setLoading(false);
      return;
    }

    const accessExpired = new Date(restored.tokens.accessExpiresAt).getTime() <= Date.now();
    const refreshExpired = new Date(restored.tokens.refreshExpiresAt).getTime() <= Date.now();
    if (refreshExpired) {
      logout();
      setLoading(false);
      return;
    }

    if (accessExpired) {
      runRefresh(restored.tokens.refreshToken)
        .catch(() => undefined)
        .finally(() => setLoading(false));
      return;
    }

    setSession(restored);
    setLoading(false);
  }, [logout, runRefresh]);

  useEffect(() => {
    if (!session) return;
    const timer = window.setInterval(() => {
      if (willExpireSoon(session.tokens.accessExpiresAt)) {
        void runRefresh(session.tokens.refreshToken).catch(() => undefined);
      }
    }, 8000);
    return () => window.clearInterval(timer);
  }, [runRefresh, session]);

  const login = useCallback(async (input: LoginInput) => {
    const next = await authService.login(input);
    setSession(next);
  }, []);

  const register = useCallback(async (input: RegisterInput) => {
    const next = await authService.register(input);
    setSession(next);
  }, []);

  const value = useMemo<AuthContextValue>(
    () => ({
      session,
      loading,
      login,
      register,
      logout,
      refreshNow
    }),
    [loading, login, logout, refreshNow, register, session]
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
};

export const useAuth = () => {
  const context = useContext(AuthContext);
  if (!context) throw new Error('useAuth must be used inside AuthProvider');
  return context;
};
