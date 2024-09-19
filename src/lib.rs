//! A library for running a crawler based on the trait `Spider`.

pub mod crawler;
mod shutdown;
mod spider;

pub use crawler::{Crawler, CrawlerOptions};
pub use spider::Spider;
