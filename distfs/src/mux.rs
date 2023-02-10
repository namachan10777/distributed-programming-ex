use std::{borrow::{Borrow, BorrowMut}, ops::{Deref, DerefMut}};

use tokio::sync::{ Mutex, MutexGuard};
use tracing::{info};

pub struct Pool<T> {
    inner: Vec<Mutex<T>>,
}

pub struct Leased<'a, T> {
    inner: MutexGuard<'a, T>,
    pub idx: usize,
}

impl<'a, T> Deref for Leased<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        info!(idx=self.idx, "use");
        self.inner.borrow()
    }
}

impl<'a, T> DerefMut for Leased<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        info!(idx=self.idx, "use");
        self.inner.borrow_mut()
    }
}

impl<T> Pool<T> {
    pub async fn new(inner: Vec<T>) -> Self {
        Pool {
            inner: inner.into_iter().map(Mutex::new).collect(),
        }
    }

    pub async fn get(&self) -> Leased<T> {
        let (inner, idx, _) = futures::future::select_all(self.inner.iter().map(|mutex| Box::pin(mutex.lock()))).await;
        Leased {
            inner,
            idx,
        }
    }
}
