use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct Statistics {
    pub num_scrapings: Arc<AtomicUsize>,
    pub num_scrape_errors: Arc<AtomicUsize>,
    pub num_processings: Arc<AtomicUsize>,
    pub num_process_errors: Arc<AtomicUsize>,
}

impl Default for Statistics {
    fn default() -> Self {
        Self {
            num_scrapings: Arc::new(AtomicUsize::new(0)),
            num_scrape_errors: Arc::new(AtomicUsize::new(0)),
            num_processings: Arc::new(AtomicUsize::new(0)),
            num_process_errors: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Statistics {
    pub fn write_to_log(&self, running_time: Duration) {
        let num_procs = self.num_processings.load(Ordering::Relaxed);
        let num_proc_errors = self.num_process_errors.load(Ordering::Relaxed);
        let num_scrapes = self.num_scrapings.load(Ordering::Relaxed);
        let num_scrap_errors = self.num_scrape_errors.load(Ordering::Relaxed);
        tracing::info!(
            num_processings = num_procs,
            num_process_errors = num_proc_errors,
            num_scrapings = num_scrapes,
            num_scrape_errors = num_scrap_errors,
            running_time = ?running_time,
            "statistics"
        );
    }
}
