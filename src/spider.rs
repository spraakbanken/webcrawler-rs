use std::error::Error as StdError;

use async_trait::async_trait;

#[async_trait]
pub trait Spider: Send + Sync {
    type Item;
    type Error: StdError;

    fn name(&self) -> String;
    fn start_urls(&self) -> Vec<String>;
    async fn scrape(&self, url: String) -> Result<(Vec<Self::Item>, Vec<String>), Self::Error>;
    async fn process(&self, url: String, item: Self::Item) -> Result<String, Self::Error>;
}
