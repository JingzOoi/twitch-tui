use std::{
    cell::{OnceCell, RefCell},
    collections::{BTreeMap, HashMap},
    rc::Rc,
    sync::{
        OnceLock,
        atomic::{AtomicU32, Ordering},
    },
    thread,
};

use color_eyre::{
    Result,
    eyre::{anyhow, bail},
};
use tokio::sync::{
    mpsc,
    mpsc::{Receiver, Sender, error::TrySendError},
    oneshot::{Receiver as OSReceiver, Sender as OSSender},
};
use tracing::{error, info, warn};

use crate::{
    config::{CoreConfig, get_cache_dir},
    emotes::{downloader::get_emotes, graphics_protocol::Image},
    twitch::oauth::TwitchOauth,
    utils::emotes::get_emote_offset,
};

mod downloader;
mod graphics_protocol;

pub use downloader::get_twitch_emote;
pub use graphics_protocol::{ApplyCommand, DecodedEmote, support_graphics_protocol};

// HashMap of emote name, emote filename, and if the emote is an overlay
pub type DownloadedEmotes = BTreeMap<String, (String, bool)>;

#[derive(Copy, Clone, Debug)]
pub struct EmoteData {
    pub width: u32,
    pub id: u32,
    pub pid: u32,
}

#[derive(Debug, Copy, Clone)]
pub struct LoadedEmote {
    /// Unique ID for displaying the image, see `next_image_id`
    pub hash: u32,
    /// Number of emotes that have been displayed
    pub n: u32,
    /// Width of the emote in pixels (resized so that it's height is equal to cell height)
    pub width: u32,
    /// If the emote should be displayed over the previous emote, if no text is between them.
    pub overlay: bool,
}

#[derive(Default, Debug)]
pub struct Emotes {
    /// Map of emote name, filename, and if the emote is an overlay.
    /// We keep track of both emotes that can be used by the current user, and emotes that can be used and received.
    /// `user_emotes` is only used in the emote picker, and when the current user sends a message.
    /// `global_emotes` is used everywhere.
    pub user_emotes: RefCell<DownloadedEmotes>,
    pub global_emotes: RefCell<DownloadedEmotes>,
    /// Info about loaded emotes
    pub info: RefCell<HashMap<String, LoadedEmote>>,
    /// Terminal cell size in pixels: (width, height)
    pub cell_size: OnceCell<(f32, f32)>,
    /// Are emotes enabled?
    pub enabled: bool,
}

pub type SharedEmotes = Rc<Emotes>;

// This Drop impl is only here to cleanup in case of panics.
// The unload method should be called before exiting the alternate screen instead of relying on the drop impl.
impl Drop for Emotes {
    fn drop(&mut self) {
        self.unload();
    }
}

impl Emotes {
    pub fn new(enabled: bool) -> Self {
        Self {
            user_emotes: RefCell::default(),
            global_emotes: RefCell::default(),
            info: RefCell::default(),
            cell_size: OnceCell::default(),
            enabled,
        }
    }

    pub fn unload(&self) {
        self.info
            .borrow()
            .iter()
            .for_each(|(_, LoadedEmote { hash, .. })| {
                graphics_protocol::Clear(*hash).apply().unwrap_or_default();
            });
        self.user_emotes.borrow_mut().clear();
        self.global_emotes.borrow_mut().clear();
        self.info.borrow_mut().clear();
    }
}

impl From<LoadedEmote> for EmoteData {
    fn from(LoadedEmote { hash, n, width, .. }: LoadedEmote) -> Self {
        Self {
            id: hash,
            pid: n,
            width,
        }
    }
}

pub fn query_emotes(
    config: &CoreConfig,
    twitch_oauth: TwitchOauth,
    channel: String,
) -> OSReceiver<(DownloadedEmotes, DownloadedEmotes)> {
    let (tx, mut rx) = tokio::sync::oneshot::channel();

    if config.frontend.is_emotes_enabled() {
        let config = config.clone();
        tokio::spawn(async move { send_emotes(&config, twitch_oauth, tx, channel).await });
    } else {
        rx.close();
    }

    rx
}

pub async fn send_emotes(
    config: &CoreConfig,
    twitch_oauth: TwitchOauth,
    tx: OSSender<(DownloadedEmotes, DownloadedEmotes)>,
    channel: String,
) {
    info!("Starting emotes download.");
    match get_emotes(config, twitch_oauth, &channel).await {
        Ok(emotes) => {
            info!("Emotes downloaded.");
            if tx.send(emotes).is_err() {
                warn!("Unable to send emotes to main thread.");
            }
        }
        Err(e) => {
            warn!("Unable to download emotes: {e}");
        }
    }
}

pub static DECODE_EMOTE_SENDER: OnceLock<Sender<Image>> = OnceLock::new();

pub fn decoder(mut rx: Receiver<Image>, tx: &Sender<Result<DecodedEmote, String>>) {
    while let Some(emote) = rx.blocking_recv() {
        let name = emote.name.clone();

        let decoded = emote.decode().map_err(|_err| name);

        if tx.blocking_send(decoded).is_err() {
            error!("Unable to send decoded emote to main thread.");
            return;
        }
    }
}

/// A counter, not a hash of the name: two emotes must never share an image ID.
/// shortcut: IDs are encoded on 3 bytes and wrap after 16 million emotes in one run.
fn next_image_id() -> u32 {
    static NEXT_ID: AtomicU32 = AtomicU32::new(1);

    NEXT_ID.fetch_add(1, Ordering::Relaxed) & 0x00FF_FFFF
}

pub fn load_emote(
    word: &str,
    filename: &str,
    overlay: bool,
    info: &mut HashMap<String, LoadedEmote>,
    cell_size: (f32, f32),
) -> Result<LoadedEmote> {
    if let Some(emote) = info.get_mut(word) {
        emote.n += 1;
        Ok(*emote)
    } else {
        let hash = next_image_id();

        // Tells the terminal to load the image for later use
        let path = get_cache_dir().join(filename);
        let image = Image::builder()
            .id(hash)
            .name(word.to_string())
            .path(path)
            .overlay(overlay)
            .cell_w(cell_size.0)
            .cell_h(cell_size.1)
            .build()?;

        let width = image.width;
        let cols = image.cols;

        let decoded = image.decode()?;

        decoded.apply()?;

        // Emote with placement id 1 is reserved for emote picker
        // We tell kitty to display it now, but as it is a unicode placeholder,
        // it will only be displayed once we print the unicode placeholder.
        display_emote(hash, 1, cols)?;

        let emote = LoadedEmote {
            hash,
            n: 2,
            width,
            overlay,
        };

        info.insert(word.to_string(), emote);
        Ok(emote)
    }
}

pub fn load_picker_emote(
    word: &str,
    filename: &str,
    overlay: bool,
    info: &mut HashMap<String, LoadedEmote>,
    cell_size: (f32, f32),
) -> Result<Option<LoadedEmote>> {
    if let Some(emote) = info.get(word) {
        Ok(Some(*emote))
    } else {
        let hash = next_image_id();

        // Tells the terminal to load the image for later use
        let path = get_cache_dir().join(filename);
        let image = Image::builder()
            .id(hash)
            .name(word.to_string())
            .path(path)
            .overlay(overlay)
            .cell_w(cell_size.0)
            .cell_h(cell_size.1)
            .build()?;

        let width = image.width;

        // Decode emote in another thread, to avoid blocking main thread as decoding images is slow.
        match DECODE_EMOTE_SENDER
            .get()
            .ok_or_else(|| anyhow!("Decoding channel has not been initialized."))?
            .try_send(image)
        {
            Ok(()) => {}
            // The decoder is busy, not the emote broken: the caller tries again later.
            Err(TrySendError::Full(_)) => return Ok(None),
            Err(e) => bail!("Unable to send emote to decoder thread. {e}"),
        }

        // Emote with placement id 1 is reserved for emote picker
        let emote = LoadedEmote {
            hash,
            n: 1,
            width,
            overlay,
        };

        info.insert(word.to_string(), emote);
        Ok(Some(emote))
    }
}

pub fn display_emote(id: u32, pid: u32, cols: u16) -> Result<()> {
    graphics_protocol::Display::new(id, pid, cols).apply()
}

pub fn overlay_emote(
    parent: (u32, u32),
    emote: EmoteData,
    layer: u32,
    cols: u16,
    root_col_offset: u16,
    cell_width: u16,
) -> Result<()> {
    // Center the overlay on top of the root emote.
    let (pixel_offset, col_offset) = get_emote_offset(emote.width as u16, cell_width, cols);

    let relative_col_offset = i32::from(root_col_offset) - i32::from(col_offset);

    graphics_protocol::Chain::new(
        emote.id,
        emote.pid,
        parent,
        layer,
        relative_col_offset,
        pixel_offset,
    )
    .apply()
}

/// Size of one terminal cell in pixels, `None` when the terminal reports no usable pixel size.
/// Emotes cannot be sized without it, and a zero size would divide by zero later on.
fn cell_size(width: u16, height: u16, columns: u16, rows: u16) -> Option<(f32, f32)> {
    let cell_width = width.checked_div(columns)?;
    let cell_height = height.checked_div(rows)?;

    (cell_width > 0 && cell_height > 0).then(|| (f32::from(cell_width), f32::from(cell_height)))
}

type DecodedEmoteReceiver = Receiver<Result<DecodedEmote, String>>;

/// Initialize the emote decoder if emotes are enabled.
/// Returns a receiver for decoded emotes, or None if emotes are disabled or initialization failed.
pub fn initialize_emote_decoder(
    config: &mut CoreConfig,
) -> Option<(DecodedEmoteReceiver, (f32, f32))> {
    if !config.frontend.is_emotes_enabled() {
        return None;
    }

    // We need to probe the terminal for it's size before starting the tui,
    // as writing on stdout on a different thread can interfere.
    let cell_size = tui::crossterm::terminal::window_size()
        .map_err(|e| e.to_string())
        .and_then(|size| {
            cell_size(size.width, size.height, size.columns, size.rows)
                .ok_or_else(|| "the terminal does not report its size in pixels".to_string())
        });

    match cell_size {
        Ok(cell_size) => {
            let (decoder_tx, decoder_rx) = mpsc::channel(100);
            DECODE_EMOTE_SENDER.get_or_init(|| decoder_tx);

            let (decoded_tx, decoded_rx) = mpsc::channel(100);

            // As decoding an image is a blocking task, spawn a separate thread to handle it.
            // We cannot use tokio tasks here as it will create noticeable freezes.
            thread::spawn(move || decoder(decoder_rx, &decoded_tx));

            Some((decoded_rx, cell_size))
        }
        Err(e) => {
            config.frontend.twitch_emotes = false;
            config.frontend.betterttv_emotes = false;
            config.frontend.seventv_emotes = false;
            config.frontend.frankerfacez_emotes = false;
            warn!("Unable to query terminal for it's dimensions, disabling emotes. {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_size_needs_a_pixel_size() {
        assert_eq!(cell_size(1920, 1080, 192, 54), Some((10.0, 20.0)));
        // Terminals that do not fill in the pixel size report 0.
        assert_eq!(cell_size(0, 0, 192, 54), None);
        assert_eq!(cell_size(100, 1080, 192, 54), None);
        assert_eq!(cell_size(1920, 1080, 0, 0), None);
    }

    #[test]
    fn image_ids_are_unique_and_fit_in_three_bytes() {
        let ids = (0..1000)
            .map(|_| next_image_id())
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(ids.len(), 1000);
        assert!(ids.iter().all(|id| *id <= 0x00FF_FFFF));
    }
}
