use crate::timing;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

#[derive(Debug, Clone, PartialEq)]
pub enum OpenStatus<T> {
    Loading,
    Ready(Result<T, String>),
}

pub struct AsyncOpener<T> {
    receiver: Receiver<Result<T, String>>,
    cached: Option<Result<T, String>>,
}

impl<T: Send + Clone + 'static> AsyncOpener<T> {
    pub fn open<F>(path: PathBuf, parser: F) -> Self
    where
        F: FnOnce(&Path) -> Result<T, String> + Send + 'static,
    {
        let (tx, rx) = channel();
        let path_clone = path.clone();
        std::thread::spawn(move || {
            timing::log(&format!("AsyncOpener parsing {:?}", path_clone));
            let result = timing::time("AsyncOpener parse", || parser(&path_clone));
            timing::log(&format!(
                "AsyncOpener result: {:?}",
                result.as_ref().map(|_| "Ok")
            ));
            let _ = tx.send(result);
        });
        Self {
            receiver: rx,
            cached: None,
        }
    }

    pub fn poll(&mut self) -> OpenStatus<T> {
        if let Some(result) = &self.cached {
            return OpenStatus::Ready(result.clone());
        }
        match self.receiver.try_recv() {
            Ok(result) => {
                self.cached = Some(result.clone());
                OpenStatus::Ready(result)
            }
            Err(_) => OpenStatus::Loading,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openitgo_core::models::{Comic, Page, PageSource, Volume};
    use std::thread;
    use std::time::Duration;

    fn dummy_comic() -> Comic {
        Comic {
            id: "test".to_string(),
            title: "Test".to_string(),
            path: PathBuf::from("/tmp/test"),
            volumes: vec![Volume {
                title: "Vol 1".to_string(),
                pages: vec![Page {
                    index: 0,
                    source: PageSource::File(PathBuf::from("page.png")),
                }],
            }],
        }
    }

    #[test]
    fn poll_returns_loading_immediately_for_slow_parser() {
        let mut opener = AsyncOpener::<Comic>::open(PathBuf::from("/tmp/test"), |_path| {
            thread::sleep(Duration::from_secs(60));
            Ok(dummy_comic())
        });
        assert_eq!(opener.poll(), OpenStatus::Loading);
    }

    #[test]
    fn poll_returns_comic_when_parser_succeeds() {
        let expected = dummy_comic();
        let mut opener = AsyncOpener::<Comic>::open(PathBuf::from("/tmp/test"), move |_path| {
            Ok(expected.clone())
        });
        thread::sleep(Duration::from_millis(50));
        let status = opener.poll();
        assert!(
            matches!(status, OpenStatus::Ready(Ok(_))),
            "expected Ready(Ok(...)), got {:?}",
            status
        );
    }

    #[test]
    fn poll_returns_error_when_parser_fails() {
        let mut opener = AsyncOpener::<Comic>::open(PathBuf::from("/tmp/test"), |_path| {
            Err("parse failed".to_string())
        });
        thread::sleep(Duration::from_millis(50));
        let status = opener.poll();
        assert_eq!(status, OpenStatus::Ready(Err("parse failed".to_string())));
    }

    #[test]
    fn repeated_poll_after_completion_returns_same_result() {
        let mut opener =
            AsyncOpener::<Comic>::open(PathBuf::from("/tmp/test"), move |_path| Ok(dummy_comic()));
        thread::sleep(Duration::from_millis(50));
        let first = opener.poll();
        let second = opener.poll();
        assert_eq!(first, second);
    }
}
