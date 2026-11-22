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
    #[cfg(test)]
    Memory(Mutex<Option<Content>>),
}

impl Clipboard {
    pub(crate) fn native() -> Self {
        Self::Native(Arc::new(Mutex::new(None)))
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self::Memory(Mutex::new(None))
    }

    pub(crate) async fn read(&self) -> Result<Option<Content>, String> {
        match self {
            Self::Native(handle) => {
                access_native(handle.clone(), |clipboard| {
                    match clipboard.get().file_list() {
                        Ok(paths) if !paths.is_empty() => return Ok(Some(Content::Files(paths))),
                        Ok(_) | Err(Error::ContentNotAvailable) => {}
                        Err(error) => return Err(error),
                    }
                    match clipboard.get_image() {
                        Ok(image) => {
                            return Ok(Some(Content::Image {
                                width: image.width,
                                height: image.height,
                                rgba: image.bytes.into_owned(),
                            }))
                        }
                        Err(Error::ContentNotAvailable) => {}
                        Err(error) => return Err(error),
                    }
                    optional_text(clipboard.get_text()).map(|text| text.map(Content::Text))
                })
                .await
            }
            #[cfg(test)]
            Self::Memory(contents) => Ok(contents
                .lock()
                .map_err(|_| "Clipboard unavailable.")?
                .clone()),
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
                *contents.lock().map_err(|_| "Clipboard unavailable.")? = Some(content);
                Ok(())
            }
        }
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
        assert!(destination.read().await.unwrap().is_none());
        source.write(Content::Text(String::new())).await.unwrap();
        assert!(
            matches!(source.read().await.unwrap(), Some(Content::Text(text)) if text == sample)
        );
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
