use std::error::Error as StdError;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use futures::stream::StreamExt;
use tokio::{
    sync::{broadcast, mpsc},
    time::{sleep, Instant},
};
use tokio_util::task::TaskTracker;

use self::state::SharedProcessingState;
use self::statistics::Statistics;
use crate::shutdown::Shutdown;
use crate::Spider;

mod state;
mod statistics;

pub struct Crawler {
    delay: Duration,
    crawling_concurrency: usize,
    processing_concurrency: usize,
    notify_shutdown: broadcast::Sender<()>,
    shutdown_complete_tx: mpsc::Sender<()>,
    visited_urls: SharedProcessingState,
    statistics: Statistics,
}

#[derive(Debug, Clone)]
pub struct CrawlerOptions {
    /// Optional path to read and save state to.
    pub saved_state_path: Option<PathBuf>,
    /// The delay to use between scrapings.
    pub delay: Duration,
    /// The number of concurrent scrapers.
    pub crawling_concurrency: usize,
    /// The number of concurrent processors.
    pub processing_concurrency: usize,
}

#[derive(Debug, Clone)]
struct Handler {
    shutdown: Shutdown,
    _shutdown_complete: mpsc::Sender<()>,
}

impl Default for CrawlerOptions {
    fn default() -> Self {
        Self {
            saved_state_path: None,
            delay: Duration::from_millis(500),
            crawling_concurrency: 1,
            processing_concurrency: 20,
        }
    }
}

/// Run the crawler with default options.
///
/// Using the provided spider to scrape and process.
///
/// `tokio::signal::ctrl_c()` can be used as the `shutdown` argument. This will
/// listen for a SIGINT signal.
pub async fn run<T: Send + 'static, E: StdError + Send + 'static>(
    spider: Arc<dyn Spider<Item = T, Error = E>>,
    shutdown: impl Future,
) {
    run_with_options(spider, shutdown, CrawlerOptions::default()).await
}

/// Run the crawler with given options.
///
/// Using the provided spider to scrape and process.
///
/// `tokio::signal::ctrl_c()` can be used as the `shutdown` argument. This will
/// listen for a SIGINT signal.
pub async fn run_with_options<T: Send + 'static, E: StdError + Send + 'static>(
    spider: Arc<dyn Spider<Item = T, Error = E>>,
    shutdown: impl Future,
    CrawlerOptions {
        saved_state_path,
        delay,
        crawling_concurrency,
        processing_concurrency,
    }: CrawlerOptions,
) {
    let starting_time = Instant::now();
    let (notify_shutdown, _) = broadcast::channel(1);
    let (shutdown_complete_tx, mut shutdown_complete_rx) = mpsc::channel(1);
    let visited_urls = state::read_state(saved_state_path.as_deref());

    let crawler = Crawler {
        delay,
        crawling_concurrency,
        processing_concurrency,
        notify_shutdown,
        shutdown_complete_tx,
        visited_urls,
        statistics: Statistics::default(),
    };

    tokio::select! {
        res = crawler.run(spider) => {
            if let Err(err) = res {
                tracing::error!(cause = %err, "crawling failed");
            }
        }
        _ = shutdown => {
            tracing::info!("shutting down");
            println!("shutting down");
        }
    }

    let Crawler {
        notify_shutdown,
        shutdown_complete_tx,
        visited_urls,
        statistics,
        ..
    } = crawler;

    drop(notify_shutdown);
    drop(shutdown_complete_tx);

    let _ = shutdown_complete_rx.recv().await;

    tracing::info!("crawler: writing state");

    statistics.write_to_log(starting_time.elapsed());

    state::write_state(saved_state_path.as_deref(), visited_urls).await;
}
impl Crawler {
    pub async fn run<T: Send + 'static, E: StdError + Send + 'static>(
        &self,
        spider: Arc<dyn Spider<Item = T, Error = E>>,
    ) -> Result<(), Box<dyn StdError>> {
        tracing::info!("running spider '{}'", spider.name());
        let visited_urls = self.visited_urls.clone();
        let crawling_concurrency = self.crawling_concurrency;
        let crawling_queue_capacity = crawling_concurrency * 400;
        let processing_concurrency = self.processing_concurrency;
        let processing_queue_capacity = processing_concurrency * 10;
        let active_spiders = Arc::new(AtomicUsize::new(0));

        let (urls_to_visit_tx, urls_to_visit_rx) = mpsc::channel(crawling_queue_capacity);
        let (items_tx, items_rx) = mpsc::channel(processing_queue_capacity);
        let (new_urls_tx, new_urls_rx) = mpsc::channel(crawling_queue_capacity);
        let tracker = TaskTracker::new();

        for url in spider.start_urls() {
            tracing::info!(start_url = url);
            visited_urls
                .write()
                .await
                .insert(url.clone(), state::CrawledState::queued());
            let _ = urls_to_visit_tx.send(url).await;
        }

        let handler = Handler {
            shutdown: Shutdown::new(self.notify_shutdown.subscribe()),
            _shutdown_complete: self.shutdown_complete_tx.clone(),
        };

        self.launch_processors(
            &tracker,
            processing_concurrency,
            self.statistics.num_processings.clone(),
            self.statistics.num_process_errors.clone(),
            visited_urls.clone(),
            spider.clone(),
            items_rx,
        );

        self.launch_scrapers(
            &tracker,
            crawling_concurrency,
            self.statistics.num_scrapings.clone(),
            self.statistics.num_scrape_errors.clone(),
            visited_urls.clone(),
            spider.clone(),
            urls_to_visit_rx,
            new_urls_tx.clone(),
            items_tx,
            active_spiders.clone(),
            self.delay,
            handler.clone(),
        );

        listen_for_new_urls(
            new_urls_tx.clone(),
            new_urls_rx,
            visited_urls.clone(),
            urls_to_visit_tx.clone(),
            handler.clone(),
            crawling_queue_capacity,
            active_spiders.clone(),
        )
        .await;
        tracker.close();

        tracing::info!("crawler: control loop exited");

        // we drop the transmitter in order to close the stream
        drop(urls_to_visit_tx);

        // and then we wait for the streams to complete
        tracker.wait().await;

        Ok(())
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
    ) {
        tracker.spawn(async move {
            tokio_stream::wrappers::ReceiverStream::new(items)
                .for_each_concurrent(concurrency, |(url, item)| async {
                    tracing::debug!(url, "processing item from url");
                    match spider.process(url.clone(), item).await {
                        Err(err) => {
                            num_process_errors.fetch_add(1, Ordering::SeqCst);
                            tracing::error!(url = url, "Processing error: {:?}", err);
                            visited_urls
                                .write()
                                .await
                                .entry(url)
                                .and_modify(|state| state.process_error(err.to_string()))
                                .or_insert_with(|| {
                                    state::CrawledState::queued_and_process_error(err.to_string())
                                });
                        }
                        Ok(output) => {
                            visited_urls
                                .write()
                                .await
                                .entry(url)
                                .and_modify(|state| state.processed_ok(&output))
                                .or_insert_with(|| {
                                    state::CrawledState::queued_and_processed_ok(&output)
                                });
                        }
                    }
                    num_processings.fetch_add(1, Ordering::SeqCst);
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
        handler: Handler,
    ) {
        tracker.spawn(async move {
            tokio_stream::wrappers::ReceiverStream::new(urls_to_visit)
                .for_each_concurrent(concurrency, |queued_url| async {
                    tracing::info!(url = queued_url, "scraping");
                    let mut handler = handler.clone();
                    active_spiders.fetch_add(1, Ordering::SeqCst);
                    let mut urls = Vec::new();
                    let res = tokio::select! {
                        res = spider.scrape(queued_url.clone()) => {
                            match res {
                                Err(err) => {
                                    num_scrapings.fetch_add(1, Ordering::SeqCst);
                                    num_scrape_errors.fetch_add(1, Ordering::SeqCst);
                                    tracing::error!(url = queued_url, "Scraping error: {:?}", err);
                                    visited_urls
                                        .write()
                                        .await
                                        .entry(queued_url.clone())
                                        .and_modify(|state| state.scrape_error(err.to_string()))
                                        .or_insert_with(|| state::CrawledState::queued_and_scrape_error(err.to_string()));
                                    None
                                }
                                Ok((items, new_urls)) => {
                                    num_scrapings.fetch_add(1, Ordering::SeqCst);
                                    visited_urls
                                        .write()
                                        .await
                                        .entry(queued_url.clone())
                                        .and_modify(|state| state.scraped_ok())
                                        .or_insert_with(state::CrawledState::queued_and_scraped_ok);
                                    Some((items, new_urls))
                                }
                            }
                        }
                            _ = handler.shutdown.recv() => {
                                // If a shutdown signal is received, return
                                tracing::info!(url = queued_url, "scraper: shutdown signal received, shutting down");
                                None
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

async fn listen_for_new_urls(
    // &self,
    new_urls_tx: mpsc::Sender<(String, Vec<String>)>,
    mut new_urls_rx: mpsc::Receiver<(String, Vec<String>)>,
    visited_urls: SharedProcessingState,
    urls_to_visit_tx: mpsc::Sender<String>,
    mut handler: Handler,
    crawling_queue_capacity: usize,
    active_spiders: Arc<AtomicUsize>,
) {
    while !handler.shutdown.is_shutdown() {
        if let Ok((visited_url, new_urls)) = new_urls_rx.try_recv() {
            visited_urls
                .write()
                .await
                .entry(visited_url)
                .and_modify(|state| state.scraped_ok())
                .or_insert_with(state::CrawledState::queued_and_scraped_ok);

            for url in new_urls {
                if !visited_urls.read().await.contains_key(&url) {
                    tracing::debug!("queueing: {}", url);

                    visited_urls
                        .write()
                        .await
                        .insert(url.clone(), state::CrawledState::queued());
                    tokio::select! {
                        _ = urls_to_visit_tx.send(url) => {
                        }
                        _ = handler.shutdown.recv() => {
                            tracing::info!("listen_for_new_urls: shutting down");
                            return;
                        }
                    }
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
}
