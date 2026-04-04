use std::sync::Arc;

use reqwest::Client;
use uuid::Uuid;

pub struct TestUser {
    pub user_id: String,
    pub account_id: String,
    pub email: String,
}

pub async fn create_test_users(
    client: &Client,
    base_url: &str,
    count: u64,
) -> Vec<Arc<TestUser>> {
    let mut users = Vec::new();

    for i in 0..count {
        let email = format!("loadtest-{}@test.com", Uuid::new_v4());
        let password = "loadtest123456";

        // Register
        let reg = client
            .post(format!("{base_url}/users/register"))
            .json(&serde_json::json!({
                "email": email,
                "password": password,
            }))
            .send()
            .await;

        let (user_id, account_id) = match reg {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let uid = body["user_id"].as_str().unwrap_or("").to_string();

                // Fetch account
                let acc_resp = client
                    .get(format!("{base_url}/accounts/{uid}"))
                    .send()
                    .await;
                let aid = match acc_resp {
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
                (uid, aid)
            }
            _ => {
                // Fallback: use fake IDs for testing without DB
                (Uuid::new_v4().to_string(), Uuid::new_v4().to_string())
            }
        };

        users.push(Arc::new(TestUser {
            user_id,
            account_id,
            email,
        }));

        if (i + 1) % 10 == 0 {
            println!("  Created {}/{} users", i + 1, count);
        }
    }

    println!("  {} test users ready", users.len());
    users
}
