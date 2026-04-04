import type { AppProps } from "next/app";
import { useRouter } from "next/router";
import { AuthProvider, useAuth } from "@/contexts/AuthProvider";
import { WebSocketProvider } from "@/contexts/WebSocketProvider";
import Layout from "@/components/layout/Layout";
import "@/styles/globals.css";

const AUTH_PAGES = ["/login", "/register"];

function AppContent({ Component, pageProps }: AppProps) {
  const { user, loading } = useAuth();
  const router = useRouter();

  // Auth pages: no sidebar/topbar layout
  if (AUTH_PAGES.includes(router.pathname)) {
    return (
      <div className="min-h-screen">
        <Component {...pageProps} />
      </div>
    );
  }

  // Loading state
  if (loading) {
    return (
      <div className="min-h-screen flex items-center justify-center">
        <div className="text-[var(--text-muted)]">Loading...</div>
      </div>
    );
  }

  // Protected pages: full layout
  return (
    <WebSocketProvider>
      <Layout>
        <Component {...pageProps} />
      </Layout>
    </WebSocketProvider>
  );
}

export default function App(props: AppProps) {
  return (
    <AuthProvider>
      <AppContent {...props} />
    </AuthProvider>
  );
}
