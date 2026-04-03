import React, {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  useCallback,
} from "react";

interface Trade {
  id: string;
  market_id: string;
  buyer_account_id: string;
  seller_account_id: string;
  price: number;
  qty: number;
  fee: number;
  created_at: string;
}

interface PriceLevel {
  price: number;
  qty: number;
}

interface OrderBook {
  bids: PriceLevel[];
  asks: PriceLevel[];
}

interface WsMessage {
  type: string;
  data: any;
}

interface WebSocketContextValue {
  connected: boolean;
  trades: Trade[];
  orderbook: OrderBook;
  subscribe: (marketId: string) => void;
  unsubscribe: (marketId: string) => void;
  lastMessage: WsMessage | null;
  addListener: (type: string, cb: (data: any) => void) => () => void;
}

const emptyOrderbook: OrderBook = { bids: [], asks: [] };

const WebSocketContext = createContext<WebSocketContextValue>({
  connected: false,
  trades: [],
  orderbook: emptyOrderbook,
  subscribe: () => {},
  unsubscribe: () => {},
  lastMessage: null,
  addListener: () => () => {},
});

export const useWebSocket = () => useContext(WebSocketContext);

const WS_URL =
  process.env.NEXT_PUBLIC_WS_URL || "ws://localhost:3000/ws/feed";
const MAX_TRADES = 200;
const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 30000;

export const WebSocketProvider: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => {
  const wsRef = useRef<WebSocket | null>(null);
  const listenersRef = useRef<Map<string, Set<(data: any) => void>>>(
    new Map()
  );
  const [connected, setConnected] = useState(false);
  const [lastMessage, setLastMessage] = useState<WsMessage | null>(null);
  const [trades, setTrades] = useState<Trade[]>([]);
  const [orderbook, setOrderbook] = useState<OrderBook>(emptyOrderbook);
  const reconnectTimer = useRef<ReturnType<typeof setTimeout>>();
  const retriesRef = useRef(0);
  const pendingSubscriptions = useRef<Set<string>>(new Set());

  const connect = useCallback(() => {
    if (
      wsRef.current?.readyState === WebSocket.OPEN ||
      wsRef.current?.readyState === WebSocket.CONNECTING
    )
      return;

    const ws = new WebSocket(WS_URL);
    wsRef.current = ws;

    ws.onopen = () => {
      setConnected(true);
      retriesRef.current = 0;

      // Re-subscribe to any markets that were active before reconnect
      pendingSubscriptions.current.forEach((marketId) => {
        ws.send(JSON.stringify({ action: "subscribe", market_id: marketId }));
      });
    };

    ws.onmessage = (event) => {
      try {
        const msg: WsMessage = JSON.parse(event.data);
        setLastMessage(msg);

        // Update first-class state
        if (msg.type === "Trade" && msg.data) {
          setTrades((prev) => [msg.data, ...prev].slice(0, MAX_TRADES));
        } else if (msg.type === "OrderBook" && msg.data) {
          setOrderbook({
            bids: msg.data.bids || [],
            asks: msg.data.asks || [],
          });
        }

        // Dispatch to type-specific listeners
        const callbacks = listenersRef.current.get(msg.type);
        if (callbacks) {
          callbacks.forEach((cb) => cb(msg.data));
        }
      } catch {}
    };

    ws.onclose = () => {
      setConnected(false);
      // Exponential backoff with jitter
      const delay = Math.min(
        RECONNECT_BASE_MS * 2 ** retriesRef.current + Math.random() * 500,
        RECONNECT_MAX_MS
      );
      retriesRef.current += 1;
      reconnectTimer.current = setTimeout(connect, delay);
    };

    ws.onerror = () => {
      ws.close();
    };
  }, []);

  useEffect(() => {
    connect();
    return () => {
      clearTimeout(reconnectTimer.current);
      wsRef.current?.close();
    };
  }, [connect]);

  const send = useCallback((data: object) => {
    if (wsRef.current?.readyState === WebSocket.OPEN) {
      wsRef.current.send(JSON.stringify(data));
    }
  }, []);

  const subscribe = useCallback(
    (marketId: string) => {
      pendingSubscriptions.current.add(marketId);
      // Reset state for the new market
      setTrades([]);
      setOrderbook(emptyOrderbook);
      send({ action: "subscribe", market_id: marketId });
    },
    [send]
  );

  const unsubscribe = useCallback(
    (marketId: string) => {
      pendingSubscriptions.current.delete(marketId);
      send({ action: "unsubscribe", market_id: marketId });
    },
    [send]
  );

  const addListener = useCallback(
    (type: string, cb: (data: any) => void) => {
      if (!listenersRef.current.has(type)) {
        listenersRef.current.set(type, new Set());
      }
      listenersRef.current.get(type)!.add(cb);
      return () => {
        listenersRef.current.get(type)?.delete(cb);
      };
    },
    []
  );

  return (
    <WebSocketContext.Provider
      value={{
        connected,
        trades,
        orderbook,
        subscribe,
        unsubscribe,
        lastMessage,
        addListener,
      }}
    >
      {children}
    </WebSocketContext.Provider>
  );
};
