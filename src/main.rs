// Hide console window on Windows (logs go to zoho-clicker.log)
#![windows_subsystem = "windows"]

use anyhow::{Context, Result};
use chrono::{Datelike, Local, NaiveTime, Weekday};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;
use thirtyfour::prelude::*;
use tokio::time::sleep;

// ── Defaults (overridden by config.env) ──
const DEFAULT_CHECKIN_START: &str = "08:50";
const DEFAULT_CHECKIN_END: &str = "10:00";
const DEFAULT_CHECKOUT_AFTER: &str = "18:00";

const LOGIN_URL: &str = "https://accounts.zoho.eu/signin?servicename=zohopeople";
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

/// Start chromedriver.exe if not already running.
fn ensure_chromedriver() -> Option<Child> {
    if std::net::TcpStream::connect("127.0.0.1:9515").is_ok() {
        info!("ChromeDriver already running on port 9515");
        return None;
    }

    let driver_path = exe_dir().join("chromedriver.exe");
    if !driver_path.exists() {
        warn!(
            "chromedriver.exe not found at {}. Trying PATH...",
            driver_path.display()
        );
        match Command::new("chromedriver")
            .arg("--port=9515")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => {
                info!("Started chromedriver from PATH");
                return Some(child);
            }
            Err(_) => {
                error!("Could not find or start chromedriver.exe");
                return None;
            }
        }
    }

    match Command::new(&driver_path)
        .arg("--port=9515")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => {
            info!("Started chromedriver from {}", driver_path.display());
            Some(child)
        }
        Err(e) => {
            error!("Failed to start chromedriver: {}", e);
            None
        }
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
    caps.add_exclude_switch("enable-automation")?;

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

    // ── Login ──
    info!("Navigating to Zoho login ...");
    driver.goto(LOGIN_URL).await?;
    sleep(PAGE_LOAD_WAIT).await;

    // Email
    info!("Entering email ...");
    let email_input = driver
        .find(By::Id("login_id"))
        .await
        .context("Could not find #login_id")?;
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

    // ── Navigate to People ──
    let url = driver.current_url().await?.to_string();
    if !url.contains("people.zoho.eu") {
        info!("Redirecting to Zoho People ...");
        driver.goto(PEOPLE_URL).await?;
        sleep(PAGE_LOAD_WAIT).await;
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

    // Start chromedriver if needed
    let _chromedriver_child = ensure_chromedriver();
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
