// Hide console window on Windows (logs go to zoho-clicker.log)
#![windows_subsystem = "windows"]

use anyhow::{Context, Result};
use chrono::{Datelike, Local, NaiveTime, Weekday};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use thirtyfour::prelude::*;
use tokio::time::sleep;

// ── Defaults (overridden by config.env) ──
const DEFAULT_CHECKIN_START: &str = "08:50";
const DEFAULT_CHECKIN_END: &str = "10:00";
const DEFAULT_CHECKOUT_AFTER: &str = "18:00";

const PEOPLE_URL: &str =
    "https://people.zoho.eu/20086748177/zp#home/myspace/overview-actionlist";

const PAGE_LOAD_WAIT: Duration = Duration::from_secs(10);
const SHORT_WAIT: Duration = Duration::from_secs(4);
const POST_CLICK_WAIT: Duration = Duration::from_secs(5);
const LOOP_SLEEP: Duration = Duration::from_secs(60);          // 1 min between checks
const CHECKIN_RETRY_SLEEP: Duration = Duration::from_secs(600); // 10 min retry
const WEEKEND_SLEEP: Duration = Duration::from_secs(3600);     // 1 hour on weekends
const WARNING_WAIT: Duration = Duration::from_secs(60);        // 1 min after popup

/// What action the daemon wants to perform.
#[derive(Clone, Copy, PartialEq)]
enum Action {
    CheckIn,
    CheckOut,
}

// ── State file ──
#[derive(Serialize, Deserialize, Default)]
struct AppState {
    last_checkin: Option<String>,
    last_checkout: Option<String>,
}

// ── Config ──
struct Config {
    email: String,
    password: String,
    checkin_start: NaiveTime,
    checkin_end: NaiveTime,
    checkout_after: NaiveTime,
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn chrome_profile_dir() -> PathBuf {
    exe_dir().join("chrome-profile")
}

fn state_path() -> PathBuf {
    exe_dir().join("state.json")
}

fn load_state() -> AppState {
    let path = state_path();
    if path.exists() {
        let data = std::fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        AppState::default()
    }
}

fn save_state(state: &AppState) {
    let path = state_path();
    if let Ok(json) = serde_json::to_string_pretty(state) {
        let _ = std::fs::write(path, json);
    }
}

fn today_str() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn load_config() -> Result<Config> {
    let env_path = exe_dir().join("config.env");
    if env_path.exists() {
        dotenvy::from_path(&env_path).ok();
    } else {
        dotenvy::dotenv().ok();
    }

    let email = std::env::var("ZOHO_EMAIL")
        .context("ZOHO_EMAIL not set in config.env")?;
    let password = std::env::var("ZOHO_PASSWORD")
        .context("ZOHO_PASSWORD not set in config.env")?;

    let parse_time = |key: &str, default: &str| -> NaiveTime {
        let val = std::env::var(key).unwrap_or_else(|_| default.to_string());
        NaiveTime::parse_from_str(&val, "%H:%M").unwrap_or_else(|_| {
            NaiveTime::parse_from_str(default, "%H:%M").unwrap()
        })
    };

    Ok(Config {
        email,
        password,
        checkin_start: parse_time("CHECKIN_START", DEFAULT_CHECKIN_START),
        checkin_end: parse_time("CHECKIN_END", DEFAULT_CHECKIN_END),
        checkout_after: parse_time("CHECKOUT_AFTER", DEFAULT_CHECKOUT_AFTER),
    })
}

fn setup_logging() {
    use simplelog::*;

    let log_path = exe_dir().join("zoho-clicker.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();

    let mut loggers: Vec<Box<dyn SharedLogger>> = vec![
        TermLogger::new(
            LevelFilter::Info,
            simplelog::Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
    ];

    if let Some(file) = log_file {
        loggers.push(WriteLogger::new(
            LevelFilter::Info,
            simplelog::Config::default(),
            file,
        ));
    }

    CombinedLogger::init(loggers).ok();
}

/// Show a Windows MessageBox warning before check-in.
#[cfg(windows)]
fn show_warning_popup() {
    use windows::core::*;
    use windows::Win32::UI::WindowsAndMessaging::*;

    let title = w!("Zoho Clicker");
    let msg = w!("Check-in \u{0447}\u{0435}\u{0440}\u{0435}\u{0437} 1 \u{043c}\u{0438}\u{043d}\u{0443}\u{0442}\u{0443}!\n\n\u{041d}\u{0430}\u{0436}\u{043c}\u{0438}\u{0442}\u{0435} OK \u{0438}\u{043b}\u{0438} \u{043f}\u{043e}\u{0434}\u{043e}\u{0436}\u{0434}\u{0438}\u{0442}\u{0435}.");

    unsafe {
        let _ = MessageBoxW(None, msg, title, MB_OK | MB_ICONWARNING | MB_SYSTEMMODAL);
    }
}

#[cfg(not(windows))]
fn show_warning_popup() {
    info!("(popup not available on this platform)");
}

/// Start a process with no visible console window (Windows).
#[cfg(windows)]
fn start_hidden(exe: &std::path::Path, args: &[&str]) -> Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    Command::new(exe)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("Failed to spawn process")
}

#[cfg(not(windows))]
fn start_hidden(exe: &std::path::Path, args: &[&str]) -> Result<std::process::Child> {
    Command::new(exe)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn process")
}

/// Detect installed Chrome version via PowerShell.
fn get_chrome_version() -> Option<String> {
    let paths = [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ];
    for path in &paths {
        let output = Command::new("powershell")
            .args(["-Command", &format!(
                "(Get-Item '{}').VersionInfo.FileVersion", path
            )])
            .output()
            .ok()?;
        if output.status.success() {
            let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !ver.is_empty() && ver.contains('.') {
                return Some(ver);
            }
        }
    }
    None
}

/// Get chromedriver version by running `chromedriver --version`.
fn get_chromedriver_version(driver_path: &std::path::Path) -> Option<String> {
    let output = Command::new(driver_path)
        .arg("--version")
        .output()
        .ok()?;
    // Output: "ChromeDriver 144.0.7559.133 (abc123...)"
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace().nth(1).map(|s| s.to_string())
}

/// Extract major.minor.build from a full version string (drop patch).
fn version_prefix(ver: &str) -> String {
    let parts: Vec<&str> = ver.split('.').collect();
    if parts.len() >= 3 {
        format!("{}.{}.{}", parts[0], parts[1], parts[2])
    } else {
        ver.to_string()
    }
}

/// Download chromedriver matching the given Chrome version.
async fn download_chromedriver(chrome_version: &str, dest: &std::path::Path) -> Result<()> {
    let url = format!(
        "https://storage.googleapis.com/chrome-for-testing-public/{}/win64/chromedriver-win64.zip",
        chrome_version
    );
    info!("Downloading chromedriver {} ...", chrome_version);
    info!("URL: {}", url);

    let resp = reqwest::get(&url).await
        .context("Failed to download chromedriver")?;

    if !resp.status().is_success() {
        // Try with just major.minor.build.0 if exact version fails
        let prefix = version_prefix(chrome_version);
        let fallback_url = format!(
            "https://storage.googleapis.com/chrome-for-testing-public/{}.0/win64/chromedriver-win64.zip",
            prefix
        );
        info!("Exact version not found, trying {} ...", fallback_url);
        let resp2 = reqwest::get(&fallback_url).await
            .context("Failed to download chromedriver (fallback)")?;
        if !resp2.status().is_success() {
            anyhow::bail!(
                "ChromeDriver download failed: HTTP {} for both {} and {}",
                resp2.status(), url, fallback_url
            );
        }
        let bytes = resp2.bytes().await?;
        extract_chromedriver_zip(&bytes, dest)?;
    } else {
        let bytes = resp.bytes().await?;
        extract_chromedriver_zip(&bytes, dest)?;
    }

    Ok(())
}

/// Extract chromedriver.exe from the downloaded zip using PowerShell.
fn extract_chromedriver_zip(zip_bytes: &[u8], dest: &std::path::Path) -> Result<()> {
    let dir = dest.parent().unwrap_or(std::path::Path::new("."));
    let zip_path = dir.join("_chromedriver_tmp.zip");
    let extract_dir = dir.join("_chromedriver_tmp");

    // Write zip to disk
    std::fs::write(&zip_path, zip_bytes)
        .context("Failed to write chromedriver zip")?;

    // Remove old extract dir if exists
    let _ = std::fs::remove_dir_all(&extract_dir);

    // Extract using PowerShell
    let status = Command::new("powershell")
        .args([
            "-Command",
            &format!(
                "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                zip_path.display(),
                extract_dir.display()
            ),
        ])
        .status()
        .context("Failed to run PowerShell Expand-Archive")?;

    if !status.success() {
        anyhow::bail!("PowerShell Expand-Archive failed");
    }

    // Move chromedriver.exe to destination
    let extracted = extract_dir.join("chromedriver-win64").join("chromedriver.exe");
    if extracted.exists() {
        // Remove old chromedriver if exists (might be locked)
        let _ = std::fs::remove_file(dest);
        std::fs::copy(&extracted, dest)
            .context("Failed to copy chromedriver.exe")?;
        info!("Installed chromedriver to {}", dest.display());
    } else {
        anyhow::bail!("chromedriver.exe not found in zip at {:?}", extracted);
    }

    // Cleanup
    let _ = std::fs::remove_file(&zip_path);
    let _ = std::fs::remove_dir_all(&extract_dir);

    Ok(())
}

/// Ensure chromedriver.exe exists, matches Chrome version, and is running.
async fn ensure_chromedriver() {
    let driver_path = exe_dir().join("chromedriver.exe");

    // ── Check versions and download if needed ──
    let chrome_ver = get_chrome_version();
    match &chrome_ver {
        Some(cv) => info!("Chrome version: {}", cv),
        None => warn!("Could not detect Chrome version"),
    }

    let need_download = if !driver_path.exists() {
        info!("chromedriver.exe not found, will download");
        true
    } else if let (Some(cv), Some(dv)) = (&chrome_ver, &get_chromedriver_version(&driver_path)) {
        let chrome_prefix = version_prefix(cv);
        let driver_prefix = version_prefix(dv);
        if chrome_prefix != driver_prefix {
            info!(
                "Version mismatch: Chrome {} vs ChromeDriver {} — will update",
                cv, dv
            );
            // Kill old chromedriver before replacing
            let _ = Command::new("taskkill")
                .args(["/F", "/IM", "chromedriver.exe"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            sleep(Duration::from_secs(1)).await;
            true
        } else {
            info!("ChromeDriver version {} matches Chrome", dv);
            false
        }
    } else {
        false
    };

    if need_download {
        if let Some(cv) = &chrome_ver {
            match download_chromedriver(cv, &driver_path).await {
                Ok(()) => info!("ChromeDriver updated successfully"),
                Err(e) => error!("Failed to download ChromeDriver: {:#}", e),
            }
        } else {
            error!("Cannot download ChromeDriver: unknown Chrome version");
        }
    }

    // ── Start chromedriver if not running ──
    if std::net::TcpStream::connect("127.0.0.1:9515").is_ok() {
        info!("ChromeDriver already running on port 9515");
        return;
    }

    if driver_path.exists() {
        match start_hidden(&driver_path, &["--port=9515", "--silent", "--log-level=OFF"]) {
            Ok(_) => info!("Started chromedriver from {}", driver_path.display()),
            Err(e) => error!("Failed to start chromedriver: {}", e),
        }
    } else {
        error!("chromedriver.exe not found and could not be downloaded");
    }
}

/// Log in to Zoho, navigate to People, and perform the requested action.
/// Returns Ok(true) if the action was performed, Ok(false) if the button
/// was already in the desired state (nothing to do).
async fn zoho_action(config: &Config, action: Action) -> Result<bool> {
    let mut caps = DesiredCapabilities::chrome();
    caps.add_arg("--disable-infobars")?;
    // Dedicated profile directory — avoids Chrome's profile picker dialog
    let profile_dir = chrome_profile_dir();
    caps.add_arg(&format!("--user-data-dir={}", profile_dir.display()))?;
    caps.add_arg("--window-size=1280,900")?;
    caps.add_arg("--window-position=-2000,0")?; // off-screen
    caps.add_arg("--disable-logging")?;
    caps.add_arg("--log-level=3")?; // suppress Chrome console output
    caps.add_exclude_switch("enable-automation")?;

    // Ensure chromedriver is running before every action (it may have died)
    ensure_chromedriver().await;
    sleep(Duration::from_secs(2)).await;

    info!("Connecting to ChromeDriver ...");
    let driver = WebDriver::new("http://localhost:9515", caps)
        .await
        .context("Failed to connect to ChromeDriver on port 9515")?;

    let result = do_zoho_action(&driver, config, action).await;

    let _ = driver.quit().await;

    result
}

async fn do_zoho_action(
    driver: &WebDriver,
    config: &Config,
    action: Action,
) -> Result<bool> {
    let action_name = match action {
        Action::CheckIn => "Check-in",
        Action::CheckOut => "Check-out",
    };

    // ── Try navigating directly to People (may already be logged in) ──
    info!("Navigating to Zoho People ...");
    driver.goto(PEOPLE_URL).await?;
    sleep(PAGE_LOAD_WAIT).await;

    // Check if we landed on login page (redirected because not authenticated)
    let url = driver.current_url().await?.to_string();
    if url.contains("accounts.zoho.eu") {
        info!("Not logged in, performing login ...");

        // Email — wait for the field to appear
        let email_input = wait_for_element(driver, By::Id("login_id"), 15).await
            .context("Could not find #login_id on login page")?;
        email_input.clear().await?;
        email_input.send_keys(&config.email).await?;

        driver.find(By::Id("nextbtn")).await?.click().await?;
        sleep(SHORT_WAIT).await;

        // Password
        info!("Entering password ...");
        let pass_input = wait_for_element(driver, By::Id("password"), 15).await
            .context("Could not find #password")?;
        pass_input.clear().await?;
        pass_input.send_keys(&config.password).await?;

        driver.find(By::Id("nextbtn")).await?.click().await?;

        info!("Waiting for login ...");
        sleep(PAGE_LOAD_WAIT).await;

        // After login, navigate to People if not there yet
        let url2 = driver.current_url().await?.to_string();
        if !url2.contains("people.zoho.eu") {
            info!("Navigating to Zoho People after login ...");
            driver.goto(PEOPLE_URL).await?;
            sleep(PAGE_LOAD_WAIT).await;
        }
    } else {
        info!("Already logged in (url: {})", url);
    }

    // ── Find button ──
    info!("Waiting for attendance button ...");
    let button = wait_for_element(driver, By::Id("ZPAtt_check_in_out"), 60).await
        .context("Could not find #ZPAtt_check_in_out")?;

    let status = get_status(driver).await;
    let btn_text = button.text().await.unwrap_or_default();

    info!("Status: {:?}, Button: {:?}", status, btn_text);

    // ── Check if button matches desired action ──
    let btn_matches = match action {
        Action::CheckIn => btn_text.contains("Check-in"),
        Action::CheckOut => btn_text.contains("Check-out"),
    };

    if !btn_matches {
        info!(
            "Button shows '{}' but we want '{}' — already in desired state, skipping.",
            btn_text, action_name
        );
        return Ok(false);
    }

    // ── Click ──
    info!("Clicking '{}' ...", btn_text);
    button.click().await?;
    sleep(POST_CLICK_WAIT).await;

    // ── Verify ──
    let status_after = get_status(driver).await;
    let btn_after = match driver.find(By::Id("ZPAtt_check_in_out")).await {
        Ok(b) => b.text().await.unwrap_or_default(),
        Err(_) => "(gone)".into(),
    };

    info!(
        "After click — status: {:?}, button: {:?}",
        status_after, btn_after
    );

    Ok(true)
}

async fn get_status(driver: &WebDriver) -> String {
    match driver.find(By::Id("att_status")).await {
        Ok(el) => el.text().await.unwrap_or_default(),
        Err(_) => "(not found)".into(),
    }
}

async fn wait_for_element(
    driver: &WebDriver,
    by: By,
    timeout_secs: u64,
) -> Result<WebElement> {
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

// ═══════════════════════════════════════════════════════════
//  Main daemon loop
// ═══════════════════════════════════════════════════════════

#[tokio::main]
async fn main() -> Result<()> {
    setup_logging();
    info!("=== Zoho Clicker started ===");

    let config = load_config()?;
    info!(
        "Config: check-in {}–{}, check-out after {}",
        config.checkin_start, config.checkin_end, config.checkout_after
    );

    // Check Chrome/ChromeDriver versions and start chromedriver
    ensure_chromedriver().await;
    sleep(Duration::from_secs(2)).await;

    loop {
        let now = Local::now();
        let weekday = now.weekday();
        let time = now.time();
        let today = today_str();

        // ── Skip weekends ──
        if weekday == Weekday::Sat || weekday == Weekday::Sun {
            info!("Weekend ({}), sleeping 1 hour ...", weekday);
            sleep(WEEKEND_SLEEP).await;
            continue;
        }

        let state = load_state();

        // ── Check-in window (8:50 – 10:00) ──
        let already_checked_in = state
            .last_checkin
            .as_deref()
            .is_some_and(|d| d == today);

        if time >= config.checkin_start && time < config.checkin_end && !already_checked_in {
            info!("Check-in window active. Showing warning popup ...");
            std::thread::spawn(show_warning_popup);
            sleep(WARNING_WAIT).await;

            info!("Attempting check-in ...");
            match zoho_action(&config, Action::CheckIn).await {
                Ok(clicked) => {
                    if clicked {
                        info!("Check-in successful!");
                    } else {
                        info!("Already checked in (button was Check-out).");
                    }
                    let mut state = load_state();
                    state.last_checkin = Some(today.clone());
                    save_state(&state);
                }
                Err(e) => {
                    error!("Check-in failed: {:#}", e);
                }
            }
            sleep(CHECKIN_RETRY_SLEEP).await;
            continue;
        }

        // ── Check-out window (after 18:00) ──
        let already_checked_out = state
            .last_checkout
            .as_deref()
            .is_some_and(|d| d == today);

        if time >= config.checkout_after && !already_checked_out {
            info!("Check-out time. No warning, proceeding ...");
            match zoho_action(&config, Action::CheckOut).await {
                Ok(clicked) => {
                    if clicked {
                        info!("Check-out successful!");
                    } else {
                        info!("Not checked in today — nothing to check out.");
                    }
                }
                Err(e) => {
                    error!("Check-out failed: {:#}", e);
                }
            }
            // Mark as done regardless — don't retry
            let mut state = load_state();
            state.last_checkout = Some(today.clone());
            save_state(&state);
            sleep(LOOP_SLEEP).await;
            continue;
        }

        // ── Outside action windows ──
        sleep(LOOP_SLEEP).await;
    }
}
