use std::{
    collections::HashMap,
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use tokio::sync::RwLock;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub(crate) struct CrawledState {
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

impl Default for CrawledState {
    fn default() -> Self {
        Self {
            queued: Utc::now(),
            scraped_at: None,
            scrape_result: None,
            processed_at: None,
            process_result: None,
        }
    }
}

impl CrawledState {
    pub fn queued() -> CrawledState {
        Self::default()
    }
    pub fn queued_and_scraped_ok() -> CrawledState {
        let mut state = CrawledState::default();
        state.scraped_ok();
        state
    }
    pub fn queued_and_scrape_error(error: String) -> CrawledState {
        let mut state = CrawledState::default();
        state.scrape_error(error);
        state
    }
    pub fn queued_and_processed_ok<S: Into<String>>(path: S) -> CrawledState {
        let mut state = CrawledState::default();
        state.processed_ok(path.into());
        state
    }
    pub fn queued_and_process_error(error: String) -> CrawledState {
        let mut state = CrawledState::default();
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
}
pub(crate) type ProcessingState = HashMap<String, CrawledState>;
pub(crate) type SharedProcessingState = Arc<RwLock<ProcessingState>>;

pub(crate) async fn write_state(
    saved_state_path: Option<&Path>,
    visited_urls: SharedProcessingState,
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

pub(crate) fn read_state(saved_state_path: Option<&Path>) -> SharedProcessingState {
    let processing_state = if let Some(saved_state_path) = saved_state_path {
        match fs::File::open(saved_state_path) {
            Ok(file) => {
                let reader = io::BufReader::new(file);
                match serde_json::from_reader::<io::BufReader<fs::File>, serde_json::Value>(reader)
                {
                    Ok(mut json) => {
                        match ProcessingState::deserialize(json["visited_urls"].take()) {
                            Ok(visited_urls) => {
                                tracing::info!(
                                    "read saved state from '{}'",
                                    saved_state_path.display()
                                );
                                visited_urls
                            }
                            Err(err) => {
                                tracing::error!(
                                    "Failed to read saved state from '{}' Error: '{:?}'. Ignoring",
                                    saved_state_path.display(),
                                    err
                                );
                                ProcessingState::new()
                            }
                        }
                    }
                    Err(err) => {
                        tracing::error!(
                            "Failed to read file '{}' Error: '{:?}'. Ignoring",
                            saved_state_path.display(),
                            err
                        );
                        ProcessingState::new()
                    }
                }
            }
            Err(err) => {
                tracing::warn!(
                    "Failed to open file from '{}' Error: '{:?}'. Ignoring",
                    saved_state_path.display(),
                    err
                );
                ProcessingState::new()
            }
        }
    } else {
        ProcessingState::new()
    };
    Arc::new(RwLock::new(processing_state))
}
