//! End-to-end tests for the intent-based trading platform.
//!
//! Requires running services:
//!   - Postgres on DATABASE_URL
//!   - Redis on REDIS_URL
//!   - Platform on E2E_BASE_URL (default http://localhost:3000)
//!
//! Run: cargo test --test e2e_test --features e2e
//!
//! With docker:
//!   docker compose up -d postgres redis intent-trading
//!   E2E_BASE_URL=http://localhost:3000 cargo test --test e2e_test --features e2e

#![cfg(feature = "e2e")]

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

// ============================================================
// Config
// ============================================================

fn base_url() -> String {
    std::env::var("E2E_BASE_URL").unwrap_or_else(|_| "http://localhost:3000".into())
}

fn http() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap()
}

// ============================================================
// Response types
// ============================================================

#[derive(Debug, Deserialize)]
struct TokenResponse {
    token: String,
    user_id: String,
    email: String,
    roles: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Account {
    id: String,
    user_id: String,
}

#[derive(Debug, Deserialize)]
struct Balance {
    id: String,
    account_id: String,
    asset: String,
    available_balance: i64,
    locked_balance: i64,
}

#[derive(Debug, Deserialize)]
struct Intent {
    id: String,
    status: String,
    amount_in: i64,
}

#[derive(Debug, Deserialize)]
struct Bid {
    id: String,
    intent_id: String,
    solver_id: String,
    amount_out: i64,
}

// ============================================================
// Helpers
// ============================================================

async fn register(client: &Client) -> TokenResponse {
    let email = format!("e2e-{}@test.com", Uuid::new_v4());
    let resp = client
        .post(format!("{}/auth/register", base_url()))
        .json(&json!({ "email": email, "password": "E2eTest!Pass123" }))
        .send()
        .await
        .expect("Register request failed");
    assert_eq!(resp.status(), StatusCode::CREATED, "Registration failed");
    resp.json().await.unwrap()
}

async fn authed(client: &Client, token: &str) -> reqwest::RequestBuilder {
    client
        .get(format!("{}/intents", base_url()))
        .header("Authorization", format!("Bearer {token}"))
}

fn auth_header(token: &str) -> (String, String) {
    ("Authorization".into(), format!("Bearer {token}"))
}

async fn get_account(client: &Client, token: &str, user_id: &str) -> Account {
    let resp = client
        .get(format!("{}/accounts/{user_id}", base_url()))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap();
    let accounts: Vec<Account> = resp.json().await.unwrap();
    accounts.into_iter().next().expect("No account found")
}

async fn deposit(client: &Client, token: &str, account_id: &str, asset: &str, amount: i64) -> Balance {
    client
        .post(format!("{}/balances/deposit", base_url()))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "account_id": account_id, "asset": asset, "amount": amount }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn get_balances(client: &Client, token: &str, account_id: &str) -> Vec<Balance> {
    client
        .get(format!("{}/balances/{account_id}", base_url()))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

// ============================================================
// Test 1: Registration and Login
// ============================================================

#[tokio::test]
async fn test_register_and_login() {
    let client = http();
    let email = format!("e2e-{}@test.com", Uuid::new_v4());
    let password = "E2eTest!Pass123";

    // Register
    let resp = client
        .post(format!("{}/auth/register", base_url()))
        .json(&json!({ "email": email, "password": password }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let reg: TokenResponse = resp.json().await.unwrap();
    assert!(!reg.token.is_empty());
    assert!(!reg.user_id.is_empty());

    // Login with same credentials
    let resp = client
        .post(format!("{}/auth/login", base_url()))
        .json(&json!({ "email": email, "password": password }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let login: TokenResponse = resp.json().await.unwrap();
    assert_eq!(login.user_id, reg.user_id);
    assert!(!login.token.is_empty());
    assert_ne!(login.token, reg.token); // different token each time

    // Login with wrong password
    let resp = client
        .post(format!("{}/auth/login", base_url()))
        .json(&json!({ "email": email, "password": "WrongPass!123" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ============================================================
// Test 2: Deposit Funds
// ============================================================

#[tokio::test]
async fn test_deposit_funds() {
    let client = http();
    let auth = register(&client).await;
    let account = get_account(&client, &auth.token, &auth.user_id).await;

    let balance = deposit(&client, &auth.token, &account.id, "USDC", 50000).await;
    assert_eq!(balance.available_balance, 50000);
    assert_eq!(balance.locked_balance, 0);

    // Deposit more
    let balance = deposit(&client, &auth.token, &account.id, "USDC", 25000).await;
    assert_eq!(balance.available_balance, 75000);
}

// ============================================================
// Test 3: Create Intent
// ============================================================

#[tokio::test]
async fn test_create_intent() {
    let client = http();
    let auth = register(&client).await;
    let account = get_account(&client, &auth.token, &auth.user_id).await;

    // Deposit funds first
    deposit(&client, &auth.token, &account.id, "ETH", 10000).await;

    // Create intent
    let deadline = chrono::Utc::now().timestamp() + 3600;
    let resp = client
        .post(format!("{}/intents", base_url()))
        .header("Authorization", format!("Bearer {}", auth.token))
        .json(&json!({
            "user_id": auth.user_id,
            "account_id": account.id,
            "token_in": "ETH",
            "token_out": "USDC",
            "amount_in": 1000,
            "min_amount_out": 900,
            "deadline": deadline
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let intent: Intent = resp.json().await.unwrap();
    assert_eq!(intent.status, "Open");
    assert_eq!(intent.amount_in, 1000);

    // Check balance is locked
    let balances = get_balances(&client, &auth.token, &account.id).await;
    let eth = balances.iter().find(|b| b.asset == "ETH").unwrap();
    assert_eq!(eth.available_balance, 9000); // 10000 - 1000 locked
    assert_eq!(eth.locked_balance, 1000);
}

// ============================================================
// Test 4: Insufficient Balance Rejection
// ============================================================

#[tokio::test]
async fn test_insufficient_balance_rejection() {
    let client = http();
    let auth = register(&client).await;
    let account = get_account(&client, &auth.token, &auth.user_id).await;

    // No deposit — create intent should fail
    let resp = client
        .post(format!("{}/intents", base_url()))
        .header("Authorization", format!("Bearer {}", auth.token))
        .json(&json!({
            "user_id": auth.user_id,
            "account_id": account.id,
            "token_in": "ETH",
            "token_out": "USDC",
            "amount_in": 1000,
            "min_amount_out": 900,
            "deadline": chrono::Utc::now().timestamp() + 3600
        }))
        .send()
        .await
        .unwrap();
    assert_ne!(resp.status(), StatusCode::CREATED);
}

// ============================================================
// Test 5: Solver Submits Bid
// ============================================================

#[tokio::test]
async fn test_solver_submits_bid() {
    let client = http();
    let auth = register(&client).await;
    let account = get_account(&client, &auth.token, &auth.user_id).await;
    deposit(&client, &auth.token, &account.id, "ETH", 50000).await;

    // Create intent
    let intent_resp = client
        .post(format!("{}/intents", base_url()))
        .header("Authorization", format!("Bearer {}", auth.token))
        .json(&json!({
            "user_id": auth.user_id,
            "account_id": account.id,
            "token_in": "ETH",
            "token_out": "USDC",
            "amount_in": 1000,
            "min_amount_out": 900,
            "deadline": chrono::Utc::now().timestamp() + 3600
        }))
        .send()
        .await
        .unwrap();
    let intent: Intent = intent_resp.json().await.unwrap();

    // Submit bid
    let resp = client
        .post(format!("{}/bids", base_url()))
        .header("Authorization", format!("Bearer {}", auth.token))
        .json(&json!({
            "intent_id": intent.id,
            "solver_id": "e2e-solver",
            "amount_out": 950,
            "fee": 10
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let bid: Bid = resp.json().await.unwrap();
    assert_eq!(bid.intent_id, intent.id);
    assert_eq!(bid.solver_id, "e2e-solver");
    assert_eq!(bid.amount_out, 950);
}

// ============================================================
// Test 6: Idempotency Keys
// ============================================================

#[tokio::test]
async fn test_idempotency_key() {
    let client = http();
    let auth = register(&client).await;
    let account = get_account(&client, &auth.token, &auth.user_id).await;
    deposit(&client, &auth.token, &account.id, "ETH", 50000).await;

    let idem_key = Uuid::new_v4().to_string();
    let body = json!({
        "user_id": auth.user_id,
        "account_id": account.id,
        "token_in": "ETH",
        "token_out": "USDC",
        "amount_in": 500,
        "min_amount_out": 400,
        "deadline": chrono::Utc::now().timestamp() + 3600
    });

    // First request
    let resp1 = client
        .post(format!("{}/intents", base_url()))
        .header("Authorization", format!("Bearer {}", auth.token))
        .header("Idempotency-Key", &idem_key)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::CREATED);
    let intent1: Intent = resp1.json().await.unwrap();

    // Second request with same key — should return cached response
    let resp2 = client
        .post(format!("{}/intents", base_url()))
        .header("Authorization", format!("Bearer {}", auth.token))
        .header("Idempotency-Key", &idem_key)
        .json(&body)
        .send()
        .await
        .unwrap();

    // Should succeed (cached) and return same intent
    let replay_header = resp2.headers().get("x-idempotent-replay");
    let body2: serde_json::Value = resp2.json().await.unwrap();
    let id2 = body2["id"].as_str().unwrap_or("");
    assert_eq!(id2, intent1.id);
}

// ============================================================
// Test 7: Withdraw
// ============================================================

#[tokio::test]
async fn test_withdraw() {
    let client = http();
    let auth = register(&client).await;
    let account = get_account(&client, &auth.token, &auth.user_id).await;
    deposit(&client, &auth.token, &account.id, "USDC", 10000).await;

    let resp = client
        .post(format!("{}/balances/withdraw", base_url()))
        .header("Authorization", format!("Bearer {}", auth.token))
        .json(&json!({ "account_id": account.id, "asset": "USDC", "amount": 3000 }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let balance: Balance = resp.json().await.unwrap();
    assert_eq!(balance.available_balance, 7000);

    // Withdraw more than available
    let resp = client
        .post(format!("{}/balances/withdraw", base_url()))
        .header("Authorization", format!("Bearer {}", auth.token))
        .json(&json!({ "account_id": account.id, "asset": "USDC", "amount": 99999 }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ============================================================
// Test 8: Health Check
// ============================================================

#[tokio::test]
async fn test_health_endpoints() {
    let client = http();

    let resp = client
        .get(format!("{}/health/live", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = client
        .get(format!("{}/health/ready", base_url()))
        .send()
        .await
        .unwrap();
    // May be 200 or 503 depending on Redis
    assert!(resp.status() == StatusCode::OK || resp.status() == StatusCode::SERVICE_UNAVAILABLE);
}

// ============================================================
// Test 9: Weak Password Rejection
// ============================================================

#[tokio::test]
async fn test_weak_password_rejected() {
    let client = http();
    let email = format!("e2e-{}@test.com", Uuid::new_v4());

    let resp = client
        .post(format!("{}/auth/register", base_url()))
        .json(&json!({ "email": email, "password": "short" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ============================================================
// Test 10: Unauthorized Access
// ============================================================

#[tokio::test]
async fn test_unauthorized_access() {
    let client = http();

    // No token
    let resp = client
        .get(format!("{}/intents", base_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Bad token
    let resp = client
        .get(format!("{}/intents", base_url()))
        .header("Authorization", "Bearer invalid-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
