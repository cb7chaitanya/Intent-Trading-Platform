import type { AppProps } from "next/app";
import { WebSocketProvider } from "@/contexts/WebSocketProvider";
import Layout from "@/components/layout/Layout";
import "@/styles/globals.css";

export default function App({ Component, pageProps }: AppProps) {
  return (
    <WebSocketProvider>
      <Layout>
        <Component {...pageProps} />
      </Layout>
    </WebSocketProvider>
  );
}
