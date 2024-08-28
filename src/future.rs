use std::os::fd::RawFd;

#[derive(Debug)]
pub enum Interest {
    Read,
    Close,
}

#[derive(Debug)]
pub enum Waitable {
    /// Registered file descriptor.
    Fd(RawFd, Interest),
}

pub trait Future {
    type Output;
    fn poll(&mut self, ready: &[Waitable]) -> impl Iterator<Item = Waitable>;
    fn take(&mut self) -> Option<Self::Output>;
}

/// A conversion into an asynchronous computation.
pub trait IntoFuture {
    type Output;
    type IntoFuture: Future<Output = Self::Output>;
    fn into_future(self) -> Self::IntoFuture;
}

impl<T> IntoFuture for T
where
    T: Future,
{
    type Output = T::Output;
    type IntoFuture = T;
    fn into_future(self) -> Self::IntoFuture {
        self
    }
}
