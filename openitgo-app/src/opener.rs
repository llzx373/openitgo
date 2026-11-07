use crate::timing;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum OpenStatus<T> {
    Loading,
    Ready(Result<T, String>),
}

pub struct AsyncOpener<T> {
    receiver: Receiver<Result<T, String>>,
    cached: Option<Result<T, String>>,
    cancel: Arc<AtomicBool>,
}

impl<T: Send + Clone + 'static> AsyncOpener<T> {
    pub fn open<F>(path: PathBuf, parser: F) -> Self
    where
        F: FnOnce(&Path) -> Result<T, String> + Send + 'static,
    {
        Self::open_cancellable(path, move |path, _| parser(path))
    }

    pub fn open_cancellable<F>(path: PathBuf, parser: F) -> Self
    where
        F: FnOnce(&Path, Arc<AtomicBool>) -> Result<T, String> + Send + 'static,
    {
        let (tx, rx) = channel();
        let path_clone = path.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        std::thread::spawn(move || {
            timing::log(&format!("AsyncOpener parsing {:?}", path_clone));
            let result = timing::time("AsyncOpener parse", || parser(&path_clone, worker_cancel));
            timing::log(&format!(
                "AsyncOpener result: {:?}",
                result.as_ref().map(|_| "Ok")
            ));
            let _ = tx.send(result);
        });
        Self {
            receiver: rx,
            cached: None,
            cancel,
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
            Err(std::sync::mpsc::TryRecvError::Empty) => OpenStatus::Loading,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                let result = Err("后台读取异常终止".to_string());
                self.cached = Some(result.clone());
                OpenStatus::Ready(result)
            }
        }
    }
}

impl<T> Drop for AsyncOpener<T> {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
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
    fn poll_returns_error_when_worker_disconnects() {
        let mut opener = AsyncOpener::<Comic>::open(PathBuf::from("/tmp/test"), |_path| {
            panic!("worker failure")
        });
        for _ in 0..200 {
            if let OpenStatus::Ready(Err(message)) = opener.poll() {
                assert_eq!(message, "后台读取异常终止");
                assert!(matches!(opener.poll(), OpenStatus::Ready(Err(_))));
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("worker disconnect should not stay loading");
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

    #[test]
    fn drop_sets_cancellation_flag() {
        let (started_tx, started_rx) = channel();
        let (cancelled_tx, cancelled_rx) = channel();
        let opener =
            AsyncOpener::<Comic>::open_cancellable(PathBuf::from("/tmp/test"), move |_, cancel| {
                started_tx.send(()).unwrap();
                while !cancel.load(Ordering::Acquire) {
                    thread::sleep(Duration::from_millis(1));
                }
                cancelled_tx.send(()).unwrap();
                Err("已取消".to_string())
            });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        drop(opener);
        cancelled_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }
}
