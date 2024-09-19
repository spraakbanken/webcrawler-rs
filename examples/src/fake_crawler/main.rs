use std::sync::Arc;

use tokio::signal;
use tracing_subscriber::{prelude::*, EnvFilter};
use webcrawler::crawler;

#[tokio::main]
async fn main() {
    println!("starting fake_crawler");
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            EnvFilter::try_from_default_env()
                .or_else(|_| EnvFilter::try_new("debug"))
                .expect("telemetry: Creating EnvFilter"),
        )
        .init();

    let spider = Arc::new(fake_crawler::FakeSpider::new());
    crawler::run(spider, signal::ctrl_c()).await;
}

pub mod fake_crawler {
    use std::{fmt, time::Duration};

    use async_trait::async_trait;
    use webcrawler::Spider;

    #[derive(Debug)]
    pub struct FakeSpider {}

    impl FakeSpider {
        pub fn new() -> Self {
            Self {}
        }
    }

    #[async_trait]
    impl Spider for FakeSpider {
        type Item = String;
        type Error = FakeError;

        fn name(&self) -> String {
            "fake-spider".to_string()
        }
        fn start_urls(&self) -> Vec<String> {
            vec![
                "https://example.com/1".to_string(),
                "https://example.com/2".to_string(),
                "https://example.com/3".to_string(),
            ]
        }
        async fn scrape(&self, url: String) -> Result<(Vec<Self::Item>, Vec<String>), Self::Error> {
            println!("scraping {}", url);
            let mut items = Vec::new();
            let mut new_urls = Vec::new();
            if !url.ends_with("0") {
                new_urls.push(format!("{}0", url));
            }
            items.push(format!("Scraped from {}", url));
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok((items, new_urls))
        }

        async fn process(&self, url: String, item: Self::Item) -> Result<String, Self::Error> {
            tokio::time::sleep(Duration::from_secs(1)).await;
            println!("processing '{}': {}", url, item);

            Ok(String::new())
        }
    }

    #[derive(Debug, Clone)]
    pub struct FakeError(String);

    impl fmt::Display for FakeError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_fmt(format_args!("FakeError: {}", self.0))
        }
    }

    impl std::error::Error for FakeError {}
}
