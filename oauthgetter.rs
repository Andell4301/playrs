// use thirtyfour = "0.35.0"

use chromedriver_manager::{loglevel::LogLevel, manager::Handler};
use serde_json::json;
use std::net::TcpListener;
use thirtyfour::error::{WebDriverError as ThirtyfourError, WebDriverErrorInner};
use thirtyfour::prelude::*;
use thiserror::Error;
use tokio;
use tokio::time::{Duration, Instant, sleep};

#[derive(Debug, Error)]
pub enum OAuthGetterError {
    #[error("Chromedriver error: {0}")]
    ChromeDriverError(String),
    #[error("WebDriver error: {0}")]
    WebDriverError(#[from] ThirtyfourError),
    #[error("Timeout error: {0}")]
    TimeoutError(String),
    #[error("Browser was closed before oauth_token was found")]
    BrowserClosed,
    #[error("No available port found for Chromedriver")]
    NoAvailablePort,
}

fn get_free_port() -> Result<u16, OAuthGetterError> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| OAuthGetterError::NoAvailablePort)?;
    Ok(listener.local_addr().map_err(|_| OAuthGetterError::NoAvailablePort)?.port())
}

#[tokio::main]
pub async fn main() -> Result<(), OAuthGetterError> {
    let mut caps = DesiredCapabilities::chrome();
    caps.add_arg("--user-agent=Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/145.0.0.0 Safari/537.36")?;
    caps.add_arg("--disable-blink-features=AutomationControlled")?;
    caps.add_experimental_option("excludeSwitches", json!(["enable-automation"]))?;

    let port = get_free_port()?;

    let mut chromedriver = Handler::new()
        .launch_chromedriver(&mut caps, &port.to_string(), LogLevel::Warning)
        .await
        .map_err(|e| OAuthGetterError::ChromeDriverError(e.to_string()))?;

    let driver = WebDriver::new(&format!("http://localhost:{port}"), caps).await?;

    // Some other scripts use https://accounts.google.com/EmbeddedSetup
    // That will work here too, but for me, using OAuth from that page to
    // gen an AAS token gives a 4xx error, whereas this one works
    let result = async {
        driver.get("https://accounts.google.com/embedded/setup/v2/android").await?;

        let deadline = Duration::from_secs(300);
        let poll = Duration::from_millis(500);
        let start = Instant::now();

        loop {
            match driver.get_named_cookie("oauth_token").await {
                Ok(cookie) => return Ok(cookie.value.to_string()),
                Err(e) => match e.as_inner() {
                    WebDriverErrorInner::NoSuchCookie(_) => {}
                    WebDriverErrorInner::NoSuchWindow(_) | WebDriverErrorInner::FatalError(_) | WebDriverErrorInner::InvalidSessionId(_) => {
                        return Err(OAuthGetterError::BrowserClosed);
                    }
                    _ => return Err(OAuthGetterError::WebDriverError(e)),
                },
            }
            if start.elapsed() >= deadline {
                return Err(OAuthGetterError::TimeoutError("oauth_token cookie not found".into()));
            }
            sleep(poll).await;
        }
    }
    .await;

    match result {
        Ok(token) => println!("OAuth token: {token}"),
        Err(e) => eprintln!("Error: {e}"),
    }

    let _ = driver.quit().await;
    let _ = chromedriver.kill();

    Ok(())
}