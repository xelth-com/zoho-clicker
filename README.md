# zoho-clicker

Simple Rust CLI tool for automating the check-in/check-out button on [Zoho People](https://people.zoho.eu/) attendance page.

Built with [thirtyfour](https://crates.io/crates/thirtyfour) (Selenium WebDriver client for Rust).

## Prerequisites

- **Rust** 1.70+
- **ChromeDriver** matching your Chrome version — download from [Chrome for Testing](https://googlechromelabs.github.io/chrome-for-testing/)

## Usage

1. **Close all Chrome windows** (needed to reuse your profile with saved Zoho session).

2. **Start ChromeDriver:**
   ```
   chromedriver.exe --port=9515
   ```

3. **Run:**
   ```
   cargo run
   ```

The tool will:
- Open Chrome with your existing profile (so Zoho cookies are preserved)
- Navigate to the Zoho People attendance page
- Find the check-in/check-out button
- Print button state **before** clicking
- Click the button
- Print button state **after** clicking and compare

## Configuration

Edit the CSS selectors in `src/main.rs` → `selectors` array to match your Zoho People page if the defaults don't work.

## License

[MIT](LICENSE)
