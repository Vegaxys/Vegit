//! Profile pictures of commit authors.
//!
//! Pictures are fetched once per e-mail on a single background thread: GitHub's own
//! picture for `…@users.noreply.github.com` addresses, Gravatar for the others. Until a
//! picture arrives (or when there is none) the initials are drawn instead. Can be turned
//! off in the settings, since it sends a hash of each address to those services.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use eframe::egui::{self, ColorImage, TextureHandle, TextureId};

pub static ENABLED: AtomicBool = AtomicBool::new(true);

enum Entry {
    Pending,
    Decoded(ColorImage),
    Ready(TextureHandle),
    Missing,
}

struct Cache {
    map: HashMap<String, Entry>,
    queue: Sender<String>,
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn cache(ctx: &egui::Context) -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<String>();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("avatars".into())
            .spawn(move || {
                let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).http_status_as_error(false).build().into();
                for email in rx {
                    let image = fetch(&agent, &email);
                    if let Some(c) = CACHE.get() {
                        if let Ok(mut c) = c.lock() {
                            c.map.insert(email, image.map_or(Entry::Missing, Entry::Decoded));
                        }
                    }
                    ctx.request_repaint();
                }
            })
            .expect("could not start the avatar thread");
        Mutex::new(Cache { map: HashMap::new(), queue: tx })
    })
}

/// The picture for `email`, if it is already here; asks for it otherwise.
pub fn texture(ctx: &egui::Context, email: &str) -> Option<TextureId> {
    if !ENABLED.load(std::sync::atomic::Ordering::Relaxed) || email.is_empty() {
        return None;
    }
    let key = email.trim().to_lowercase();
    let mut c = cache(ctx).lock().ok()?;
    match c.map.get(&key) {
        Some(Entry::Ready(t)) => Some(t.id()),
        Some(Entry::Pending | Entry::Missing) => None,
        Some(Entry::Decoded(_)) => {
            let Some(Entry::Decoded(img)) = c.map.remove(&key) else { return None };
            let tex = ctx.load_texture(format!("avatar-{key}"), img, egui::TextureOptions::LINEAR);
            let id = tex.id();
            c.map.insert(key, Entry::Ready(tex));
            Some(id)
        }
        None => {
            let _ = c.queue.send(key.clone());
            c.map.insert(key, Entry::Pending);
            None
        }
    }
}

/// Where a picture for this address can be found.
pub fn url_for(email: &str) -> String {
    // 12345+login@users.noreply.github.com  or  login@users.noreply.github.com
    if let Some(local) = email.strip_suffix("@users.noreply.github.com") {
        return match local.split_once('+') {
            Some((id, _)) if id.chars().all(|c| c.is_ascii_digit()) => format!("https://avatars.githubusercontent.com/u/{id}?s=64&v=4"),
            _ => format!("https://github.com/{local}.png?size=64"),
        };
    }
    format!("https://www.gravatar.com/avatar/{:x}?s=64&d=404", md5::compute(email.as_bytes()))
}

fn fetch(agent: &ureq::Agent, email: &str) -> Option<ColorImage> {
    let mut resp = agent.get(&url_for(email)).header("User-Agent", "Vegit").call().ok()?;
    if resp.status().as_u16() != 200 {
        return None;
    }
    let bytes = resp.body_mut().with_config().limit(2 * 1024 * 1024).read_to_vec().ok()?;
    let img = image::load_from_memory(&bytes).ok()?.thumbnail(64, 64).to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn github_noreply_addresses_use_github_pictures() {
        assert_eq!(super::url_for("123+octo@users.noreply.github.com"), "https://avatars.githubusercontent.com/u/123?s=64&v=4");
        assert_eq!(super::url_for("octo@users.noreply.github.com"), "https://github.com/octo.png?size=64");
        assert!(super::url_for("someone@example.com").starts_with("https://www.gravatar.com/avatar/"));
    }
}
