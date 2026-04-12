use std::sync::atomic::{AtomicBool, Ordering};

use indicatif::{ProgressBar, ProgressStyle};

use crate::types::Progress;

/// A [`Progress`] implementation backed by [`indicatif::ProgressBar`].
///
/// Requires the `indicatif` crate feature.
///
/// The progress bar starts as a spinner and switches to a determinate bar
/// once the total size is known (passed via [`Progress::on_progress`]).
pub struct IndicatifProgress {
    bar: ProgressBar,
    total_set: AtomicBool,
}

impl IndicatifProgress {
    #[must_use]
    pub fn new() -> Self {
        let bar = ProgressBar::new_spinner();
        bar.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec})")
                .unwrap_or_else(|_| ProgressStyle::default_bar())
                .progress_chars("#>-"),
        );
        Self {
            bar,
            total_set: AtomicBool::new(false),
        }
    }
}

impl Default for IndicatifProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress for IndicatifProgress {
    fn on_progress(&self, downloaded: u64, total: Option<u64>) {
        if let Some(total) = total
            && !self.total_set.swap(true, Ordering::Relaxed)
        {
            self.bar.set_length(total);
        }
        self.bar.set_position(downloaded);
    }

    fn on_finish(&self) {
        self.bar.finish();
    }
}
