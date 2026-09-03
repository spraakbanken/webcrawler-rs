use std::{
    collections::HashMap,
    io::{self, Write},
    path::Path,
    sync::Arc,
};

use fs_err as fs;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::spider::Url;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(bound = "U: serde::Serialize + serde::de::DeserializeOwned")]
pub(crate) struct CrawledState<U: Url> {
    pub(crate) url: U,
    queued: DateTime<Utc>,
    scraped_at: Option<DateTime<Utc>>,
    scrape_result: Option<StateOutcome>,
    processed_at: Option<DateTime<Utc>>,
    process_result: Option<StateOutcome>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "status", content = "outcome")]
pub enum StateOutcome {
    Ok(String),
    Error(String),
}

impl<U: Url> CrawledState<U> {
    pub fn new(url: U) -> Self {
        Self {
            url,
            queued: Utc::now(),
            scraped_at: None,
            scrape_result: None,
            processed_at: None,
            process_result: None,
        }
    }
    pub fn queued(url: U) -> CrawledState<U> {
        Self::new(url)
    }
    pub fn queued_and_scraped_ok(url: U) -> CrawledState<U> {
        let mut state = CrawledState::new(url);
        state.scraped_ok();
        state
    }
    pub fn queued_and_scrape_error(url: U, error: String) -> CrawledState<U> {
        let mut state = CrawledState::new(url);
        state.scrape_error(error);
        state
    }
    pub fn queued_and_processed_ok<S: Into<String>>(url: U, path: S) -> CrawledState<U> {
        let mut state = CrawledState::new(url);
        state.processed_ok(path.into());
        state
    }
    pub fn queued_and_process_error(url: U, error: String) -> CrawledState<U> {
        let mut state = CrawledState::new(url);
        state.process_error(error);
        state
    }
    pub fn processed_ok<S: Into<String>>(&mut self, path: S) {
        self.processed_at = Some(Utc::now());
        self.process_result = Some(StateOutcome::Ok(path.into()));
    }
    pub fn scraped_ok(&mut self) {
        self.scraped_at = Some(Utc::now());
        self.scrape_result = Some(StateOutcome::Ok("".into()))
    }
    pub fn process_error<S: Into<String>>(&mut self, error: S) {
        self.processed_at = Some(Utc::now());
        self.process_result = Some(StateOutcome::Error(error.into()));
    }
    pub fn scrape_error<S: Into<String>>(&mut self, error: S) {
        self.scraped_at = Some(Utc::now());
        self.scrape_result = Some(StateOutcome::Error(error.into()))
    }
    pub fn reset_as_queued(&mut self) {
        self.queued = Utc::now();
        self.scraped_at = None;
        self.scrape_result = None;
        self.processed_at = None;
        self.process_result = None;
    }

    pub fn is_processed(&self) -> bool {
        matches!(self.process_result, Some(StateOutcome::Ok(_)))
    }
}
pub(crate) type ProcessingState<U> = HashMap<String, CrawledState<U>>;
pub(crate) type SharedProcessingState<U> = Arc<RwLock<ProcessingState<U>>>;

pub(crate) async fn write_state<U: Url>(
    saved_state_path: Option<&Path>,
    visited_urls: SharedProcessingState<U>,
) {
    let json = serde_json::json!({ "visited_urls": &*visited_urls.read().await });
    match serde_json::to_string(&json) {
        Ok(json_string) => {
            if let Some(state_path) = saved_state_path {
                tracing::info!("crawler: writing state to '{}'", state_path.display());

                match fs::File::create(state_path) {
                    Ok(mut file) => match file.write_all(json_string.as_bytes()) {
                        Ok(_) => {
                            tracing::info!("crawler: wrote state to '{}'", state_path.display())
                        }
                        Err(err) => {
                            tracing::error!(
                                "failed write to '{}', error '{:?}'",
                                state_path.display(),
                                err
                            );
                            tracing::error!("visited_urls={:?}", json_string);
                        }
                    },
                    Err(err) => {
                        tracing::error!(
                            "failed to create '{}', error '{:?}'",
                            state_path.display(),
                            err
                        );
                        tracing::error!("visited_urls={:?}", json_string);
                    }
                }
            } else {
                tracing::info!("crawler: writing state to 'stdout'");
                let _ = io::stdout().lock().write_all(json_string.as_bytes());
            }
        }
        Err(err) => {
            tracing::error!("failed to serialize state, error '{:?}'", err);
            tracing::error!("visited_urls={:?}", json);
        }
    }
}

pub(crate) fn read_state<U: Url>(saved_state_path: Option<&Path>) -> SharedProcessingState<U> {
    let processing_state = if let Some(saved_state_path) = saved_state_path {
        match fs::File::open(saved_state_path) {
            Ok(file) => {
                let reader = io::BufReader::new(file);
                match serde_json::from_reader::<io::BufReader<fs::File>, serde_json::Value>(reader)
                {
                    Ok(mut json) => {
                        match ProcessingState::deserialize(json["visited_urls"].take()) {
                            Ok(visited_urls) => {
                                tracing::debug!(
                                    "read saved state from '{}'",
                                    saved_state_path.display()
                                );
                                visited_urls
                            }
                            Err(err) => {
                                tracing_log_error::log_error!(
                                    err,
                                    "Failed to read saved state from '{}'. Ignoring",
                                    saved_state_path.display(),
                                );
                                ProcessingState::new()
                            }
                        }
                    }
                    Err(err) => {
                        tracing_log_error::log_error!(
                            err,
                            "Failed to read file '{}'. Ignoring",
                            saved_state_path.display(),
                        );
                        ProcessingState::new()
                    }
                }
            }
            Err(err) => {
                tracing::warn!(
                    error.message = tracing_log_error::fields::error_message(&err),
                    error.details = tracing_log_error::fields::error_details(&err),
                    error.source_chain = tracing_log_error::fields::error_source_chain(&err),
                    "Failed to open file from '{}'. Ignoring",
                    saved_state_path.display(),
                );
                ProcessingState::new()
            }
        }
    } else {
        ProcessingState::new()
    };
    Arc::new(RwLock::new(processing_state))
}
