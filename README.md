# zoho-clicker

Background daemon that automatically checks in/out on [Zoho People](https://people.zoho.eu/) attendance page.

Runs as a background process on Windows. Checks in during the morning window and checks out in the evening, skipping weekends.

## How it works

- **08:50–10:00 (weekdays):** Shows a warning popup, waits 1 minute, then performs check-in
- **After 18:00 (weekdays):** Silently performs check-out
- **Weekends:** Sleeps, does nothing
- Tracks state in `state.json` — won't retry if already done today
- Logs everything to `zoho-clicker.log`
- **Auto-manages ChromeDriver** — detects your Chrome version and downloads the matching driver automatically

## Prerequisites

- **Google Chrome** installed
- That's it. ChromeDriver is downloaded automatically.

## Setup

1. **Place `zoho-clicker.exe` in a folder** (e.g. `C:\Tools\ZohoClicker\`)

2. **Create `config.env`** in the same folder:
   ```
   ZOHO_EMAIL=your@email.com
   ZOHO_PASSWORD=yourpassword
   CHECKIN_START=08:50
   CHECKIN_END=10:00
   CHECKOUT_AFTER=18:00
   ```

3. **Run** `zoho-clicker.exe`

On first run the program will automatically download the correct `chromedriver.exe`. If Chrome updates, it will re-download the matching version.

## Auto-start with Windows

Add a shortcut to `zoho-clicker.exe` in your Startup folder:
- Press `Win+R`, type `shell:startup`, press Enter
- Create a shortcut to `zoho-clicker.exe` there

Or use Task Scheduler (recommended — no console window):
- Create task with trigger "At log on"
- Action: start `zoho-clicker.exe`, working directory = your folder

## Build from source

```
cargo build --release
```

Output: `target/release/zoho-clicker.exe` (~4.5 MB)

## License

[MIT](LICENSE)
