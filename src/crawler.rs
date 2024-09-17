use crate::Spider;
use chrono::{DateTime, Utc};
use futures::stream::StreamExt;
use serde::de::Deserialize;
use std::error::Error as StdError;
use std::{
    collections::HashMap,
    fs,
    io::{self, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::signal;
use tokio::{
    sync::{mpsc, RwLock},
    time::{sleep, Instant},
};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

type ProcessingState = HashMap<String, CrawledState>;
type SharedProcessingState = Arc<RwLock<ProcessingState>>;

pub struct Crawler {
    delay: Duration,
    crawling_concurrency: usize,
    processing_concurrency: usize,
    saved_state_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct CrawlerOptions {
    pub delay: Duration,
    pub crawling_concurrency: usize,
    pub processing_concurrency: usize,
}

impl Crawler {
    pub fn new(
        saved_state_path: Option<PathBuf>,
        CrawlerOptions {
            delay,
            crawling_concurrency,
            processing_concurrency,
        }: CrawlerOptions,
    ) -> Self {
        Self {
            delay,
            crawling_concurrency,
            processing_concurrency,
            saved_state_path,
        }
    }

    pub async fn run<T: Send + 'static, E: StdError + Send + 'static>(
        &self,
        spider: Arc<dyn Spider<Item = T, Error = E>>,
    ) {
        tracing::info!("running spider '{}'", spider.name());
        let starting_time = Instant::now();
        let visited_urls = self.read_state();
        let crawling_concurrency = self.crawling_concurrency;
        let crawling_queue_capacity = crawling_concurrency * 400;
        let processing_concurrency = self.processing_concurrency;
        let processing_queue_capacity = processing_concurrency * 10;
        let active_spiders = Arc::new(AtomicUsize::new(0));

        // Statistics
        let num_scrapings = Arc::new(AtomicUsize::new(0));
        let num_scrape_errors = Arc::new(AtomicUsize::new(0));
        let num_processings = Arc::new(AtomicUsize::new(0));
        let num_process_errors = Arc::new(AtomicUsize::new(0));

        let (urls_to_visit_tx, urls_to_visit_rx) = mpsc::channel(crawling_queue_capacity);
        let (items_tx, items_rx) = mpsc::channel(processing_queue_capacity);
        let (new_urls_tx, mut new_urls_rx) = mpsc::channel(crawling_queue_capacity);
        let token = CancellationToken::new();
        let tracker = TaskTracker::new();

        for url in spider.start_urls() {
            tracing::info!(start_url = url);
            visited_urls
                .write()
                .await
                .insert(url.clone(), CrawledState::queued());
            let _ = urls_to_visit_tx.send(url).await;
        }

        self.launch_processors(
            &tracker,
            processing_concurrency,
            num_processings.clone(),
            num_process_errors.clone(),
            visited_urls.clone(),
            spider.clone(),
            items_rx,
            token.clone(),
        );

        self.launch_scrapers(
            &tracker,
            crawling_concurrency,
            num_scrapings.clone(),
            num_scrape_errors.clone(),
            visited_urls.clone(),
            spider.clone(),
            urls_to_visit_rx,
            new_urls_tx.clone(),
            items_tx,
            active_spiders.clone(),
            self.delay,
            token.clone(),
        );

        tracker.spawn(async move {
            let token = token.clone();
            if let Err(error) = signal::ctrl_c().await {
                tracing::error!("Failed to listen for event: {:?}", error);
            }
            token.cancel();
        });
        tracker.close();

        loop {
            if let Ok((visited_url, new_urls)) = new_urls_rx.try_recv() {
                visited_urls
                    .write()
                    .await
                    .entry(visited_url)
                    .and_modify(|state| state.scraped_ok())
                    .or_insert_with(|| {
                        let mut state = CrawledState::default();
                        state.scraped_ok();
                        state
                    });

                for url in new_urls {
                    if !visited_urls.read().await.contains_key(&url) {
                        visited_urls
                            .write()
                            .await
                            .insert(url.clone(), CrawledState::queued());
                        tracing::debug!("queueing: {}", url);
                        let _ = urls_to_visit_tx.send(url).await;
                    }
                }
            }

            if new_urls_tx.capacity() == crawling_queue_capacity // new_urls channel is empty
            && urls_to_visit_tx.capacity() == crawling_queue_capacity // urls_to_visit channel is empty
            && active_spiders.load(Ordering::SeqCst) == 0
            {
                // no more work, we leave
                break;
            }

            sleep(Duration::from_millis(5)).await;
        }

        tracing::info!("crawler: control loop exited");

        // we drop the transmitter in order to close the stream
        drop(urls_to_visit_tx);

        // and then we wait for the streams to complete
        tracker.wait().await;

        self.write_state(visited_urls).await;

        let num_procs = num_processings.load(Ordering::Relaxed);
        let num_proc_errors = num_process_errors.load(Ordering::Relaxed);
        let num_scrapes = num_scrapings.load(Ordering::Relaxed);
        let num_scrap_errors = num_scrape_errors.load(Ordering::Relaxed);
        let total_running_time = format!("{:?}", starting_time.elapsed());
        tracing::info!(
            num_processings = num_procs,
            num_process_errors = num_proc_errors,
            num_scrapings = num_scrapes,
            num_scrape_errors = num_scrap_errors,
            running_time = total_running_time,
            "statistics"
        );
    }

    async fn write_state(&self, visited_urls: SharedProcessingState) {
        let json = serde_json::json!({ "visited_urls": &*visited_urls.read().await });
        match serde_json::to_string(&json) {
            Ok(json_string) => {
                if let Some(state_path) = &self.saved_state_path {
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

    fn read_state(&self) -> SharedProcessingState {
        let processing_state = if let Some(saved_state_path) = &self.saved_state_path {
            match fs::File::open(saved_state_path) {
                Ok(file) => {
                    let reader = io::BufReader::new(file);
                    match serde_json::from_reader::<io::BufReader<fs::File>, serde_json::Value>(
                        reader,
                    ) {
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

    fn launch_processors<T: Send + 'static, E: StdError + Send + 'static>(
        &self,
        tracker: &TaskTracker,
        concurrency: usize,
        num_processings: Arc<AtomicUsize>,
        num_process_errors: Arc<AtomicUsize>,
        visited_urls: SharedProcessingState,
        spider: Arc<dyn Spider<Item = T, Error = E>>,
        items: mpsc::Receiver<(String, T)>,
        token: CancellationToken,
    ) {
        tracker.spawn(async move {
            tokio_stream::wrappers::ReceiverStream::new(items)
                .for_each_concurrent(concurrency, |(url, item)| async {
                    let token = token.clone();
                    num_processings.fetch_add(1, Ordering::SeqCst);
                    match spider.process(url.clone(), item, token).await {
                        Err(err) => {
                            num_process_errors.fetch_add(1, Ordering::SeqCst);
                            tracing::error!(url = url, "Processing error: {:?}", err);
                            visited_urls
                                .write()
                                .await
                                .entry(url)
                                .and_modify(|state| state.process_error(err.to_string()))
                                .or_insert_with(|| {
                                    let mut state = CrawledState::default();
                                    state.process_error(err.to_string());
                                    state
                                });
                        }
                        Ok(output) => {
                            visited_urls
                                .write()
                                .await
                                .entry(url)
                                .and_modify(|state| state.processed_ok(&output))
                                .or_insert_with(|| {
                                    let mut state = CrawledState::default();
                                    state.processed_ok(&output);
                                    state
                                });
                        }
                    }
                })
                .await;
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn launch_scrapers<T: Send + 'static, E: StdError + Send + 'static>(
        &self,
        tracker: &TaskTracker,
        concurrency: usize,
        num_scrapings: Arc<AtomicUsize>,
        num_scrape_errors: Arc<AtomicUsize>,
        visited_urls: SharedProcessingState,
        spider: Arc<dyn Spider<Item = T, Error = E>>,
        urls_to_visit: mpsc::Receiver<String>,
        new_urls_tx: mpsc::Sender<(String, Vec<String>)>,
        items_tx: mpsc::Sender<(String, T)>,
        active_spiders: Arc<AtomicUsize>,
        delay: Duration,
        token: CancellationToken,
    ) {
        tracker.spawn(async move {
            tokio_stream::wrappers::ReceiverStream::new(urls_to_visit)
                .for_each_concurrent(concurrency, |queued_url| async {
                    let token = token.clone();
                    active_spiders.fetch_add(1, Ordering::SeqCst);
                    let mut urls = Vec::new();
                    num_scrapings.fetch_add(1, Ordering::SeqCst);
                    let res = match spider.scrape(queued_url.clone(), token).await {
                        Err(err) => {
                            num_scrape_errors.fetch_add(1, Ordering::SeqCst);
                            tracing::error!(url = queued_url, "Scraping error: {:?}", err);
                            visited_urls
                                .write()
                                .await
                                .entry(queued_url.clone())
                                .and_modify(|state| state.scrape_error(err.to_string()))
                                .or_insert_with(|| {
                                    let mut state = CrawledState::default();
                                    state.scrape_error(err.to_string());
                                    state
                                });
                            None
                        }
                        Ok((items, new_urls)) => {
                            visited_urls
                                .write()
                                .await
                                .entry(queued_url.clone())
                                .and_modify(|state| state.scraped_ok())
                                .or_insert_with(|| {
                                    let mut state = CrawledState::default();
                                    state.scraped_ok();
                                    state
                                });
                            Some((items, new_urls))
                        }
                    };

                    if let Some((items, new_urls)) = res {
                        for item in items {
                            let _ = items_tx.send((queued_url.clone(), item)).await;
                        }
                        urls = new_urls;
                    }

                    let _ = new_urls_tx.send((queued_url, urls)).await;
                    sleep(delay).await;
                    active_spiders.fetch_sub(1, Ordering::SeqCst);
                })
                .await;

            drop(items_tx);
        });
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct CrawledState {
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
