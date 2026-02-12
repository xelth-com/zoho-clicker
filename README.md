# zoho-clicker

Background daemon that automatically checks in/out on [Zoho People](https://people.zoho.eu/) attendance page.

Runs as a background process on Windows. Checks in during the morning window and checks out in the evening, skipping weekends.

## How it works

- **08:50–10:00 (weekdays):** Shows a warning popup, waits 1 minute, then performs check-in
- **After 18:00 (weekdays):** Silently performs check-out
- **Weekends:** Sleeps, does nothing
- Tracks state in `state.json` — won't retry if already done today
- Logs everything to `zoho-clicker.log`

## Setup

1. **Download ChromeDriver** matching your Chrome version from [Chrome for Testing](https://googlechromelabs.github.io/chrome-for-testing/)

2. **Place these files together:**
   ```
   zoho-clicker.exe
   chromedriver.exe
   config.env
   ```

3. **Edit `config.env`:**
   ```
   ZOHO_EMAIL=your@email.com
   ZOHO_PASSWORD=yourpassword
   CHECKIN_START=08:50
   CHECKIN_END=10:00
   CHECKOUT_AFTER=18:00
   ```

4. **Run** `zoho-clicker.exe`

## Auto-start with Windows

Add a shortcut to `zoho-clicker.exe` in your Startup folder:
- Press `Win+R`, type `shell:startup`, press Enter
- Create a shortcut to `zoho-clicker.exe` there

## Build from source

```
cargo build --release
```

Output: `target/release/zoho-clicker.exe` (~4.5 MB)

## License

[MIT](LICENSE)
