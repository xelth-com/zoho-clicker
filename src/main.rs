use anyhow::{Context, Result};
use std::time::Duration;
use thirtyfour::prelude::*;
use tokio::time::sleep;

// ── Configuration ──
const EMAIL: &str = "d.suro@inbody.com";
const PASSWORD: &str = "dima13.,Dima241";

const LOGIN_URL: &str = "https://accounts.zoho.eu/signin?servicename=zohopeople";
const PEOPLE_URL: &str = "https://people.zoho.eu/20086748177/zp#home/myspace/overview-actionlist";

const PAGE_LOAD_WAIT: Duration = Duration::from_secs(8);
const SHORT_WAIT: Duration = Duration::from_secs(3);
const POST_CLICK_WAIT: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> Result<()> {
    // ── 1. Connect to ChromeDriver ──
    let mut caps = DesiredCapabilities::chrome();
    caps.add_arg("--disable-infobars")?;
    caps.add_exclude_switch("enable-automation")?;

    println!("[*] Connecting to ChromeDriver on http://localhost:9515 ...");
    let driver = WebDriver::new("http://localhost:9515", caps)
        .await
        .context(
            "Failed to connect to ChromeDriver.\n\
             Make sure chromedriver.exe is running:\n  \
             chromedriver.exe --port=9515",
        )?;

    // ── 2. Login to Zoho ──
    println!("[*] Navigating to Zoho login ...");
    driver.goto(LOGIN_URL).await?;
    sleep(PAGE_LOAD_WAIT).await;

    // Step 1: Enter email
    println!("[*] Entering email ...");
    let email_input = driver
        .find(By::Id("login_id"))
        .await
        .context("Could not find email input #login_id")?;
    email_input.clear().await?;
    email_input.send_keys(EMAIL).await?;

    // Click "Weiter" (Next)
    let next_btn = driver
        .find(By::Id("nextbtn"))
        .await
        .context("Could not find #nextbtn")?;
    next_btn.click().await?;
    sleep(SHORT_WAIT).await;

    // Step 2: Enter password
    println!("[*] Entering password ...");
    let pass_input = wait_for_element(&driver, By::Id("password"), 10).await
        .context("Could not find password input #password")?;
    pass_input.clear().await?;
    pass_input.send_keys(PASSWORD).await?;

    // Click "Anmelden" (Sign in) — same #nextbtn id
    let sign_in_btn = driver
        .find(By::Id("nextbtn"))
        .await
        .context("Could not find sign-in button #nextbtn")?;
    sign_in_btn.click().await?;

    println!("[*] Waiting for login to complete ...");
    sleep(PAGE_LOAD_WAIT).await;

    // ── 3. Navigate to Zoho People dashboard ──
    let current_url = driver.current_url().await?.to_string();
    if !current_url.contains("people.zoho.eu") {
        println!("[*] Navigating to Zoho People ...");
        driver.goto(PEOPLE_URL).await?;
        sleep(PAGE_LOAD_WAIT).await;
    }

    // Wait for the SPA to fully load — look for the check-in button
    println!("[*] Waiting for dashboard to load ...");
    let button = wait_for_element(&driver, By::Id("ZPAtt_check_in_out"), 30).await
        .context("Could not find check-in/check-out button #ZPAtt_check_in_out")?;

    // ── 4. Read state BEFORE click ──
    let status_before = get_status(&driver).await;
    let btn_text_before = button.text().await.unwrap_or_default();
    let aria_before = button
        .attr("aria-label")
        .await?
        .unwrap_or_default();

    println!();
    println!("[BEFORE] status     = {:?}", status_before);
    println!("[BEFORE] button     = {:?}", btn_text_before);
    println!("[BEFORE] aria-label = {:?}", aria_before);

    // ── 5. Click the button ──
    println!();
    println!("[*] Clicking '{}' ...", btn_text_before);
    button.click().await?;
    sleep(POST_CLICK_WAIT).await;

    // ── 6. Read state AFTER click ──
    let status_after = get_status(&driver).await;
    let btn_after = driver.find(By::Id("ZPAtt_check_in_out")).await;
    let btn_text_after = match &btn_after {
        Ok(b) => b.text().await.unwrap_or_default(),
        Err(_) => "(button gone)".into(),
    };
    let aria_after = match &btn_after {
        Ok(b) => b.attr("aria-label").await.ok().flatten().unwrap_or_default(),
        Err(_) => String::new(),
    };

    println!();
    println!("[AFTER]  status     = {:?}", status_after);
    println!("[AFTER]  button     = {:?}", btn_text_after);
    println!("[AFTER]  aria-label = {:?}", aria_after);

    // ── 7. Compare ──
    println!();
    if status_before != status_after || btn_text_before != btn_text_after {
        println!("[OK] State changed: '{}' -> '{}'", btn_text_before, btn_text_after);
    } else {
        println!("[WARN] State looks the same — check manually.");
    }

    // Keep browser open for verification.
    println!();
    println!("[*] Done. Press Ctrl+C to exit (browser stays open).");
    tokio::signal::ctrl_c().await?;
    driver.quit().await?;

    Ok(())
}

/// Read the attendance status text from `#att_status`.
async fn get_status(driver: &WebDriver) -> String {
    match driver.find(By::Id("att_status")).await {
        Ok(el) => el.text().await.unwrap_or_default(),
        Err(_) => "(not found)".into(),
    }
}

/// Poll for an element to appear (useful for SPA transitions).
async fn wait_for_element(
    driver: &WebDriver,
    by: By,
    timeout_secs: u64,
) -> Result<WebElement, anyhow::Error> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        if let Ok(el) = driver.find(by.clone()).await {
            if el.is_displayed().await.unwrap_or(false) {
                return Ok(el);
            }
        }
        if tokio::time::Instant::now() > deadline {
            anyhow::bail!("Timed out waiting for element after {}s", timeout_secs);
        }
        sleep(Duration::from_millis(500)).await;
    }
}
