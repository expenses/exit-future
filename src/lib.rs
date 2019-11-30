use std::pin::Pin;
use std::task::{Poll, Context};
use futures::{Future, FutureExt, future::{select, Either}, executor::block_on};

/// Future that resolves when the exit signal has fired.
#[derive(Clone)]
pub struct Exit(broadcaster::BroadcastChannel<()>);

impl Future for Exit {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let mut future = Pin::into_inner(self).0.recv();
        Pin::new(&mut future).poll(cx).map(drop)
    }
}

impl Exit {
    /// Check if the signal hasn't been fired.
    /*pub fn is_live(&self) -> bool {
        // Hasn't received anything, hasn't been cancelled.
        self.0.lock().try_recv() == Ok(None)
    }*/

    /// Perform given work until complete.
    pub fn until<F: Future + Unpin>(self, future: F) -> impl Future<Output = Option<F::Output>> {
        select(self, future)
            .map(|either| match either {
                Either::Left(_) => None,
                Either::Right((output, _)) => Some(output)
            })
    }

    /// Block the current thread until complete.
    pub fn wait(self) {
        block_on(self)
    }
}

/// Exit signal that fires either manually or on drop.
pub struct Signal(broadcaster::BroadcastChannel<()>);

impl Signal {
    /// Fire the signal manually.
    pub fn fire(&self) -> Result<(), ()> {
        block_on(self.0.send(&())).map_err(drop)
    }
}

impl Drop for Signal {
    fn drop(&mut self) {
        self.fire().unwrap()
    }
}

/// Create a signal and exit pair. `Exit` is a future that resolves when the `Signal` object is
/// either dropped or has `fire` called on it.
pub fn signal() -> (Signal, Exit) {
    let channel = broadcaster::BroadcastChannel::new();

    let receiver = channel.clone();

    (Signal(channel), Exit(receiver))
}

#[cfg(test)]
mod tests {
    use futures::future::{join3, ready, pending, lazy};
    use std::thread::{spawn, sleep};
    use std::time::Duration;
    use std::sync::Arc;
    use super::*;

    #[test]
    fn it_works() {
        let (signal, exit_a) = signal();
        let exit_b = exit_a.clone();
        let exit_c = exit_b.clone();

        //assert!(exit_a.is_live() && exit_b.is_live());

        let barrier = Arc::new(::std::sync::Barrier::new(2));
        let thread_barrier = barrier.clone();
        let handle = spawn(move || {
            let barrier = ::futures::future::lazy(move |_| {
                thread_barrier.wait();
            });

            block_on(join3(exit_a, exit_b, barrier));
        });

        barrier.wait();
        signal.fire().unwrap();

        let _ = handle.join();
        //assert!(!exit_c.is_live());
        exit_c.wait()
    }

    #[test]
    fn drop_signal() {
        let (signal, exit) = signal();

        let thread = spawn(move || {
            sleep(Duration::from_secs(1));
            drop(signal)
        });

        thread.join().unwrap();
        exit.wait()
    }

    #[test]
    fn many_exit_signals() {
        let mut handles = Vec::new();
        let (signal, exit) = signal();

        for _ in 0 .. 100 {
            let exit = exit.clone();
            handles.push(spawn(move || {
                sleep(Duration::from_secs(1));
                exit.wait();
            }));
        }

        signal.fire().unwrap();

        for handle in handles {
            handle.join().unwrap();
        }
    }

    #[test]
    fn exit_signal_are_send_and_sync() {
        fn is_send_and_sync<T: Send + Sync>() {}

        is_send_and_sync::<Exit>();
        is_send_and_sync::<Signal>();
    }

    #[test]
    fn work_until() {
        let (signal, exit) = signal();
        let work_a = exit.clone().until(ready(5));
        assert_eq!(block_on(work_a), Some(5));

        signal.fire().unwrap();
        let work_b = exit.until(pending::<()>());
        assert_eq!(block_on(work_b), None);
    }

    #[test]
    fn works_from_other_thread() {
        let (signal, exit) = signal();

        ::std::thread::spawn(move || {
            ::std::thread::sleep(::std::time::Duration::from_millis(2500));
            signal.fire().unwrap();
        });

        block_on(exit);
    }

    #[test]
    fn clone_works() {
        let (_signal, mut exit) = signal();

        let future = lazy(move |cx| {
            let _ = Pin::new(&mut exit).poll(cx);

            let mut exit2 = exit.clone();
            let _ = Pin::new(&mut exit2).poll(cx);
        });

        block_on(future)
    }

    #[test]
    fn compat_works() {
        use futures01::Future;
        use futures::TryFutureExt;

        let (_sender, recv) = futures01::sync::oneshot::channel();
        let (signal, exit) = signal();

        let handle = spawn(move || {
            sleep(Duration::from_secs(1));
            signal.fire().unwrap();
        });

        let _ = recv
            .select(exit.clone().map(Ok).compat())
            .wait()
            .unwrap_or_else(|_| panic!());

        exit.wait();

        handle.join().unwrap();
    }
}
