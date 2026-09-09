use std::{
    error::Error as StdError,
    fmt::{self, Display},
};

use async_trait::async_trait;
use exn::Exn;

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
    type ScrapeError: StdError + Send + Sync;
    type ProcessError: StdError + Send + Sync;

    fn name(&self) -> String;
    fn start_urls(&self) -> Vec<Self::Url>;
    async fn scrape(
        &self,
        url: Self::Url,
    ) -> Result<(Vec<Self::Item>, Vec<Self::Url>), Exn<Self::ScrapeError>>;
    async fn process(
        &self,
        url: Self::Url,
        item: Self::Item,
    ) -> Result<String, Exn<Self::ProcessError>>;
}
