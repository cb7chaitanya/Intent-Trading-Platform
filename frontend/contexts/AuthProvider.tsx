import React, {
  createContext,
  useContext,
  useEffect,
  useState,
  useCallback,
} from "react";
import { useRouter } from "next/router";
import api from "@/lib/api";

interface User {
  user_id: string;
  email: string;
  roles: string[];
}

interface AuthContextValue {
  user: User | null;
  token: string | null;
  loading: boolean;
  login: (email: string, password: string) => Promise<void>;
  register: (email: string, password: string) => Promise<void>;
  logout: () => void;
  hasRole: (role: string) => boolean;
}

const AuthContext = createContext<AuthContextValue>({
  user: null,
  token: null,
  loading: true,
  login: async () => {},
  register: async () => {},
  logout: () => {},
  hasRole: () => false,
});

export const useAuth = () => useContext(AuthContext);

const TOKEN_KEY = "itx_token";
const USER_KEY = "itx_user";

// Public pages that don't require auth
const PUBLIC_PATHS = ["/login", "/register"];

export const AuthProvider: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => {
  const [user, setUser] = useState<User | null>(null);
  const [token, setToken] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const router = useRouter();

  // Restore session from localStorage on mount
  useEffect(() => {
    const savedToken = localStorage.getItem(TOKEN_KEY);
    const savedUser = localStorage.getItem(USER_KEY);

    if (savedToken && savedUser) {
      try {
        setToken(savedToken);
        setUser(JSON.parse(savedUser));
      } catch {
        localStorage.removeItem(TOKEN_KEY);
        localStorage.removeItem(USER_KEY);
      }
    }
    setLoading(false);
  }, []);

  // Axios interceptor: attach Authorization header
  useEffect(() => {
    const requestInterceptor = api.interceptors.request.use((config) => {
      const t = localStorage.getItem(TOKEN_KEY);
      if (t) {
        config.headers.Authorization = `Bearer ${t}`;
      }
      return config;
    });

    const responseInterceptor = api.interceptors.response.use(
      (response) => response,
      (error) => {
        if (error.response?.status === 401) {
          // Token expired or invalid — clear session
          localStorage.removeItem(TOKEN_KEY);
          localStorage.removeItem(USER_KEY);
          setUser(null);
          setToken(null);
          if (!PUBLIC_PATHS.includes(window.location.pathname)) {
            router.push("/login");
          }
        }
        return Promise.reject(error);
      }
    );

    return () => {
      api.interceptors.request.eject(requestInterceptor);
      api.interceptors.response.eject(responseInterceptor);
    };
  }, [router]);

  // Redirect unauthenticated users away from protected pages
  useEffect(() => {
    if (!loading && !user && !PUBLIC_PATHS.includes(router.pathname)) {
      router.push("/login");
    }
  }, [loading, user, router.pathname]);

  const login = useCallback(
    async (email: string, password: string) => {
      const data = await api
        .post("/auth/login", { email, password })
        .then((r) => r.data);

      const u: User = {
        user_id: data.user_id,
        email: data.email,
        roles: data.roles || [],
      };

      localStorage.setItem(TOKEN_KEY, data.token);
      localStorage.setItem(USER_KEY, JSON.stringify(u));
      setToken(data.token);
      setUser(u);
      router.push("/");
    },
    [router]
  );

  const register = useCallback(
    async (email: string, password: string) => {
      const data = await api
        .post("/auth/register", { email, password })
        .then((r) => r.data);

      const u: User = {
        user_id: data.user_id,
        email: data.email,
        roles: data.roles || [],
      };

      localStorage.setItem(TOKEN_KEY, data.token);
      localStorage.setItem(USER_KEY, JSON.stringify(u));
      setToken(data.token);
      setUser(u);
      router.push("/");
    },
    [router]
  );

  const logout = useCallback(() => {
    localStorage.removeItem(TOKEN_KEY);
    localStorage.removeItem(USER_KEY);
    setToken(null);
    setUser(null);
    router.push("/login");
  }, [router]);

  const hasRole = useCallback(
    (role: string) => {
      return user?.roles.includes(role) || user?.roles.includes("admin") || false;
    },
    [user]
  );

  return (
    <AuthContext.Provider
      value={{ user, token, loading, login, register, logout, hasRole }}
    >
      {children}
    </AuthContext.Provider>
  );
};
