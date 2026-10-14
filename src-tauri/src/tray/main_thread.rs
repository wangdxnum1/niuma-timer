//! 后台查询只跨线程传递普通结果，UI 资源在查询闭包内创建和释放。
use std::sync::mpsc;
use std::time::Duration;

pub(super) fn query<T, F, D, E>(dispatch: D, query: F, timeout: Duration) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    D: FnOnce(Box<dyn FnOnce() + Send>) -> Result<(), E>,
{
    let (tx, rx) = mpsc::channel();
    dispatch(Box::new(move || {
        let result = query();
        // query 的临时 UI 资源已经释放，再把普通结果发给 worker。
        let _ = tx.send(result);
    }))
    .ok()?;
    rx.recv_timeout(timeout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn non_send_resource_is_created_cloned_and_dropped_on_ui_thread() {
        let (tasks, queued) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
        let dropped = Arc::new(Mutex::new(None));
        let drop_record = dropped.clone();
        let ui = thread::spawn(move || {
            let id = thread::current().id();
            queued.recv().unwrap()();
            id
        });
        struct UiResource(Arc<Mutex<Option<thread::ThreadId>>>);
        impl Drop for UiResource {
            fn drop(&mut self) {
                *self.0.lock().unwrap() = Some(thread::current().id());
            }
        }
        let result = query(
            |task| tasks.send(task),
            move || {
                let resource = Rc::new(UiResource(drop_record));
                let clone = resource.clone();
                let value = Cell::new(42);
                drop(clone);
                drop(resource);
                value.get()
            },
            Duration::from_secs(5),
        );
        assert_eq!(result, Some(42));
        let ui_id = ui.join().unwrap();
        assert_ne!(ui_id, thread::current().id());
        assert_eq!(*dropped.lock().unwrap(), Some(ui_id));
    }

    #[test]
    fn failed_dispatch_does_not_run_query() {
        assert_eq!(
            query(|_| Err(()), || panic!("must not run"), Duration::ZERO),
            None::<()>
        );
    }

    #[test]
    fn timed_out_query_can_finish_without_a_waiting_worker() {
        let (tasks, queued) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
        let (finished, completion) = mpsc::channel();
        assert_eq!(
            query(
                |task| tasks.send(task),
                move || {
                    finished.send(thread::current().id()).unwrap();
                    42
                },
                Duration::ZERO,
            ),
            None
        );
        let ui = thread::spawn(move || queued.recv().unwrap()());
        let ui_id = ui.thread().id();
        ui.join().unwrap();
        assert_eq!(completion.recv().unwrap(), ui_id);
    }
}
