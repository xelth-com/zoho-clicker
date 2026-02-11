use anyhow::{Context, Result};
use std::time::Duration;
use thirtyfour::prelude::*;
use tokio::time::sleep;

const ZOHO_URL: &str =
    "https://people.zoho.eu/20086748177/zp#home/myspace/overview-actionlist";

/// How long to wait for the page / SPA to finish rendering.
const PAGE_LOAD_WAIT: Duration = Duration::from_secs(8);
/// How long to wait after clicking the button.
const POST_CLICK_WAIT: Duration = Duration::from_secs(4);

#[tokio::main]
async fn main() -> Result<()> {
    // ── 1. Connect to ChromeDriver with existing user profile ──
    let mut caps = DesiredCapabilities::chrome();

    // Reuse existing Chrome profile so Zoho session cookies are available.
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "Dmytro".into());
    let user_data_dir = format!(
        r"C:\Users\{}\AppData\Local\Google\Chrome\User Data",
        user
    );
    caps.add_arg(&format!("--user-data-dir={}", user_data_dir))?;
    caps.add_arg("--profile-directory=Default")?;
    // Disable "Chrome is being controlled by automated software" bar.
    caps.add_arg("--disable-infobars")?;
    caps.add_exclude_switch("enable-automation")?;

    println!("[*] Connecting to ChromeDriver on http://localhost:9515 ...");
    let driver = WebDriver::new("http://localhost:9515", caps)
        .await
        .context(
            "Failed to connect to ChromeDriver.\n\
             Make sure chromedriver.exe is running:\n  \
             chromedriver.exe --port=9515\n\
             Also close ALL existing Chrome windows first \
             (profile lock).",
        )?;

    // ── 2. Navigate to Zoho People ──
    println!("[*] Navigating to Zoho People ...");
    driver.goto(ZOHO_URL).await?;
    sleep(PAGE_LOAD_WAIT).await;

    // ── 3. Find the check-in / check-out button ──
    //
    // Zoho People attendance widget – common selectors (may need tweaking):
    //   - The main punch/check-in button
    //   - Usually inside a widget with class containing "attendance" or "checkin"
    //
    // We try several selectors from most to least specific.
    let selectors = [
        // Zoho People attendance punch button (common)
        r#"button.punchbutton"#,
        r#".att-punch-btn"#,
        r#".checkin-checkout-btn"#,
        r#"[data-lc="checkin"]"#,
        r#"[data-lc="checkout"]"#,
        r#".ztm-punch-btn"#,
        // Generic fallback: a prominent action button inside the overview
        r#".overview-actionlist button"#,
        r#".zp_action_btn"#,
    ];

    let button = find_first_match(&driver, &selectors).await.context(
        "Could not find the check-in/check-out button.\n\
         Run with --inspect to see page elements, \
         then update the `selectors` list in main.rs.",
    )?;

    // ── 4. State BEFORE click ──
    let text_before = button.text().await.unwrap_or_default();
    let class_before = button
        .attr("class")
        .await?
        .unwrap_or_default();
    println!();
    println!("[BEFORE] text  = {:?}", text_before);
    println!("[BEFORE] class = {:?}", class_before);

    // ── 5. Click ──
    println!();
    println!("[*] Clicking the button ...");
    button.click().await?;
    sleep(POST_CLICK_WAIT).await;

    // ── 6. State AFTER click ──
    // Re-query – the DOM may have changed.
    let button_after = find_first_match(&driver, &selectors).await;
    match button_after {
        Some(btn) => {
            let text_after = btn.text().await.unwrap_or_default();
            let class_after = btn.attr("class").await?.unwrap_or_default();
            println!();
            println!("[AFTER]  text  = {:?}", text_after);
            println!("[AFTER]  class = {:?}", class_after);

            if text_before != text_after || class_before != class_after {
                println!();
                println!("[OK] Button state changed!");
            } else {
                println!();
                println!("[WARN] Button state looks the same – check manually.");
            }
        }
        None => {
            println!("[WARN] Button disappeared after click (may be normal).");
        }
    }

    // Keep the browser open so the user can verify.
    println!();
    println!("[*] Done. Press Ctrl+C to exit (browser stays open).");
    tokio::signal::ctrl_c().await?;
    driver.quit().await?;

    Ok(())
}

/// Try each CSS selector and return the first element found.
async fn find_first_match(
    driver: &WebDriver,
    selectors: &[&str],
) -> Option<WebElement> {
    for sel in selectors {
        if let Ok(el) = driver.find(By::Css(*sel)).await {
            // Make sure the element is displayed / interactable.
            if el.is_displayed().await.unwrap_or(false) {
                return Some(el);
            }
        }
    }
    None
}
