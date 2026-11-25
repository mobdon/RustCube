//! Host state shared by every game thread. Each GameCube OS thread runs on a
//! host thread of its own (see `os`), one at a time, so hook state can't be
//! thread-local. `Global` has the `thread_local!` cell methods
//! (`with_borrow`, `with_borrow_mut`, `get`, `set`, `replace`, `take`), so a
//! `thread_local! { static X: RefCell<T> = ... }` becomes
//! `static X: Global<T> = Global::new(|| ...)` with call sites unchanged.
//!
//! Don't hold a borrow across a guest call: the next thread to run may want it.
use std::sync::{LazyLock, Mutex, MutexGuard};

pub struct Global<T>(LazyLock<Mutex<T>>);

impl<T> Global<T> {
    pub const fn new(init: fn() -> Mutex<T>) -> Self {
        Self(LazyLock::new(init))
    }

    fn lock(&self) -> MutexGuard<'_, T> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn with_borrow<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        f(&self.lock())
    }

    pub fn with_borrow_mut<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        f(&mut self.lock())
    }

    pub fn set(&self, v: T) {
        *self.lock() = v;
    }

    pub fn replace(&self, v: T) -> T {
        std::mem::replace(&mut self.lock(), v)
    }

    pub fn take(&self) -> T
    where
        T: Default,
    {
        std::mem::take(&mut self.lock())
    }
}

impl<T: Copy> Global<T> {
    pub fn get(&self) -> T {
        *self.lock()
    }
}

/// `static X: Global<T> = global!(value);`
#[macro_export]
macro_rules! global {
    ($e:expr) => {
        $crate::global::Global::new(|| ::std::sync::Mutex::new($e))
    };
}
