//! Stills for the recent list, made on a worker the first time the list shows a file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cut::Ticks;
use eframe::egui::{self, TextureHandle};
use thumbnails::Posters;

#[derive(Default)]
pub struct RecentPosters {
    worker: Option<Posters>,
    /// `None` once a file is known to give no still.
    textures: HashMap<PathBuf, Option<TextureHandle>>,
    pending: usize,
}

impl RecentPosters {
    /// The still for `path`, asking for one taken at `at` if there's none yet.
    pub fn get(&mut self, path: &Path, at: Option<Ticks>) -> Option<TextureHandle> {
        if let Some(texture) = self.textures.get(path) {
            return texture.clone();
        }
        let worker = self.worker.get_or_insert_with(Posters::new);
        self.textures.insert(path.to_path_buf(), None);
        worker.request(path.to_path_buf(), at);
        self.pending += 1;
        None
    }

    /// Takes in finished stills, and keeps the window waking while any are on their way.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(worker) = &mut self.worker else {
            return;
        };
        for (path, image) in worker.poll() {
            self.pending = self.pending.saturating_sub(1);
            let texture =
                image.map(|image| crate::seek_bar::texture(ctx, "recent poster", &Arc::new(image)));
            self.textures.insert(path, texture);
        }
        if self.pending > 0 {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// Forgets every still, e.g. after the history is cleared.
    pub fn clear(&mut self) {
        *self = RecentPosters::default();
    }
}
