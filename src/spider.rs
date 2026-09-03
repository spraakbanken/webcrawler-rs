use std::{
    error::Error as StdError,
    fmt::{self, Display},
};

use async_trait::async_trait;

mod url_impls;

pub trait Url:
    Clone + fmt::Debug + Send + Sync + Display + serde::de::DeserializeOwned + serde::Serialize
{
    fn url(&self) -> &String;
}

#[async_trait]
pub trait Spider: Send + Sync {
    type Url: Url;
    type Item;
    type Error: StdError;

    fn name(&self) -> String;
    fn start_urls(&self) -> Vec<Self::Url>;
    async fn scrape(
        &self,
        url: Self::Url,
    ) -> Result<(Vec<Self::Item>, Vec<Self::Url>), Self::Error>;
    async fn process(&self, url: Self::Url, item: Self::Item) -> Result<String, Self::Error>;
}
