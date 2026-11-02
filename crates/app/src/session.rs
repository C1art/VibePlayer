use std::path::PathBuf;
use std::time::Instant;

use cut::Ticks;
use eframe::egui::{self, TextureHandle};
use playback::{LoadState, Player, PlayerOptions};
use thumbnails::{BoundedCache, Bucket, Thumbnails};

use crate::seek_bar;

const THUMBNAIL_TEXTURES: usize = 48;

/// One open file: the player plus the previews that belong to it.
pub struct Session {
    pub player: Player,
    thumbnails: Option<Thumbnails>,
    textures: BoundedCache<Bucket, TextureHandle>,
    pub title_shown: bool,
}

impl Session {
    pub fn open(path: PathBuf, options: PlayerOptions, now: Instant) -> Session {
        Session {
            player: Player::open(path, options, now),
            thumbnails: None,
            textures: BoundedCache::new(THUMBNAIL_TEXTURES),
            title_shown: false,
        }
    }

    pub fn file_name(&self) -> String {
        self.player
            .path()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Starts the preview worker once the duration is known. Returns whether previews changed.
    pub fn poll_thumbnails(&mut self) -> bool {
        if self.thumbnails.is_none()
            && let LoadState::Ready(opened) = self.player.state()
            && opened.video.is_some()
            && let Some(duration) = opened.info.duration
        {
            self.thumbnails = Some(Thumbnails::open(self.player.path().to_path_buf(), duration));
        }
        self.thumbnails.as_mut().is_some_and(Thumbnails::poll)
    }

    /// The seek-bar preview texture for `at`, if decoded yet.
    pub fn thumbnail(&mut self, ctx: &egui::Context, at: Ticks) -> Option<TextureHandle> {
        let thumbnails = self.thumbnails.as_mut()?;
        let bucket = thumbnails.bucket(at);
        if let Some(texture) = self.textures.get(&bucket) {
            return Some(texture.clone());
        }
        let image = thumbnails.get(at)?;
        let texture = seek_bar::texture(ctx, "thumbnail", &image);
        self.textures.insert(bucket, texture.clone());
        Some(texture)
    }
}
