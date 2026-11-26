use arboard::{Clipboard as SystemClipboard, Error};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub(crate) enum Content {
    Text(String),
    Files(Vec<std::path::PathBuf>),
    Image {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
}

pub(crate) enum Clipboard {
    // Keeping the handle alive also keeps copied text available to Linux clipboard clients.
    Native(Arc<Mutex<Option<SystemClipboard>>>),
    /// Every format currently offered, like a native clipboard holding text and a picture.
    #[cfg(test)]
    Memory(Mutex<Vec<Content>>),
}

impl Clipboard {
    pub(crate) fn native() -> Self {
        Self::Native(Arc::new(Mutex::new(None)))
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self::Memory(Mutex::new(Vec::new()))
    }

    /// Offers several formats at once, as apps such as spreadsheets do.
    #[cfg(test)]
    pub(crate) fn offer(&self, formats: Vec<Content>) {
        if let Self::Memory(contents) = self {
            *contents.lock().unwrap() = formats;
        }
    }

    /// Reads the one format TrapLink will send. `rich` peers accept files and
    /// images, and `text_limit` is the longest text they accept.
    pub(crate) async fn read(
        &self,
        rich: bool,
        text_limit: usize,
    ) -> Result<Option<Content>, String> {
        match self {
            Self::Native(handle) => {
                access_native(handle.clone(), move |clipboard| {
                    match clipboard.get().file_list() {
                        Ok(paths) if !paths.is_empty() => return Ok(Some(Content::Files(paths))),
                        Ok(_) | Err(Error::ContentNotAvailable) => {}
                        Err(error) => return Err(error),
                    }
                    let text = optional_text(clipboard.get_text());
                    choose(text, rich, text_limit, || match clipboard.get_image() {
                        Ok(image) => Ok(Some(Content::Image {
                            width: image.width,
                            height: image.height,
                            rgba: image.bytes.into_owned(),
                        })),
                        Err(Error::ContentNotAvailable) => Ok(None),
                        Err(error) => Err(error),
                    })
                })
                .await
            }
            #[cfg(test)]
            Self::Memory(contents) => {
                let formats = contents
                    .lock()
                    .map_err(|_| "Clipboard unavailable.")?
                    .clone();
                if let Some(files) = formats.iter().find(|c| matches!(c, Content::Files(_))) {
                    return Ok(Some(files.clone()));
                }
                let text = formats.iter().find_map(|c| match c {
                    Content::Text(text) if !text.is_empty() => Some(text.clone()),
                    _ => None,
                });
                choose(Ok(text), rich, text_limit, || {
                    Ok(formats
                        .iter()
                        .find(|c| matches!(c, Content::Image { .. }))
                        .cloned())
                })
                .map_err(|_| "Clipboard unavailable.".into())
            }
        }
    }

    pub(crate) async fn write(&self, content: Content) -> Result<(), String> {
        if matches!(&content, Content::Text(text) if text.is_empty()) {
            return Ok(());
        }
        match self {
            Self::Native(handle) => {
                access_native(handle.clone(), move |clipboard| match &content {
                    Content::Text(text) => clipboard.set_text(text.as_str()),
                    Content::Files(paths) => clipboard.set().file_list(paths),
                    Content::Image {
                        width,
                        height,
                        rgba,
                    } => clipboard.set_image(arboard::ImageData {
                        width: *width,
                        height: *height,
                        bytes: rgba.as_slice().into(),
                    }),
                })
                .await
            }
            #[cfg(test)]
            Self::Memory(contents) => {
                *contents.lock().map_err(|_| "Clipboard unavailable.")? = vec![content];
                Ok(())
            }
        }
    }
}

/// Picks text over a picture of the same copy. Spreadsheets, documents and
/// slides offer both, and text is what most pastes expect. Text is also the
/// only format older peers accept. A picture is used when there is no text,
/// or when the text is too long for the peer.
fn choose(
    text: Result<Option<String>, Error>,
    rich: bool,
    text_limit: usize,
    image: impl FnOnce() -> Result<Option<Content>, Error>,
) -> Result<Option<Content>, Error> {
    let text = match text {
        Ok(Some(text)) if !rich || text.len() <= text_limit => {
            return Ok(Some(Content::Text(text)))
        }
        text => text,
    };
    match image() {
        Ok(Some(image)) => Ok(Some(image)),
        Ok(None) => text.map(|text| text.map(Content::Text)),
        Err(error) => match text {
            Ok(Some(text)) => Ok(Some(Content::Text(text))),
            _ => Err(error),
        },
    }
}

fn optional_text(result: Result<String, Error>) -> Result<Option<String>, Error> {
    match result {
        Err(Error::ContentNotAvailable) => Ok(None),
        Ok(text) if text.is_empty() => Ok(None),
        other => other.map(Some),
    }
}

async fn access_native<T: Send + 'static>(
    handle: Arc<Mutex<Option<SystemClipboard>>>,
    mut operation: impl FnMut(&mut SystemClipboard) -> Result<T, Error> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(move || {
        let mut slot = handle.lock().map_err(|_| "Clipboard unavailable.")?;
        retry_occupied(|| {
            let clipboard = match slot.as_mut() {
                Some(clipboard) => clipboard,
                None => slot.insert(SystemClipboard::new()?),
            };
            operation(clipboard)
        })
        .map_err(|error| match error {
            Error::ClipboardOccupied => "Another application is using the clipboard.",
            Error::ConversionFailure => "The clipboard text could not be converted.",
            _ => "The system clipboard is unavailable.",
        })
    })
    .await
    .map_err(|_| "Clipboard task failed.")?
    .map_err(str::to_owned)
}

fn retry_occupied<T>(mut operation: impl FnMut() -> Result<T, Error>) -> Result<T, Error> {
    for delay_ms in [15, 30, 60] {
        match operation() {
            Err(Error::ClipboardOccupied) => std::thread::sleep(Duration::from_millis(delay_ms)),
            result => return result,
        }
    }
    operation()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_clipboards_are_isolated_and_empty_writes_preserve_text() {
        let source = Clipboard::in_memory();
        let destination = Clipboard::in_memory();
        let sample = "  Olá\n世界 🌍\r\n";
        source
            .write(Content::Text(sample.to_owned()))
            .await
            .unwrap();
        assert!(destination.read(true, usize::MAX).await.unwrap().is_none());
        source.write(Content::Text(String::new())).await.unwrap();
        assert!(
            matches!(source.read(true, usize::MAX).await.unwrap(), Some(Content::Text(text)) if text == sample)
        );
    }

    fn picture() -> Content {
        Content::Image {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        }
    }

    fn pick(text: Option<&str>, rich: bool, image: Option<Content>) -> Option<Content> {
        choose(Ok(text.map(str::to_owned)), rich, 8, || Ok(image)).unwrap()
    }

    #[test]
    fn copied_text_wins_over_its_picture_and_older_peers_get_text() {
        for rich in [false, true] {
            assert!(
                matches!(pick(Some("cells"), rich, Some(picture())), Some(Content::Text(t)) if t == "cells")
            );
        }
        assert!(matches!(
            pick(None, true, Some(picture())),
            Some(Content::Image { .. })
        ));
        // An older peer cannot take a picture. Reading it still lets the
        // sender explain that both computers need an update.
        assert!(matches!(
            pick(None, false, Some(picture())),
            Some(Content::Image { .. })
        ));
        assert!(pick(None, true, None).is_none());
        // Text that is too long for a modern peer falls back to the picture.
        let long = "x".repeat(9);
        assert!(matches!(
            pick(Some(&long), true, Some(picture())),
            Some(Content::Image { .. })
        ));
        assert!(matches!(pick(Some(&long), true, None), Some(Content::Text(t)) if t == long));
        assert!(
            matches!(pick(Some(&long), false, Some(picture())), Some(Content::Text(t)) if t == long)
        );
        // A broken picture does not hide usable text, and vice versa.
        assert!(matches!(
            choose(Ok(Some(long.clone())), true, 8, || Err(
                Error::ConversionFailure
            )),
            Ok(Some(Content::Text(_)))
        ));
        assert!(matches!(
            choose(Err(Error::ConversionFailure), true, 8, || Ok(Some(
                picture()
            ))),
            Ok(Some(Content::Image { .. }))
        ));
        assert!(matches!(
            choose(Err(Error::ConversionFailure), true, 8, || Ok(None)),
            Err(Error::ConversionFailure)
        ));
    }

    #[tokio::test]
    async fn memory_clipboard_offers_text_and_picture_together() {
        let clipboard = Clipboard::in_memory();
        clipboard.offer(vec![picture(), Content::Text("A1".into())]);
        assert!(
            matches!(clipboard.read(false, 8).await.unwrap(), Some(Content::Text(t)) if t == "A1")
        );
        clipboard.write(picture()).await.unwrap();
        assert!(matches!(
            clipboard.read(true, 8).await.unwrap(),
            Some(Content::Image { .. })
        ));
    }

    #[test]
    fn empty_and_nontext_reads_have_no_payload() {
        assert_eq!(optional_text(Ok(String::new())).unwrap(), None);
        assert_eq!(
            optional_text(Err(Error::ContentNotAvailable)).unwrap(),
            None
        );
        assert_eq!(optional_text(Ok(" ".into())).unwrap().as_deref(), Some(" "));
        assert!(matches!(
            optional_text(Err(Error::ConversionFailure)),
            Err(Error::ConversionFailure)
        ));
    }

    #[test]
    fn occupied_clipboard_retries_then_recovers() {
        let mut attempts = 0;
        let result = retry_occupied(|| {
            attempts += 1;
            if attempts == 1 {
                Err(Error::ClipboardOccupied)
            } else {
                Ok(())
            }
        });
        assert!(result.is_ok());
        assert_eq!(attempts, 2);
    }

    #[test]
    fn contention_has_a_bound_and_permanent_errors_return_immediately() {
        let mut attempts = 0;
        let result = retry_occupied::<()>(|| {
            attempts += 1;
            Err(Error::ClipboardOccupied)
        });
        assert!(matches!(result, Err(Error::ClipboardOccupied)));
        assert_eq!(attempts, 4);

        let mut attempts = 0;
        let result = retry_occupied::<()>(|| {
            attempts += 1;
            Err(Error::ClipboardNotSupported)
        });
        assert!(matches!(result, Err(Error::ClipboardNotSupported)));
        assert_eq!(attempts, 1);
    }
}
