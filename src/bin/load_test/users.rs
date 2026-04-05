use std::sync::Arc;

use reqwest::Client;
use uuid::Uuid;

pub struct TestUser {
    pub user_id: String,
    pub account_id: String,
    pub token: String,
    pub email: String,
}

/// Create `count` test users in parallel batches.
pub async fn create_test_users(
    client: &Client,
    base_url: &str,
    count: u64,
) -> Vec<Arc<TestUser>> {
    let mut users = Vec::with_capacity(count as usize);

    // Batch creation in groups of 50 for concurrency
    let batch_size = 50;
    let mut created = 0u64;

    while created < count {
        let batch = (count - created).min(batch_size);
        let mut handles = Vec::with_capacity(batch as usize);

        for _ in 0..batch {
            let c = client.clone();
            let url = base_url.to_string();
            handles.push(tokio::spawn(async move {
                create_single_user(&c, &url).await
            }));
        }

        for h in handles {
            if let Ok(Some(user)) = h.await {
                users.push(Arc::new(user));
            }
        }

        created += batch;
        if created % 100 == 0 || created >= count {
            println!("  Created {}/{} users", users.len(), count);
        }
    }

    println!("  {} test users ready", users.len());
    users
}

async fn create_single_user(client: &Client, base_url: &str) -> Option<TestUser> {
    let email = format!("loadtest-{}@test.com", Uuid::new_v4());
    let password = "LoadTest!Passw0rd";

    // Register
    let resp = client
        .post(format!("{base_url}/users/register"))
        .json(&serde_json::json!({
            "email": email,
            "password": password,
        }))
        .send()
        .await;

    let (user_id, token) = match resp {
        Ok(r) if r.status().is_success() => {
            let body: serde_json::Value = r.json().await.unwrap_or_default();
            let uid = body["user_id"].as_str().unwrap_or("").to_string();
            let tok = body["token"].as_str().unwrap_or("").to_string();
            (uid, tok)
        }
        _ => {
            return Some(TestUser {
                user_id: Uuid::new_v4().to_string(),
                account_id: Uuid::new_v4().to_string(),
                token: String::new(),
                email,
            });
        }
    };

    // Fetch account
    let account_id = match client
        .get(format!("{base_url}/accounts/{user_id}"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => {
            let accounts: Vec<serde_json::Value> = r.json().await.unwrap_or_default();
            accounts
                .first()
                .and_then(|a| a["id"].as_str())
                .unwrap_or("")
                .to_string()
        }
        _ => String::new(),
    };

    // Seed balance so intents don't get rejected
    let _ = client
        .post(format!("{base_url}/balances/deposit"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "account_id": account_id,
            "asset": "USDC",
            "amount": 1_000_000,
        }))
        .send()
        .await;
    let _ = client
        .post(format!("{base_url}/balances/deposit"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "account_id": account_id,
            "asset": "ETH",
            "amount": 1_000_000,
        }))
        .send()
        .await;

    Some(TestUser {
        user_id,
        account_id,
        token,
        email,
    })
}
