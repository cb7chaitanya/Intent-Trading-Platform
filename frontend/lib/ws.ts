// Shared WebSocket types used by the provider and components

export interface WsTrade {
  id: string;
  market_id: string;
  buyer_account_id: string;
  seller_account_id: string;
  price: number;
  qty: number;
  fee: number;
  created_at: string;
}

export interface PriceLevel {
  price: number;
  qty: number;
}

export interface WsOrderBook {
  market_id: string;
  bids: PriceLevel[];
  asks: PriceLevel[];
  timestamp: string;
}

export interface WsAuctionResult {
  intent_id: string;
  winner_solver_id: string;
  amount_out: number;
  fee: number;
}

export type WsMessageType =
  | "Trade"
  | "OrderBook"
  | "AuctionResult"
  | "Subscribed"
  | "Unsubscribed"
  | "Pong";

export interface WsMessage {
  type: WsMessageType;
  data: any;
}
