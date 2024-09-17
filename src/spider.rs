use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait Spider: Send + Sync {
    type Item;
    type Error;

    fn name(&self) -> String;
    fn start_urls(&self) -> Vec<String>;
    async fn scrape(
        &self,
        url: String,
        token: CancellationToken,
    ) -> Result<(Vec<Self::Item>, Vec<String>), Self::Error>;
    async fn process(
        &self,
        url: String,
        item: Self::Item,
        token: CancellationToken,
    ) -> Result<String, Self::Error>;
}
