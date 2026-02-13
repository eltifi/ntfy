use clap::Args;
use crate::config::Config;
use crate::server;
use reqwest::Client;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, error};
use colored::Colorize;

#[derive(Args, Debug)]
pub struct TestArgs {}

pub async fn handle_test_command(_args: TestArgs, mut config: Config) -> anyhow::Result<()> {
    info!("Starting integration tests...");

    // 1. Setup temporary environment
    let temp_dir = tempfile::tempdir()?;
    let db_path = temp_dir.path().join("test.db");
    let attachments_dir = temp_dir.path().join("attachments");
    std::fs::create_dir_all(&attachments_dir)?;

    config.cache_file = db_path.clone();
    config.auth_file = db_path.clone();
    config.attachment_cache_dir = attachments_dir;
    config.listen_http = "127.0.0.1:12345".to_string(); 
    let base_url = "http://127.0.0.1:12345";
    config.base_url = base_url.to_string();
    
    let config_for_server = config.clone();
    
    tokio::spawn(async move {
        if let Err(e) = server::run(config_for_server).await {
            error!("Server error during test: {}", e);
        }
    });

    info!("Server started on {}", base_url);
    // Give it a moment to start
    sleep(Duration::from_millis(1000)).await;

    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?;
    
    let base_url = "http://127.0.0.1:12345";
    let mut results = Vec::new();

    // --- TEST SUITE ---

    // 1. Health
    results.push(test_feature("Health Check", async {
        let resp = client.get(format!("{}/v1/health", base_url)).send().await?;
        if resp.status().is_success() { Ok(()) } else { Err(anyhow::anyhow!("Status {}", resp.status())) }
    }).await);

    // 2. Config
    results.push(test_feature("Config Retrieval", async {
        let resp = client.get(format!("{}/v1/config", base_url)).send().await?;
        if resp.status().is_success() { Ok(()) } else { Err(anyhow::anyhow!("Status {}", resp.status())) }
    }).await);

    // 3. Publishing
    results.push(test_feature("Publishing (Simple)", async {
        let resp = client.post(format!("{}/test_topic", base_url))
            .body("Hello from test")
            .send().await?;
        if resp.status().is_success() { Ok(()) } else { Err(anyhow::anyhow!("Status {}", resp.status())) }
    }).await);

    results.push(test_feature("Publishing (JSON)", async {
        let resp = client.post(format!("{}/", base_url))
            .header("Content-Type", "application/json")
            .body(r#"{"topic": "test_json", "message": "Hello JSON"}"#)
            .send().await?;
        if resp.status().is_success() { Ok(()) } else { Err(anyhow::anyhow!("Status {}", resp.status())) }
    }).await);

    // 4. Matrix Gateway
    results.push(test_feature("Matrix Discovery", async {
        let resp = client.get(format!("{}/_matrix/push/v1/notify", base_url)).send().await?;
        let body = resp.text().await?;
        if body.contains("matrix") { Ok(()) } else { Err(anyhow::anyhow!("Body doesn't contain matrix: {}", body)) }
    }).await);

    results.push(test_feature("Matrix Push", async {
        let body = r#"{"notification": {"devices": [{"pushkey": "http://127.0.0.1:12345/upABC"}]}}"#;
        let resp = client.post(format!("{}/_matrix/push/v1/notify", base_url))
            .header("Content-Type", "application/json")
            .body(body)
            .send().await?;
        if resp.status().is_success() { Ok(()) } else { Err(anyhow::anyhow!("Status {}", resp.status())) }
    }).await);

    // 5. Subscriptions (Poll)
    results.push(test_feature("Subscriptions (Poll Mode)", async {
        let resp = client.get(format!("{}/test_topic/json?poll=1&since=all", base_url)).send().await?;
        if resp.status().is_success() { Ok(()) } else { Err(anyhow::anyhow!("Status {}", resp.status())) }
    }).await);

    // 6. Attachments
    results.push(test_feature("File Attachments (Upload)", async {
        let resp = client.put(format!("{}/test_topic", base_url))
            .header("Filename", "test.txt")
            .body("this is a test file")
            .send().await?;
        if resp.status().is_success() { Ok(()) } else { Err(anyhow::anyhow!("Status {}", resp.status())) }
    }).await);

    // 7. Actions Parsing
    results.push(test_feature("Action Buttons (Simple format)", async {
        let resp = client.post(format!("{}/test_actions", base_url))
            .header("Actions", "view, Open portal, https://ntfy.sh")
            .body("Check actions")
            .send().await?;
        if resp.status().is_success() { Ok(()) } else { Err(anyhow::anyhow!("Status {}", resp.status())) }
    }).await);

    // 8. Scheduled Delivery
    results.push(test_feature("Scheduled Delivery (Delay header)", async {
        let resp = client.post(format!("{}/test_delay", base_url))
            .header("Delay", "20s")
            .body("Delayed message")
            .send().await?;
        if resp.status().is_success() { Ok(()) } else { Err(anyhow::anyhow!("Status {}", resp.status())) }
    }).await);

    // 9. Web UI
    results.push(test_feature("Web UI Assets", async {
        let resp = client.get(format!("{}/index.html", base_url)).send().await?;
        if resp.status().is_success() { Ok(()) } else { Err(anyhow::anyhow!("Status {}", resp.status())) }
    }).await);

    // --- REPORT ---
    println!("\n{}", "Integration Test Report".bold().underline());
    let mut success_count = 0;
    for (name, result) in &results {
        let status = if result.is_ok() {
            success_count += 1;
            "PASS".green()
        } else {
            "FAIL".red()
        };
        println!("{:<35} {}", name, status);
        if let Err(e) = result {
            println!("   -> {}", e.to_string().dimmed());
        }
    }
    
    println!("\nSummary: {}/{} tests passed.\n", success_count, results.len());

    if success_count == results.len() {
        info!("All core features verified successfully.");
        Ok(())
    } else {
        Err(anyhow::anyhow!("Some core features failed verification"))
    }
}

async fn test_feature<Fut>(name: &'static str, f: Fut) -> (&'static str, Result<(), anyhow::Error>) 
where 
    Fut: std::future::Future<Output = Result<(), anyhow::Error>>
{
    (name, f.await)
}
